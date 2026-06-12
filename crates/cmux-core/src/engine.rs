//! Engine skeleton (M0). PTY/layout logic lands in M1/M2.

use cmux_protocol::Snapshot;
use parking_lot::RwLock;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Events fanned out to the Tauri layer (→ webview) and other listeners.
#[derive(Debug, Clone)]
pub enum EngineEvent {
    /// Engine state changed; listeners should pull a fresh `Snapshot`.
    StateChanged,
}

pub struct Engine {
    state: RwLock<Snapshot>,
    events: broadcast::Sender<EngineEvent>,
}

impl Engine {
    pub fn new() -> Arc<Self> {
        let (events, _) = broadcast::channel(256);
        Arc::new(Self { state: RwLock::new(Snapshot::default()), events })
    }

    pub fn snapshot(&self) -> Snapshot {
        self.state.read().clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.events.subscribe()
    }
}
