//! The HTTP door the phone knocks on.
//!
//! amux already has a door — the local socket — and it is deliberately
//! unauthenticated so the `amux` CLI and in-pane agents can drive the app.
//! That one stays open. This is a second door, and only this one is locked.
//! Past the lock both doors call exactly the same engine methods.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use amux_protocol::{rpc_codes, PaneInfo, WorkspaceInfo};
use axum::{
    extract::{ConnectInfo, Path, Query, Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::Auth;
use crate::rpc::{Rpc, RpcError};

#[derive(Clone)]
pub struct AppState {
    pub rpc: Rpc,
    pub auth: Arc<Auth>,
    /// The local issuer's certificate, when running over HTTPS. Served so the
    /// phone can install it once — without it the phone has no way to fetch the
    /// very thing it needs in order to trust this server.
    pub ca_pem: Option<Arc<String>>,
    /// Unix seconds of the last request. The idle shutdown in `main` watches
    /// this so a session forgotten after lunch does not stay open all night.
    pub last_seen: Arc<AtomicU64>,
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn router(state: AppState) -> Router {
    // Pairing is the one unauthenticated route — it is how a device becomes
    // authenticated in the first place — so it sits outside the guard.
    let open = Router::new()
        .route("/api/pair", post(pair))
        .route("/favicon.ico", get(icon))
        .route("/ca.crt", get(ca_cert))
        .route("/", get(index));

    let guarded = Router::new()
        .route("/api/me", get(me))
        .route("/api/tabs", get(tabs))
        .route("/api/screen/{pane}", get(screen))
        .route("/api/send", post(send))
        .route("/api/keys", post(keys))
        .layer(middleware::from_fn_with_state(state.clone(), guard));

    open.merge(guarded)
        .layer(middleware::from_fn(no_store))
        .layer(middleware::from_fn(host_guard))
        .layer(middleware::from_fn_with_state(state.clone(), access_log))
        .with_state(state)
}

/// Nothing here should be written to a phone's disk cache: `/api/screen`
/// carries source code and `/api/pair` carries a token.
async fn no_store(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    response
        .headers_mut()
        .insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response
}

/// Record who knocked, and what they asked for.
///
/// When the phone cannot reach this server the question is always the same:
/// did the request arrive and get refused, or did it never arrive at all? A
/// firewall drop and a network that does not route look identical from the
/// phone — one line here tells them apart.
async fn access_log(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    state.last_seen.store(now_secs(), Ordering::Relaxed);
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let response = next.run(request).await;
    tracing::debug!("{} {} {} → {}", peer.ip(), method, path, response.status().as_u16());
    response
}

/// Only answer requests addressed to an IP literal or `localhost`.
///
/// DNS rebinding works by pointing a *name* the attacker controls at this
/// machine, so a browser treats their page and this server as one origin. An
/// address that is already an IP cannot be re-pointed, which makes this a
/// one-line defence rather than a list of hostnames to maintain.
async fn host_guard(request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let name = host.rsplit_once(':').map_or(host, |(h, _)| h);
    let name = name.trim_start_matches('[').trim_end_matches(']');

    let ok = name.is_empty()
        || name == "localhost"
        || name.parse::<std::net::IpAddr>().is_ok();
    if !ok {
        tracing::warn!("거절: Host 헤더가 이름입니다 ({host}) — DNS 리바인딩 가능성");
        return fail(StatusCode::MISDIRECTED_REQUEST, "bad_host").into_response();
    }
    next.run(request).await
}

/// Reject anything without a registered device token before it can reach the
/// engine. A rejected request must never have touched amux.
async fn guard(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    let token = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .unwrap_or("");

    if token.is_empty() || state.auth.verify(token, Some(&peer.ip().to_string())).is_none() {
        return fail(StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    next.run(request).await
}

fn fail(code: StatusCode, error: &str) -> (StatusCode, Json<Value>) {
    (code, Json(json!({ "error": error })))
}

/// Translate a socket-level failure into the phone's four distinct screens.
/// "amux is dead" and "that tab is gone" must not arrive as the same spinner.
fn rpc_fail(e: RpcError) -> (StatusCode, Json<Value>) {
    match e {
        RpcError::Down(err) => {
            tracing::warn!("amux unreachable: {err:#}");
            fail(StatusCode::SERVICE_UNAVAILABLE, "amux_down")
        }
        RpcError::Rpc { code, message } if code == rpc_codes::NOT_FOUND => {
            tracing::debug!("pane gone: {message}");
            fail(StatusCode::NOT_FOUND, "pane_gone")
        }
        RpcError::Rpc { code, message } => {
            tracing::warn!("amux rpc error {code}: {message}");
            fail(StatusCode::BAD_GATEWAY, "amux_error")
        }
    }
}

async fn index() -> Response {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        include_str!("web/index.html"),
    )
        .into_response()
}

/// The browser asks for this on its own, and a home-screen shortcut shows it.
/// Serving it here also stops the token gate from answering 401 to a request
/// nobody authenticated — that reads as a failure in the log when it is not.
async fn icon() -> Response {
    (
        [(header::CONTENT_TYPE, "image/svg+xml")],
        include_str!("web/icon.svg"),
    )
        .into_response()
}

/// Hand out the issuer certificate for the one-time install on the phone.
///
/// Unauthenticated on purpose: it is a public key, it grants nothing on its
/// own, and requiring a token here would be circular — the phone cannot reach
/// an HTTPS server it does not yet trust.
async fn ca_cert(State(state): State<AppState>) -> Response {
    match state.ca_pem {
        Some(pem) => (
            [
                (header::CONTENT_TYPE, "application/x-x509-ca-cert"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"amux-phone-ca.crt\"",
                ),
            ],
            pem.to_string(),
        )
            .into_response(),
        None => fail(StatusCode::NOT_FOUND, "no_ca").into_response(),
    }
}

async fn me(State(state): State<AppState>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let version = state.rpc.call("version", Value::Null).await.map_err(rpc_fail)?;
    let panes = state.rpc.call("pane.list", Value::Null).await.map_err(rpc_fail)?;
    Ok(Json(json!({
        "ok": true,
        "amux": version.get("version").cloned().unwrap_or(Value::Null),
        "panes": panes.as_array().map(|a| a.len()).unwrap_or(0),
    })))
}

/// Workspaces → tabs → panes, joined on tab id.
///
/// `tab.list` is not used: it flattens every workspace's tabs into one array,
/// which loses the grouping the phone's list screen is built around.
async fn tabs(State(state): State<AppState>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let workspaces: Vec<WorkspaceInfo> = serde_json::from_value(
        state.rpc.call("workspace.list", Value::Null).await.map_err(rpc_fail)?,
    )
    .map_err(|e| {
        tracing::error!("workspace.list did not match the protocol type: {e}");
        fail(StatusCode::BAD_GATEWAY, "amux_error")
    })?;

    let panes: Vec<PaneInfo> = serde_json::from_value(
        state.rpc.call("pane.list", Value::Null).await.map_err(rpc_fail)?,
    )
    .map_err(|e| {
        tracing::error!("pane.list did not match the protocol type: {e}");
        fail(StatusCode::BAD_GATEWAY, "amux_error")
    })?;

    let out: Vec<Value> = workspaces
        .iter()
        .map(|w| {
            json!({
                "id": w.id.to_string(),
                "name": w.name,
                "active_tab": w.active_tab.map(|t| t.to_string()),
                "tabs": w.tabs.iter().map(|t| json!({
                    "id": t.id.to_string(),
                    "name": t.name,
                    "panes": panes.iter().filter(|p| p.tab == t.id).map(|p| json!({
                        "id": p.id.to_string(),
                        "status": p.status,
                        "exited": p.exited,
                        "cwd": p.meta.cwd,
                        "git_branch": p.meta.git_branch,
                        "title": p.meta.title,
                        "notification": p.notification,
                    })).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();

    Ok(Json(json!({ "workspaces": out })))
}

#[derive(Deserialize)]
pub struct ScreenQuery {
    /// Hash the phone already has; when it still matches we skip the body.
    since: Option<String>,
}

/// One screenful of a pane, plus the measurements the phone needs to size its
/// text. Pane widths are not uniform — measured live they range from 161×45 to
/// 202×59 — so the phone must be told, not assume.
async fn screen(
    State(state): State<AppState>,
    Path(pane): Path<String>,
    Query(q): Query<ScreenQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let result = state
        .rpc
        .call("pane.read_screen", json!({ "pane": pane }))
        .await
        .map_err(rpc_fail)?;

    let text = result.get("text").and_then(Value::as_str).unwrap_or("");
    let hash = short_hash(text);

    if q.since.as_deref() == Some(hash.as_str()) {
        return Ok(Json(json!({ "hash": hash, "unchanged": true })));
    }

    let cols = text.lines().map(|l| l.chars().count()).max().unwrap_or(0);
    let rows = text.lines().count();
    Ok(Json(json!({ "hash": hash, "cols": cols, "rows": rows, "text": text })))
}

#[derive(Deserialize)]
pub struct SendBody {
    pane: String,
    text: String,
    #[serde(default)]
    enter: bool,
}

/// Whole strings only — never a keystroke at a time. That is what keeps the
/// phone clear of the Hangul composition bugs the desktop had to fix: a
/// half-composed syllable never reaches the terminal.
async fn send(
    State(state): State<AppState>,
    Json(body): Json<SendBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    state
        .rpc
        .call("pane.send_text", json!({ "pane": body.pane, "text": body.text }))
        .await
        .map_err(rpc_fail)?;

    if body.enter {
        state
            .rpc
            .call("pane.send_keys", json!({ "pane": body.pane, "keys": ["Enter"] }))
            .await
            .map_err(rpc_fail)?;
    }
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct KeysBody {
    pane: String,
    keys: Vec<String>,
}

/// Named keys straight through to amux. The vocabulary is whatever
/// `amux_protocol::key_to_bytes` accepts — `Enter`, `Escape`, `Up`, `C-c`, …
async fn keys(
    State(state): State<AppState>,
    Json(body): Json<KeysBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    state
        .rpc
        .call("pane.send_keys", json!({ "pane": body.pane, "keys": body.keys }))
        .await
        .map_err(rpc_fail)?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct PairBody {
    code: String,
    #[serde(default)]
    label: String,
}

async fn pair(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(body): Json<PairBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    // What actually makes guessing hopeless is the attempt budget inside
    // `Auth::pair`, not this pause — five tries against six digits is
    // 5-in-a-million per code. The pause only keeps a script from turning the
    // budget over as fast as the network allows.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let from = peer.ip().to_string();
    match state.auth.pair(&body.code, &body.label, Some(&from)) {
        Ok(Some(device)) => {
            // Announce it: enrolment the user did not perform should be
            // something they can notice afterwards.
            tracing::info!("기기가 등록되었습니다 — {} (from {from})", device.label);
            Ok(Json(json!({ "token": device.token, "label": device.label })))
        }
        Ok(None) => {
            tracing::warn!("페어링 실패 (from {from})");
            Err(fail(StatusCode::UNAUTHORIZED, "bad_code"))
        }
        Err(e) => {
            tracing::error!("could not save the device: {e:#}");
            Err(fail(StatusCode::INTERNAL_SERVER_ERROR, "save_failed"))
        }
    }
}

fn short_hash(text: &str) -> String {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
