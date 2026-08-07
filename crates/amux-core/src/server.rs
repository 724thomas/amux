//! NDJSON JSON-RPC 2.0 server over a local socket — a Unix domain socket on
//! Unix, a named pipe on Windows, abstracted by `interprocess`.
//!
//! One request per line, one response per line. On Unix it stays trivially
//! debuggable with `socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/amux/amux.sock`. The
//! server calls the same `Engine` methods as the UI, so anything the UI can
//! do, an agent can script.

use std::sync::Arc;

use amux_protocol::{
    methods::*, rpc_codes, PaneId, RpcRequest, RpcResponse, TabId, WorkspaceId,
};
use interprocess::local_socket::{
    tokio::{prelude::*, Stream},
    ListenerOptions, Name,
};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::engine::{Engine, EngineError};

/// Map our canonical socket string to the platform's `interprocess` name:
/// a filesystem path on Unix, a namespaced pipe name on Windows.
fn local_name(s: &str) -> std::io::Result<Name<'_>> {
    #[cfg(windows)]
    {
        use interprocess::local_socket::{GenericNamespaced, ToNsName};
        s.to_ns_name::<GenericNamespaced>()
    }
    #[cfg(not(windows))]
    {
        use interprocess::local_socket::{GenericFilePath, ToFsName};
        s.to_fs_name::<GenericFilePath>()
    }
}

pub async fn run(engine: Arc<Engine>) -> anyhow::Result<()> {
    let name_str = amux_protocol::default_socket_name();

    // On Unix the name is a filesystem path: ensure the parent dir exists
    // (0700) and clear a stale socket left by a crashed instance. On Windows
    // the name is a pipe — there is no file to prepare or clean up.
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let path = std::path::Path::new(&name_str);
        if let Some(dir) = path.parent() {
            if !dir.exists() {
                std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
            }
        }
        if path.exists() {
            // Live socket → another instance owns it; stale → clean up and bind.
            if Stream::connect(local_name(&name_str)?).await.is_ok() {
                anyhow::bail!("another amux instance is already serving {name_str}");
            }
            std::fs::remove_file(path)?;
        }
    }

    let listener = ListenerOptions::new()
        .name(local_name(&name_str)?)
        .create_tokio()?;
    tracing::info!("socket server listening at {name_str}");

    loop {
        let stream = listener.accept().await?;
        let engine = Arc::clone(&engine);
        tokio::spawn(async move {
            if let Err(e) = serve_connection(stream, engine).await {
                tracing::debug!("connection ended: {e}");
            }
        });
    }
}

