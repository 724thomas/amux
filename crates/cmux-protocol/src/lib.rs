//! Shared types between the cmux engine, the Tauri app, and the `cmux` CLI.
//!
//! Everything that crosses a process or IPC boundary lives here:
//! entity IDs, the state `Snapshot` mirrored to the frontend, and the
//! JSON-RPC 2.0 envelope spoken over the Unix socket.

use serde::{Deserialize, Serialize};
use std::fmt;

// ---------------------------------------------------------------------------
// IDs
// ---------------------------------------------------------------------------

macro_rules! id_type {
    ($name:ident, $prefix:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub uuid::Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(uuid::Uuid::new_v4())
            }

            /// Short human-friendly form used by the CLI (`p-3fa2c1`).
            pub fn short(&self) -> String {
                format!("{}-{}", $prefix, &self.0.simple().to_string()[..6])
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

id_type!(PaneId, "p");
id_type!(WorkspaceId, "w");

// ---------------------------------------------------------------------------
// Snapshot (engine state mirrored to the frontend / returned by the socket)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitAxis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayoutNode {
    Leaf {
        pane: PaneId,
    },
    Split {
        axis: SplitAxis,
        ratio: f32,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyKind {
    Attention,
    Done,
    Progress,
    Bell,
    Idle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaneNotification {
    pub kind: NotifyKind,
    pub title: Option<String>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PaneMeta {
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub listening_ports: Vec<u16>,
    pub title: Option<String>,
    /// kitty keyboard protocol (disambiguate) active — the frontend switches
    /// key encodings (Esc, modified Enter) and shift+arrow passthrough on it.
    pub kitty_keyboard: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneInfo {
    pub id: PaneId,
    pub workspace: WorkspaceId,
    pub name: String,
    pub meta: PaneMeta,
    pub notification: Option<PaneNotification>,
    pub exited: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub id: WorkspaceId,
    pub name: String,
    pub layout: LayoutNode,
    pub active_pane: Option<PaneId>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub workspaces: Vec<WorkspaceInfo>,
    pub panes: Vec<PaneInfo>,
    pub active_workspace: Option<WorkspaceId>,
}

// ---------------------------------------------------------------------------
// JSON-RPC 2.0 envelope (NDJSON over the Unix socket)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcResponse {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl RpcResponse {
    pub fn ok(id: serde_json::Value, result: serde_json::Value) -> Self {
        Self { jsonrpc: "2.0".into(), id, result: Some(result), error: None }
    }

    pub fn err(id: serde_json::Value, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(RpcError { code, message: message.into() }),
        }
    }
}

/// JSON-RPC error codes (standard range plus an application range).
pub mod rpc_codes {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;
    /// Referenced pane/workspace does not exist.
    pub const NOT_FOUND: i64 = -32000;
    /// Pane exists but its process has exited.
    pub const PANE_EXITED: i64 = -32001;
}

/// Default socket path: `$XDG_RUNTIME_DIR/cmux/cmux.sock`,
/// falling back to `/tmp/cmux-$UID/cmux.sock`.
/// `$CMUX_SOCKET` (set inside panes) overrides both.
pub fn default_socket_path() -> std::path::PathBuf {
    if let Some(explicit) = std::env::var_os(env_keys::SOCKET) {
        return std::path::PathBuf::from(explicit);
    }
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(|d| std::path::PathBuf::from(d).join("cmux"))
        .unwrap_or_else(|| {
            let uid = unsafe { libc_geteuid() };
            std::path::PathBuf::from(format!("/tmp/cmux-{uid}"))
        });
    dir.join("cmux.sock")
}

// Avoid a libc dependency for one call: geteuid via extern.
extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
}

/// Environment variables injected into every pane's child process.
pub mod env_keys {
    pub const PANE_ID: &str = "CMUX_PANE_ID";
    pub const WORKSPACE_ID: &str = "CMUX_WORKSPACE_ID";
    pub const SOCKET: &str = "CMUX_SOCKET";
}

// ---------------------------------------------------------------------------
// RPC method params/results (shared between the server and the CLI)
// ---------------------------------------------------------------------------

pub mod methods {
    use serde::{Deserialize, Serialize};

    use crate::SplitAxis;

    #[derive(Debug, Serialize, Deserialize)]
    pub struct WorkspaceCreateParams {
        pub name: Option<String>,
        pub cwd: Option<String>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct WorkspaceCreateResult {
        pub workspace: String,
        pub pane: String,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct WorkspaceRefParams {
        pub workspace: String,
    }

    /// `pane` accepts a full UUID or a short prefix (`p-3fa2c1` / `3fa2c1`).
    #[derive(Debug, Serialize, Deserialize)]
    pub struct PaneRefParams {
        pub pane: String,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct PaneSplitParams {
        pub pane: String,
        pub axis: SplitAxis,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct PaneSplitResult {
        pub pane: String,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct SendTextParams {
        pub pane: String,
        pub text: String,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct SendKeysParams {
        pub pane: String,
        pub keys: Vec<String>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct ReadScreenResult {
        pub text: String,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct PaneRenameParams {
        pub pane: String,
        pub name: String,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct WorkspaceRenameParams {
        pub workspace: String,
        pub name: String,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct NotifySetParams {
        pub pane: Option<String>,
        pub kind: crate::NotifyKind,
        pub title: Option<String>,
        pub body: Option<String>,
    }
}

/// tmux-style key name → bytes for the PTY (`Enter`, `C-c`, `Up`, ...).
/// Single non-special characters pass through literally.
pub fn key_to_bytes(key: &str) -> Option<Vec<u8>> {
    let bytes: Vec<u8> = match key {
        "Enter" | "CR" => b"\r".to_vec(),
        "Tab" => b"\t".to_vec(),
        "Escape" | "Esc" => b"\x1b".to_vec(),
        "Space" => b" ".to_vec(),
        "BSpace" | "Backspace" => b"\x7f".to_vec(),
        "Up" => b"\x1b[A".to_vec(),
        "Down" => b"\x1b[B".to_vec(),
        "Right" => b"\x1b[C".to_vec(),
        "Left" => b"\x1b[D".to_vec(),
        "Home" => b"\x1b[H".to_vec(),
        "End" => b"\x1b[F".to_vec(),
        "PageUp" | "PgUp" => b"\x1b[5~".to_vec(),
        "PageDown" | "PgDn" => b"\x1b[6~".to_vec(),
        "Delete" | "DC" => b"\x1b[3~".to_vec(),
        _ => {
            if let Some(c) = key.strip_prefix("C-").and_then(|r| {
                let mut chars = r.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => Some(c),
                    _ => None,
                }
            }) {
                // Ctrl-letter → control byte (C-c = 0x03).
                let upper = c.to_ascii_uppercase();
                if upper.is_ascii_uppercase() || ('@'..='_').contains(&upper) {
                    vec![(upper as u8) & 0x1f]
                } else {
                    return None;
                }
            } else if let Some(c) = key.strip_prefix("M-").and_then(|r| {
                let mut chars = r.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => Some(c),
                    _ => None,
                }
            }) {
                // Alt-letter → ESC prefix.
                let mut v = vec![0x1b];
                v.extend(c.to_string().into_bytes());
                v
            } else if key.chars().count() == 1 {
                key.as_bytes().to_vec()
            } else {
                return None;
            }
        }
    };
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_mapping() {
        assert_eq!(key_to_bytes("Enter").unwrap(), b"\r");
        assert_eq!(key_to_bytes("C-c").unwrap(), vec![0x03]);
        assert_eq!(key_to_bytes("Up").unwrap(), b"\x1b[A");
        assert_eq!(key_to_bytes("a").unwrap(), b"a");
        assert_eq!(key_to_bytes("M-f").unwrap(), b"\x1bf");
        assert!(key_to_bytes("NotAKey").is_none());
    }
}
