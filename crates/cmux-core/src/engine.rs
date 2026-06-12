//! The engine: single source of truth for workspaces and panes.
//!
//! Both the Tauri UI layer and the Unix-socket automation server call the
//! same methods here. Every mutation broadcasts `StateChanged`; listeners
//! pull a fresh `Snapshot`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use cmux_protocol::{
    LayoutNode, PaneId, PaneInfo, Snapshot, SplitAxis, WorkspaceId, WorkspaceInfo,
};
use indexmap::IndexMap;
use parking_lot::RwLock;
use tokio::sync::broadcast;

use crate::layout;
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
    #[error("workspace not found: {0}")]
    WorkspaceNotFound(WorkspaceId),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

struct WorkspaceState {
    name: String,
    layout: LayoutNode,
    active_pane: PaneId,
}

#[derive(Default)]
struct Workspaces {
    /// Insertion-ordered; sidebar order = map order.
    map: IndexMap<WorkspaceId, WorkspaceState>,
    active: Option<WorkspaceId>,
    created_count: usize,
    pane_created_count: usize,
}

pub struct Engine {
    panes: RwLock<HashMap<PaneId, Arc<Pane>>>,
    workspaces: RwLock<Workspaces>,
    events: broadcast::Sender<EngineEvent>,
}

impl Engine {
    pub fn new() -> Arc<Self> {
        let (events, _) = broadcast::channel(256);
        Arc::new(Self {
            panes: RwLock::new(HashMap::new()),
            workspaces: RwLock::new(Workspaces::default()),
            events,
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.events.subscribe()
    }

    fn notify_state_changed(&self) {
        let _ = self.events.send(EngineEvent::StateChanged);
    }

    fn spawn_pane(
        self: &Arc<Self>,
        workspace: WorkspaceId,
        cols: u16,
        rows: u16,
        cwd: Option<std::path::PathBuf>,
    ) -> Result<Arc<Pane>, EngineError> {
        let id = PaneId::new();
        let name = {
            let mut ws = self.workspaces.write();
            ws.pane_created_count += 1;
            format!("터미널 {}", ws.pane_created_count)
        };
        let engine = Arc::downgrade(self);
        let pane = Pane::spawn(id, workspace, name, cols, rows, cwd, move || {
            // Shell exited → remove the pane from its layout, like tmux.
            if let Some(engine) = engine.upgrade() {
                let _ = engine.close_pane(id);
            }
        })?;
        self.panes.write().insert(id, Arc::clone(&pane));
        Ok(pane)
    }

    // -- workspaces ---------------------------------------------------------

    pub fn create_workspace(
        self: &Arc<Self>,
        name: Option<String>,
        cwd: Option<std::path::PathBuf>,
        cols: u16,
        rows: u16,
    ) -> Result<(WorkspaceId, PaneId), EngineError> {
        let ws_id = WorkspaceId::new();
        let pane = self.spawn_pane(ws_id, cols, rows, cwd)?;
        let mut ws = self.workspaces.write();
        ws.created_count += 1;
        let name = name.unwrap_or_else(|| format!("워크스페이스 {}", ws.created_count));
        ws.map.insert(
            ws_id,
            WorkspaceState { name, layout: LayoutNode::Leaf { pane: pane.id }, active_pane: pane.id },
        );
        ws.active = Some(ws_id);
        drop(ws);
        self.notify_state_changed();
        Ok((ws_id, pane.id))
    }

    pub fn close_workspace(&self, id: WorkspaceId) -> Result<(), EngineError> {
        let removed = {
            let mut ws = self.workspaces.write();
            let state = ws
                .map
                .shift_remove(&id)
                .ok_or(EngineError::WorkspaceNotFound(id))?;
            if ws.active == Some(id) {
                ws.active = ws.map.keys().last().copied();
            }
            state
        };
        let mut panes = self.panes.write();
        for pane_id in layout::panes(&removed.layout) {
            if let Some(pane) = panes.remove(&pane_id) {
                pane.kill();
            }
        }
        drop(panes);
        self.notify_state_changed();
        Ok(())
    }

    pub fn focus_workspace(&self, id: WorkspaceId) -> Result<(), EngineError> {
        let mut ws = self.workspaces.write();
        if !ws.map.contains_key(&id) {
            return Err(EngineError::WorkspaceNotFound(id));
        }
        ws.active = Some(id);
        drop(ws);
        self.notify_state_changed();
        Ok(())
    }

    pub fn rename_workspace(&self, id: WorkspaceId, name: String) -> Result<(), EngineError> {
        let mut ws = self.workspaces.write();
        let state = ws.map.get_mut(&id).ok_or(EngineError::WorkspaceNotFound(id))?;
        state.name = name;
        drop(ws);
        self.notify_state_changed();
        Ok(())
    }

    /// Reorder for sidebar drag & drop.
    pub fn move_workspace(&self, id: WorkspaceId, to_index: usize) -> Result<(), EngineError> {
        let mut ws = self.workspaces.write();
        let from = ws
            .map
            .get_index_of(&id)
            .ok_or(EngineError::WorkspaceNotFound(id))?;
        let to = to_index.min(ws.map.len() - 1);
        ws.map.move_index(from, to);
        drop(ws);
        self.notify_state_changed();
        Ok(())
    }

    // -- panes ---------------------------------------------------------------

    pub fn split_pane(
        self: &Arc<Self>,
        target: PaneId,
        axis: SplitAxis,
        cols: u16,
        rows: u16,
    ) -> Result<PaneId, EngineError> {
        let target_pane = self.pane(target)?;
        // New pane inherits the directory the target is currently in.
        let cwd = target_pane.meta.lock().cwd.clone().map(Into::into);
        let ws_id = target_pane.workspace;

        let pane = self.spawn_pane(ws_id, cols, rows, cwd)?;
        let mut ws = self.workspaces.write();
        let Some(state) = ws.map.get_mut(&ws_id) else {
            drop(ws);
            // Workspace vanished mid-split: don't leak the new pane.
            if let Some(p) = self.panes.write().remove(&pane.id) {
                p.kill();
            }
            return Err(EngineError::WorkspaceNotFound(ws_id));
        };
        layout::split(&mut state.layout, target, axis, pane.id);
        state.active_pane = pane.id;
        ws.active = Some(ws_id);
        drop(ws);
        self.notify_state_changed();
        Ok(pane.id)
    }

    pub fn close_pane(&self, id: PaneId) -> Result<(), EngineError> {
        let pane = {
            let mut panes = self.panes.write();
            panes.remove(&id).ok_or(EngineError::PaneNotFound(id))?
        };
        pane.kill();

        let ws_id = pane.workspace;
        let mut ws = self.workspaces.write();
        if let Some(state) = ws.map.get_mut(&ws_id) {
            match layout::remove(state.layout.clone(), id) {
                Some(layout) => {
                    if state.active_pane == id {
                        state.active_pane = *layout::panes(&layout).first().expect("non-empty");
                    }
                    state.layout = layout;
                }
                None => {
                    // Last pane: the workspace goes too.
                    ws.map.shift_remove(&ws_id);
                    if ws.active == Some(ws_id) {
                        ws.active = ws.map.keys().last().copied();
                    }
                }
            }
        }
        drop(ws);
        self.notify_state_changed();
        Ok(())
    }

    pub fn rename_pane(&self, id: PaneId, name: String) -> Result<(), EngineError> {
        *self.pane(id)?.name.lock() = name;
        self.notify_state_changed();
        Ok(())
    }

    pub fn focus_pane(&self, id: PaneId) -> Result<(), EngineError> {
        let pane = self.pane(id)?;
        let mut ws = self.workspaces.write();
        let state = ws
            .map
            .get_mut(&pane.workspace)
            .ok_or(EngineError::WorkspaceNotFound(pane.workspace))?;
        state.active_pane = id;
        ws.active = Some(pane.workspace);
        drop(ws);
        self.notify_state_changed();
        Ok(())
    }

    pub fn set_ratio(
        &self,
        workspace: WorkspaceId,
        path: &[bool],
        ratio: f32,
    ) -> Result<(), EngineError> {
        let mut ws = self.workspaces.write();
        let state = ws
            .map
            .get_mut(&workspace)
            .ok_or(EngineError::WorkspaceNotFound(workspace))?;
        layout::set_ratio(&mut state.layout, path, ratio);
        drop(ws);
        self.notify_state_changed();
        Ok(())
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

    // -- snapshot & background tasks -----------------------------------------

    pub fn snapshot(&self) -> Snapshot {
        let ws = self.workspaces.read();
        let panes = self.panes.read();
        Snapshot {
            workspaces: ws
                .map
                .iter()
                .map(|(id, s)| WorkspaceInfo {
                    id: *id,
                    name: s.name.clone(),
                    layout: s.layout.clone(),
                    active_pane: Some(s.active_pane),
                })
                .collect(),
            panes: panes
                .values()
                .map(|p| PaneInfo {
                    id: p.id,
                    workspace: p.workspace,
                    name: p.name.lock().clone(),
                    meta: p.meta.lock().clone(),
                    notification: None, // M4
                    exited: p.has_exited(),
                })
                .collect(),
            active_workspace: ws.active,
        }
    }

    /// Background thread polling pane metadata (cwd / git branch / ports).
    /// Emits a state change only when something actually differs. A plain
    /// thread (not a tokio task): the work is all synchronous /proc reads.
    pub fn start_meta_sweeper(self: &Arc<Self>) {
        let engine = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("meta-sweeper".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_secs(2));
                let Some(engine) = engine.upgrade() else { break };
                let panes: Vec<Arc<Pane>> = engine.panes.read().values().cloned().collect();
                let mut changed = false;
                for pane in panes {
                    let fresh = crate::meta::compute(&pane);
                    let mut slot = pane.meta.lock();
                    if *slot != fresh {
                        *slot = fresh;
                        changed = true;
                    }
                }
                if changed {
                    engine.notify_state_changed();
                }
            })
            .expect("spawn meta sweeper");
    }

    /// Kill every pane (app shutdown).
    pub fn shutdown(&self) {
        for pane in self.panes.write().drain().map(|(_, p)| p) {
            pane.kill();
        }
    }
}