async fn serve_connection(stream: Stream, engine: Arc<Engine>) -> anyhow::Result<()> {
    let (read, mut write) = tokio::io::split(stream);
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
    // A method whose fields are all optional should be callable with no params
    // at all (`amux tab new`), but serde refuses to build a struct from `null`.
    // Treat a missing params object as an empty one and let the field defaults
    // do their job; methods with required fields still fail as before.
    let params = if params.is_null() { Value::Object(Default::default()) } else { params };
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

fn resolve_tab(engine: &Engine, reference: &str) -> Result<TabId, RpcDispatchError> {
    let needle = reference.strip_prefix("t-").unwrap_or(reference).replace('-', "");
    let snapshot = engine.snapshot();
    let matches: Vec<TabId> = snapshot
        .workspaces
        .iter()
        .flat_map(|w| w.tabs.iter().map(|t| t.id))
        .filter(|id| id.0.simple().to_string().starts_with(&needle.to_lowercase()))
        .collect();
    match matches.as_slice() {
        [one] => Ok(*one),
        [] => Err(RpcDispatchError::UnknownId(reference.into())),
        _ => Err(RpcDispatchError::Other(format!("ambiguous tab id: {reference}"))),
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
            let (ws, tab, pane) =
                engine.create_workspace(p.name, p.cwd.map(Into::into), 80, 24)?;
            Ok(serde_json::to_value(WorkspaceCreateResult {
                workspace: ws.to_string(),
                tab: tab.to_string(),
                pane: pane.to_string(),
            })
            .unwrap())
        }

        "workspace.focus" => {
            let p: WorkspaceRefParams = parse(params)?;
            engine.focus_workspace(resolve_workspace(engine, &p.workspace)?)?;
            Ok(Value::Null)
        }

        // -- tabs: the named screens inside a workspace ----------------------
        "tab.list" => Ok(serde_json::to_value(
            engine
                .snapshot()
                .workspaces
                .into_iter()
                .flat_map(|w| w.tabs)
                .collect::<Vec<_>>(),
        )
        .unwrap()),

        "tab.new" => {
            let p: TabNewParams = parse(params)?;
            let workspace = match &p.workspace {
                Some(reference) => resolve_workspace(engine, reference)?,
                None => engine
                    .snapshot()
                    .active_workspace
                    .ok_or_else(|| RpcDispatchError::Other("no active workspace".into()))?,
            };
            let (tab, pane) = engine.new_tab(workspace, p.name, 80, 24)?;
            Ok(serde_json::to_value(TabNewResult {
                tab: tab.to_string(),
                pane: pane.to_string(),
            })
            .unwrap())
        }

        "tab.close" => {
            let p: TabRefParams = parse(params)?;
            engine.close_tab(resolve_tab(engine, &p.tab)?)?;
            Ok(Value::Null)
        }

        "tab.focus" => {
            let p: TabRefParams = parse(params)?;
            engine.focus_tab(resolve_tab(engine, &p.tab)?)?;
            Ok(Value::Null)
        }

        "tab.rename" => {
            let p: TabRenameParams = parse(params)?;
            engine.rename_tab(resolve_tab(engine, &p.tab)?, p.name)?;
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

        "pane.set_done" => {
            let p: PaneSetDoneParams = parse(params)?;
            engine.set_pane_done(resolve_pane(engine, &p.pane)?, p.done)?;
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
                let mapped = amux_protocol::key_to_bytes(key)
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
                None => {
                    let snapshot = engine.snapshot();
                    snapshot
                        .workspaces
                        .iter()
                        .find(|w| Some(w.id) == snapshot.active_workspace)
                        .and_then(|w| w.tabs.iter().find(|t| Some(t.id) == w.active_tab))
                        .and_then(|t| t.active_pane)
                        .ok_or_else(|| RpcDispatchError::Other("no active pane".into()))?
                }
            };
            engine.notify_pane(pane, p.kind, p.title, p.body);
            Ok(Value::Null)
        }

        other => Err(RpcDispatchError::MethodNotFound(other.into())),
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};

    /// End-to-end transport check: start the server, then connect a *sync*
    /// client over `interprocess` (the same path the `amux` CLI uses) and run
    /// one `version` request. Proves the local-socket round-trip works at
    /// runtime — the transport is identical on Windows (named pipe), so a green
    /// run here is strong evidence the Windows IPC works too.
    #[tokio::test]
    async fn ipc_round_trip() {
        use interprocess::local_socket::{prelude::*, Stream};

        // Platform-appropriate unique name: a temp file path on Unix, a bare
        // pipe name on Windows (named pipes have no filesystem path). The
        // client uses the same `local_name` mapping as the server, so they
        // agree by construction — this exercises the real transport on both.
        #[cfg(windows)]
        let sock = format!("amux-ipctest-{}.sock", std::process::id());
        #[cfg(not(windows))]
        let sock = std::env::temp_dir()
            .join(format!("amux-ipctest-{}.sock", std::process::id()))
            .to_string_lossy()
            .into_owned();
        std::env::set_var(amux_protocol::env_keys::SOCKET, &sock);

        let engine = crate::engine::Engine::new();
        tokio::spawn(super::run(engine));

        let name = sock.clone();
        let response = tokio::task::spawn_blocking(move || {
            // Retry until the server has bound the socket.
            for _ in 0..50 {
                if let Ok(mut stream) = Stream::connect(super::local_name(&name).unwrap()) {
                    stream
                        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"version\"}\n")
                        .unwrap();
                    let mut line = String::new();
                    BufReader::new(&stream).read_line(&mut line).unwrap();
                    return line;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            panic!("could not connect to the test server");
        })
        .await
        .unwrap();

        std::env::remove_var(amux_protocol::env_keys::SOCKET);
        let _ = std::fs::remove_file(&sock);

        assert!(
            response.contains("\"result\"") && response.contains("version"),
            "unexpected response: {response}"
        );
    }
}
