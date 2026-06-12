//! amux-core: the engine behind amux.
//!
//! The `Engine` owns all workspaces and panes and is the single source of
//! truth. Both the Tauri UI layer and the Unix-socket automation server call
//! into the same `Engine`; state changes fan out through a broadcast channel.
//!
//! Module map (filled in milestone by milestone):
//! - `engine`     — workspace/pane state machine, event broadcast (M1/M2)
//! - `layout`     — split-tree operations (M2)
//! - `pane`       — PTY spawn/read-loop/write/resize/kill (M1)
//! - `term_state` — server-side terminal grid for read-screen (M1)
//! - `osc`        — BEL / OSC notification parsing (M4)
//! - `meta`       — cwd / git branch / listening-port sweeper (M2)
//! - `notify`     — notification policy + desktop dispatch (M4)
//! - `server`     — NDJSON JSON-RPC Unix-socket server (M3)
//! - `persist`    — session save/restore (M5)

pub mod engine;
pub mod layout;
pub mod meta;
pub mod notify;
pub mod osc;
pub mod pane;
pub mod server;
pub mod term_state;

pub use engine::{Engine, EngineError, EngineEvent};
