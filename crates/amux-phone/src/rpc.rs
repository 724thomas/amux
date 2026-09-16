//! Client for the running amux app's local socket.
//!
//! Same wire format the `amux` CLI speaks: one NDJSON JSON-RPC 2.0 request per
//! line, one response line back. We open a fresh connection per call — at one
//! call every couple of seconds that costs nothing, and it means a restarted
//! amux heals on the next request instead of leaving a dead pooled socket.

use anyhow::{anyhow, Context};
use interprocess::local_socket::{
    tokio::{prelude::*, Stream},
    Name,
};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Give up on a call rather than letting an HTTP request hang forever. amux
/// can be alive but wedged — a lock held by a busy pane — and a phone waiting
/// on a spinner learns nothing from that.
const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

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

/// Why an RPC failed. The HTTP layer maps these onto the four distinct phone
/// screens — a dead amux and a missing pane must never look alike.
#[derive(Debug)]
pub enum RpcError {
    /// Could not reach the app at all: not running, or a stale socket.
    Down(anyhow::Error),
    /// The app answered with a JSON-RPC error.
    Rpc { code: i64, message: String },
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RpcError::Down(e) => write!(f, "amux is not reachable: {e}"),
            RpcError::Rpc { code, message } => write!(f, "{message} (code {code})"),
        }
    }
}

#[derive(Clone)]
pub struct Rpc {
    socket: String,
}

impl Rpc {
    pub fn new(socket: String) -> Self {
        Self { socket }
    }

    pub fn socket(&self) -> &str {
        &self.socket
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        match tokio::time::timeout(CALL_TIMEOUT, self.call_inner(method, params)).await {
            Ok(result) => result,
            Err(_) => Err(RpcError::Down(anyhow!(
                "amux did not answer {method} within {CALL_TIMEOUT:?}"
            ))),
        }
    }

    async fn call_inner(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let name = local_name(&self.socket)
            .map_err(|e| RpcError::Down(anyhow!("bad socket name {}: {e}", self.socket)))?;
        let stream = Stream::connect(name)
            .await
            .with_context(|| format!("connecting to {}", self.socket))
            .map_err(RpcError::Down)?;

        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });
        let mut line = serde_json::to_vec(&request).expect("request serializes");
        line.push(b'\n');

        let (read_half, mut write_half) = stream.split();
        write_half
            .write_all(&line)
            .await
            .context("writing request")
            .map_err(RpcError::Down)?;
        write_half.flush().await.ok();

        let mut response_line = String::new();
        BufReader::new(read_half)
            .read_line(&mut response_line)
            .await
            .context("reading response")
            .map_err(RpcError::Down)?;

        let response: Value = serde_json::from_str(&response_line)
            .context("invalid response from amux")
            .map_err(RpcError::Down)?;

        if let Some(error) = response.get("error").filter(|e| !e.is_null()) {
            return Err(RpcError::Rpc {
                code: error["code"].as_i64().unwrap_or(0),
                message: error["message"].as_str().unwrap_or("unknown error").to_string(),
            });
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }
}
