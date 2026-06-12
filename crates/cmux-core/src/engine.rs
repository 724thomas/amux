//! The engine: single source of truth for workspaces and panes.
//!
//! M1 scope: a flat pane registry plus one implicit workspace per pane.
//! M2 replaces the implicit workspaces with a real layout tree.

use std::collections::HashMap;
use std::sync::Arc;

use cmux_protocol::{PaneId, PaneInfo, PaneMeta, Snapshot, WorkspaceId};
use parking_lot::RwLock;
use tokio::sync::broadcast;

use crate::pane::{OutputSink, Pane};

/// Events fanned out to the Tauri layer (→ webview) and other listeners.
#[derive(Debug, Clone)]
pub enum EngineEvent {
    /// Engine state changed; listeners should pull a fresh `Snapshot`.
    StateChanged,
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("pane not found: {0}")]
    PaneNotFound(PaneId),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub struct Engine {
    panes: RwLock<HashMap<PaneId, Arc<Pane>>>,
    events: broadcast::Sender<EngineEvent>,
}

impl Engine {
    pub fn new() -> Arc<Self> {
        let (events, _) = broadcast::channel(256);
        Arc::new(Self { panes: RwLock::new(HashMap::new()), events })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.events.subscribe()
    }

    fn notify_state_changed(&self) {
        let _ = self.events.send(EngineEvent::StateChanged);
    }

    pub fn create_pane(
        self: &Arc<Self>,
        cols: u16,
        rows: u16,
        cwd: Option<std::path::PathBuf>,
    ) -> Result<PaneId, EngineError> {
        let id = PaneId::new();
        let workspace = WorkspaceId::new();
        let engine = Arc::downgrade(self);
        let pane = Pane::spawn(id, workspace, cols, rows, cwd, move || {
            if let Some(engine) = engine.upgrade() {
                engine.notify_state_changed();
            }
        })?;
        self.panes.write().insert(id, pane);
        self.notify_state_changed();
        Ok(id)
    }

    pub fn pane(&self, id: PaneId) -> Result<Arc<Pane>, EngineError> {
        self.panes
            .read()
            .get(&id)
            .cloned()
            .ok_or(EngineError::PaneNotFound(id))
    }

    pub fn write_pane(&self, id: PaneId, data: &[u8]) -> Result<(), EngineError> {
        self.pane(id)?.write(data).map_err(EngineError::Other)
    }

    pub fn resize_pane(&self, id: PaneId, cols: u16, rows: u16) -> Result<(), EngineError> {
        self.pane(id)?.resize(cols, rows).map_err(EngineError::Other)
    }

    pub fn read_screen(&self, id: PaneId) -> Result<String, EngineError> {
        Ok(self.pane(id)?.term.read_screen())
    }

    pub fn subscribe_pane(&self, id: PaneId, sink: OutputSink) -> Result<(), EngineError> {
        self.pane(id)?.set_sink(sink);
        Ok(())
    }

    pub fn close_pane(&self, id: PaneId) -> Result<(), EngineError> {
        let pane = self
            .panes
            .write()
            .remove(&id)
            .ok_or(EngineError::PaneNotFound(id))?;
        pane.kill();
        self.notify_state_changed();
        Ok(())
    }

    pub fn snapshot(&self) -> Snapshot {
        let panes = self.panes.read();
        Snapshot {
            workspaces: Vec::new(), // M2
            panes: panes
                .values()
                .map(|p| PaneInfo {
                    id: p.id,
                    workspace: p.workspace,
                    meta: PaneMeta::default(),
                    notification: None,
                    exited: p.has_exited(),
                })
                .collect(),
            active_workspace: None,
        }
    }

    /// Kill every pane (app shutdown).
    pub fn shutdown(&self) {
        for pane in self.panes.write().drain().map(|(_, p)| p) {
            pane.kill();
        }
    }
}
