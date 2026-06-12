//! NDJSON JSON-RPC 2.0 server over a Unix domain socket.
//!
//! One request per line, one response per line — trivially debuggable with
//! `socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/cmux/cmux.sock`. The server calls
//! the same `Engine` methods as the UI, so anything the UI can do, an agent
//! can script.

use std::os::unix::fs::DirBuilderExt;
use std::sync::Arc;

use cmux_protocol::{
    methods::*, rpc_codes, PaneId, RpcRequest, RpcResponse, WorkspaceId,
};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::engine::{Engine, EngineError};

pub async fn run(engine: Arc<Engine>) -> anyhow::Result<()> {
    let path = cmux_protocol::default_socket_path();
    let dir = path.parent().expect("socket path has a parent");
    if !dir.exists() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }

    if path.exists() {
        // Live socket → another instance owns it; stale → clean up and bind.
        if UnixStream::connect(&path).await.is_ok() {
            anyhow::bail!("another cmux instance is already serving {}", path.display());
        }
        std::fs::remove_file(&path)?;
    }

    let listener = UnixListener::bind(&path)?;
    tracing::info!("socket server listening at {}", path.display());

    loop {
        let (stream, _) = listener.accept().await?;
        let engine = Arc::clone(&engine);
        tokio::spawn(async move {
            if let Err(e) = serve_connection(stream, engine).await {
                tracing::debug!("connection ended: {e}");
            }
        });
    }
}

async fn serve_connection(stream: UnixStream, engine: Arc<Engine>) -> anyhow::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<RpcRequest>(&line) {
            Ok(request) => {
                let id = request.id.clone();
                match dispatch(&engine, &request.method, request.params) {
                    Ok(result) => RpcResponse::ok(id, result),
                    Err(e) => RpcResponse::err(id, error_code(&e), e.to_string()),
                }
            }
            Err(e) => RpcResponse::err(Value::Null, rpc_codes::PARSE_ERROR, e.to_string()),
        };
        let mut payload = serde_json::to_vec(&response)?;
        payload.push(b'\n');
        write.write_all(&payload).await?;
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
enum RpcDispatchError {
    #[error("method not found: {0}")]
    MethodNotFound(String),
    #[error("invalid params: {0}")]
    InvalidParams(serde_json::Error),
    #[error("unknown id: {0}")]
    UnknownId(String),
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error("{0}")]
    Other(String),
}

fn error_code(e: &RpcDispatchError) -> i64 {
    match e {
        RpcDispatchError::MethodNotFound(_) => rpc_codes::METHOD_NOT_FOUND,
        RpcDispatchError::InvalidParams(_) => rpc_codes::INVALID_PARAMS,
        RpcDispatchError::UnknownId(_) | RpcDispatchError::Engine(EngineError::PaneNotFound(_))
        | RpcDispatchError::Engine(EngineError::WorkspaceNotFound(_)) => rpc_codes::NOT_FOUND,
        RpcDispatchError::Engine(_) | RpcDispatchError::Other(_) => rpc_codes::INTERNAL_ERROR,
    }
}

fn parse<T: serde::de::DeserializeOwned>(params: Value) -> Result<T, RpcDispatchError> {
    serde_json::from_value(params).map_err(RpcDispatchError::InvalidParams)
}

/// Accept a full UUID or a short prefix (with or without `p-`/`w-`).
fn resolve_pane(engine: &Engine, reference: &str) -> Result<PaneId, RpcDispatchError> {
    let needle = reference.strip_prefix("p-").unwrap_or(reference).replace('-', "");
    let snapshot = engine.snapshot();
    let matches: Vec<PaneId> = snapshot
        .panes
        .iter()
        .map(|p| p.id)
        .filter(|id| id.0.simple().to_string().starts_with(&needle.to_lowercase()))
        .collect();
    match matches.as_slice() {
        [one] => Ok(*one),
        [] => Err(RpcDispatchError::UnknownId(reference.into())),
        _ => Err(RpcDispatchError::Other(format!("ambiguous pane id: {reference}"))),
    }
}

fn resolve_workspace(engine: &Engine, reference: &str) -> Result<WorkspaceId, RpcDispatchError> {
    let needle = reference.strip_prefix("w-").unwrap_or(reference).replace('-', "");
    let snapshot = engine.snapshot();
    let matches: Vec<WorkspaceId> = snapshot
        .workspaces
        .iter()
        .map(|w| w.id)
        .filter(|id| id.0.simple().to_string().starts_with(&needle.to_lowercase()))
        .collect();
    match matches.as_slice() {
        [one] => Ok(*one),
        [] => Err(RpcDispatchError::UnknownId(reference.into())),
        _ => Err(RpcDispatchError::Other(format!("ambiguous workspace id: {reference}"))),
    }
}

fn dispatch(
    engine: &Arc<Engine>,
    method: &str,
    params: Value,
) -> Result<Value, RpcDispatchError> {
    match method {
        "version" => Ok(json!({ "version": env!("CARGO_PKG_VERSION") })),

        "workspace.list" => Ok(serde_json::to_value(engine.snapshot().workspaces).unwrap()),

        "workspace.create" => {
            let p: WorkspaceCreateParams = parse(params)?;
            let (ws, pane) =
                engine.create_workspace(p.name, p.cwd.map(Into::into), 80, 24)?;
            Ok(serde_json::to_value(WorkspaceCreateResult {
                workspace: ws.to_string(),
                pane: pane.to_string(),
            })
            .unwrap())
        }

        "workspace.focus" => {
            let p: WorkspaceRefParams = parse(params)?;
            engine.focus_workspace(resolve_workspace(engine, &p.workspace)?)?;
            Ok(Value::Null)
        }

        "pane.list" => Ok(serde_json::to_value(engine.snapshot().panes).unwrap()),

        "pane.split" => {
            let p: PaneSplitParams = parse(params)?;
            let pane = resolve_pane(engine, &p.pane)?;
            let new_pane = engine.split_pane(pane, p.axis, 80, 24)?;
            Ok(serde_json::to_value(PaneSplitResult { pane: new_pane.to_string() }).unwrap())
        }

        "pane.close" => {
            let p: PaneRefParams = parse(params)?;
            engine.close_pane(resolve_pane(engine, &p.pane)?)?;
            Ok(Value::Null)
        }

        "pane.focus" => {
            let p: PaneRefParams = parse(params)?;
            engine.focus_pane(resolve_pane(engine, &p.pane)?)?;
            Ok(Value::Null)
        }

        "pane.send_text" => {
            let p: SendTextParams = parse(params)?;
            engine.write_pane(resolve_pane(engine, &p.pane)?, p.text.as_bytes())?;
            Ok(Value::Null)
        }

        "pane.send_keys" => {
            let p: SendKeysParams = parse(params)?;
            let pane = resolve_pane(engine, &p.pane)?;
            let mut bytes = Vec::new();
            for key in &p.keys {
                let mapped = cmux_protocol::key_to_bytes(key)
                    .ok_or_else(|| RpcDispatchError::Other(format!("unknown key: {key}")))?;
                bytes.extend(mapped);
            }
            engine.write_pane(pane, &bytes)?;
            Ok(Value::Null)
        }

        "pane.rename" => {
            let p: PaneRenameParams = parse(params)?;
            engine.rename_pane(resolve_pane(engine, &p.pane)?, p.name)?;
            Ok(Value::Null)
        }

        "workspace.rename" => {
            let p: WorkspaceRenameParams = parse(params)?;
            engine.rename_workspace(resolve_workspace(engine, &p.workspace)?, p.name)?;
            Ok(Value::Null)
        }

        "pane.read_screen" => {
            let p: PaneRefParams = parse(params)?;
            let text = engine.read_screen(resolve_pane(engine, &p.pane)?)?;
            Ok(serde_json::to_value(ReadScreenResult { text }).unwrap())
        }

        "notify.set" => {
            let p: NotifySetParams = parse(params)?;
            // Default to the calling pane; fall back to the visible pane.
            let pane = match &p.pane {
                Some(reference) => resolve_pane(engine, reference)?,
                None => engine
                    .snapshot()
                    .workspaces
                    .iter()
                    .find(|w| Some(w.id) == engine.snapshot().active_workspace)
                    .and_then(|w| w.active_pane)
                    .ok_or_else(|| RpcDispatchError::Other("no active pane".into()))?,
            };
            engine.notify_pane(pane, p.kind, p.title, p.body);
            Ok(Value::Null)
        }

        other => Err(RpcDispatchError::MethodNotFound(other.into())),
    }
}
