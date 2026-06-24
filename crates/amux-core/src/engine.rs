//! The engine: single source of truth for workspaces and panes.
//!
//! Both the Tauri UI layer and the Unix-socket automation server call the
//! same methods here. Every mutation broadcasts `StateChanged`; listeners
//! pull a fresh `Snapshot`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use std::collections::VecDeque;

use amux_protocol::{
    LayoutNode, NotificationEntry, NotifyKind, PaneId, PaneInfo, PaneNotification, PaneStatus,
    Snapshot, SplitAxis, WorkspaceId, WorkspaceInfo,
};
use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};
use tokio::sync::broadcast;

use crate::layout;
use crate::osc::OscEvent;
use crate::pane::{OutputSink, Pane};

/// Events fanned out to the Tauri layer (→ webview) and other listeners.
#[derive(Debug, Clone)]
pub enum EngineEvent {
    /// Engine state changed; listeners should pull a fresh `Snapshot`.
    StateChanged,
    /// A pane wants attention right now (drives the UI highlight ring).
    PaneRing(PaneId),
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

const HISTORY_CAP: usize = 200;

pub struct Engine {
    panes: RwLock<HashMap<PaneId, Arc<Pane>>>,
    workspaces: RwLock<Workspaces>,
    events: broadcast::Sender<EngineEvent>,
    window_focused: AtomicBool,
    history: Mutex<VecDeque<NotificationEntry>>,
}

impl Engine {
    pub fn new() -> Arc<Self> {
        let (events, _) = broadcast::channel(256);
        Arc::new(Self {
            panes: RwLock::new(HashMap::new()),
            workspaces: RwLock::new(Workspaces::default()),
            events,
            window_focused: AtomicBool::new(true),
            history: Mutex::new(VecDeque::new()),
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
        let engine_for_osc = Arc::downgrade(self);
        let pane = Pane::spawn(
            id,
            workspace,
            name,
            cols,
            rows,
            cwd,
            move || {
                // Shell exited → remove the pane from its layout, like tmux.
                if let Some(engine) = engine.upgrade() {
                    let _ = engine.close_pane(id);
                }
            },
            move |event| {
                if let Some(engine) = engine_for_osc.upgrade() {
                    let (kind, title, body) = match event {
                        OscEvent::Bell => (NotifyKind::Bell, None, None),
                        OscEvent::Notify { title, body } => {
                            (NotifyKind::Attention, title, body)
                        }
                    };
                    engine.notify_pane(id, kind, title, body);
                }
            },
        )?;
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
        let visible_pane = ws.map.get(&id).map(|s| s.active_pane);
        drop(ws);
        if let Some(pane) = visible_pane {
            if let Ok(pane) = self.pane(pane) {
                *pane.notification.lock() = None;
            }
            self.mark_pane_seen(pane);
        }
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

    // -- notifications --------------------------------------------------------

    /// Is this pane the one the user is looking at right now?
    fn pane_visible_and_focused(&self, id: PaneId) -> bool {
        if !self.window_focused.load(Ordering::SeqCst) {
            return false;
        }
        let Ok(pane) = self.pane(id) else { return false };
        let ws = self.workspaces.read();
        ws.active == Some(pane.workspace)
            && ws.map.get(&pane.workspace).is_some_and(|s| s.active_pane == id)
    }

    /// Notification pipeline shared by OSC detection, lifecycle hooks, and
    /// the socket API. Status side-effects always apply; the noisy parts
    /// (badge, desktop notification, ring, history) are suppressed when the
    /// user is already looking at the pane.
    pub fn notify_pane(
        &self,
        id: PaneId,
        kind: NotifyKind,
        title: Option<String>,
        body: Option<String>,
    ) {
        let Ok(pane) = self.pane(id) else { return };
        let visible = self.pane_visible_and_focused(id);

        // Status side-effects per kind:
        // - attention/bell → the app waits for the user
        // - progress (UserPromptSubmit hook) → work started; quiet signal
        // - done (Stop hook) → work finished
        // progress/done mark the pane hook-managed: lifecycle hooks are
        // authoritative from then on, the silence heuristic stands down.
        match kind {
            NotifyKind::Attention | NotifyKind::Bell => {
                *pane.waiting_since.lock() = Some(std::time::Instant::now());
                *pane.status.lock() = PaneStatus::Waiting;
            }
            NotifyKind::Progress => {
                pane.hook_managed.store(true, Ordering::SeqCst);
                *pane.waiting_since.lock() = None;
                *pane.status.lock() = PaneStatus::Processing;
                self.notify_state_changed();
                return; // quiet: no desktop notification, no ring, no history
            }
            NotifyKind::Done => {
                pane.hook_managed.store(true, Ordering::SeqCst);
                *pane.waiting_since.lock() = None;
                *pane.status.lock() =
                    if visible { PaneStatus::Idle } else { PaneStatus::Processed };
            }
            NotifyKind::Idle => {
                // SessionStart hook: the app declares itself idle and
                // hook-managed — no work in flight, heuristic stands down.
                pane.hook_managed.store(true, Ordering::SeqCst);
                *pane.waiting_since.lock() = None;
                *pane.status.lock() = PaneStatus::Idle;
                self.notify_state_changed();
                return; // quiet: a fresh session is not an announcement
            }
        }

        // The user is already looking at this pane — announce nothing.
        if visible {
            self.notify_state_changed();
            return;
        }

        let title = title.unwrap_or_else(|| crate::notify::default_title(kind).to_string());
        *pane.notification.lock() = Some(PaneNotification {
            kind,
            title: Some(title.clone()),
            body: body.clone(),
        });
        crate::notify::send_desktop(kind, &title, body.as_deref().unwrap_or(""));
        let _ = self.events.send(EngineEvent::PaneRing(id));
        let entry = NotificationEntry {
            pane: id,
            pane_name: pane.name.lock().clone(),
            kind,
            title: Some(title),
            body,
            at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        };
        let mut history = self.history.lock();
        history.push_front(entry);
        history.truncate(HISTORY_CAP);
        drop(history);
        self.notify_state_changed();
    }

    pub fn clear_notification_history(&self) {
        self.history.lock().clear();
        self.notify_state_changed();
    }

    fn clear_notification(&self, id: PaneId) {
        if let Ok(pane) = self.pane(id) {
            if pane.notification.lock().take().is_some() {
                self.notify_state_changed();
            }
        }
    }

    /// Window focus changes (from the windowing layer). Regaining focus
    /// clears the visible pane's pending notification.
    pub fn set_window_focused(&self, focused: bool) {
        self.window_focused.store(focused, Ordering::SeqCst);
        if focused {
            let visible = {
                let ws = self.workspaces.read();
                ws.active.and_then(|a| ws.map.get(&a)).map(|s| s.active_pane)
            };
            if let Some(pane) = visible {
                self.clear_notification(pane);
                self.mark_pane_seen(pane);
                self.notify_state_changed();
            }
        }
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
        // Looking at it now — its pending notification is acknowledged.
        *pane.notification.lock() = None;
        self.mark_pane_seen(id);
        self.notify_state_changed();
        Ok(())
    }

    /// Drag-rearrange: detach `pane` from its position and re-insert it as a
    /// split of `target` (same workspace). `before` puts it left/top.
    pub fn move_pane(
        &self,
        pane_id: PaneId,
        target: PaneId,
        axis: SplitAxis,
        before: bool,
    ) -> Result<(), EngineError> {
        if pane_id == target {
            return Ok(());
        }
        let pane = self.pane(pane_id)?;
        let target_pane = self.pane(target)?;
        if pane.workspace != target_pane.workspace {
            return Err(EngineError::Other(anyhow::anyhow!(
                "panes are in different workspaces"
            )));
        }
        let mut ws = self.workspaces.write();
        let state = ws
            .map
            .get_mut(&pane.workspace)
            .ok_or(EngineError::WorkspaceNotFound(pane.workspace))?;
        // Detach (keeps the pane process alive), then re-insert next to target.
        let Some(without) = layout::remove(state.layout.clone(), pane_id) else {
            return Ok(()); // it's the only pane — nothing to rearrange
        };
        let mut layout = without;
        if !layout::split_insert(&mut layout, target, axis, pane_id, before) {
            return Err(EngineError::PaneNotFound(target));
        }
        state.layout = layout;
        state.active_pane = pane_id;
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
        self.pane(id)?.write_input(data).map_err(EngineError::Other)
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
                    notification: p.notification.lock().clone(),
                    status: *p.status.lock(),
                    exited: p.has_exited(),
                })
                .collect(),
            active_workspace: ws.active,
            notifications: self.history.lock().iter().cloned().collect(),
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
                std::thread::sleep(Duration::from_secs(1));
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
                    drop(slot);
                    changed |= engine.sweep_status(&pane);
                }
                if changed {
                    engine.notify_state_changed();
                }
            })
            .expect("spawn meta sweeper");
    }

    /// Work-status state machine, driven every sweep tick. A pane is always
    /// in exactly one of four states:
    /// - processing (red): work in progress (work output within the last 4s)
    /// - processed (green): work finished, user hasn't looked yet
    /// - idle (blue): nothing in flight, latest result already seen
    /// - waiting (yellow): waiting-for-input signal (hook/bell)
    ///
    /// "Work output" excludes echo: bytes arriving right after user input
    /// (typing, prompt repaint) never count — see `Pane::write_input`.
    fn sweep_status(&self, pane: &Arc<Pane>) -> bool {
        const SILENCE: Duration = Duration::from_secs(4);
        const MIN_BURST: Duration = Duration::from_secs(2);

        let activity = *pane.activity.lock();
        let silent_for = activity.last_output.elapsed();
        let burst_len = activity.last_output.duration_since(activity.burst_start);
        let app_running = {
            let fg = pane.shell_pid();
            fg.is_some() && fg != pane.child_pid()
        };

        let hook_managed = pane.hook_managed.load(Ordering::SeqCst);
        let mut status = pane.status.lock();
        let old = *status;

        // In-flight work resolves to processed (or straight to idle when the
        // user is already looking); settled states stay as they are.
        let finished = |old: PaneStatus| match old {
            PaneStatus::Processing | PaneStatus::Waiting => {
                if self.pane_visible_and_focused(pane.id) {
                    PaneStatus::Idle
                } else {
                    PaneStatus::Processed
                }
            }
            settled => settled,
        };

        // Lifecycle hooks own the status while the app runs: TUIs like
        // Claude Code repaint constantly, so the heuristic below would
        // misread them. Once the app exits, resolve whatever was in flight
        // and hand control back to the heuristic.
        if hook_managed {
            let new = if app_running {
                old
            } else {
                pane.hook_managed.store(false, Ordering::SeqCst);
                *pane.waiting_since.lock() = None;
                finished(old)
            };
            *status = new;
            return old != new;
        }

        // Heuristic path: a waiting signal is consumed once work output
        // resumed after it (2s grace — the app repaints while *showing*
        // the prompt too).
        {
            let mut waiting = pane.waiting_since.lock();
            if let Some(since) = *waiting {
                if activity.last_output > since + Duration::from_secs(2) {
                    *waiting = None;
                }
            }
        }
        let waiting = pane.waiting_since.lock().is_some();

        let new = if !app_running {
            // Foreground is the shell again — the command (if any) is done.
            *pane.waiting_since.lock() = None;
            finished(old)
        } else if waiting {
            PaneStatus::Waiting
        } else if silent_for < SILENCE {
            PaneStatus::Processing
        } else {
            // App still running but quiet: a real burst (≥2s) just wrapped
            // up; short blips weren't work in the first place.
            match old {
                PaneStatus::Processing | PaneStatus::Waiting => {
                    if burst_len >= MIN_BURST {
                        finished(old)
                    } else {
                        PaneStatus::Idle
                    }
                }
                settled => settled,
            }
        };
        *status = new;
        old != new
    }

    /// The user is looking at this pane now: in-flight `processed` and
    /// input-`waiting` both resolve to idle (focusing acknowledges the wait),
    /// and this pane's notification history clears so unread entries in the
    /// sidebar panel don't pile up.
    fn mark_pane_seen(&self, id: PaneId) {
        let Ok(pane) = self.pane(id) else { return };
        {
            let mut status = pane.status.lock();
            if matches!(*status, PaneStatus::Processed | PaneStatus::Waiting) {
                *status = PaneStatus::Idle;
            }
        }
        *pane.waiting_since.lock() = None;
        self.history.lock().retain(|e| e.pane != id);
    }

    /// Kill every pane (app shutdown).
    pub fn shutdown(&self) {
        for pane in self.panes.write().drain().map(|(_, p)| p) {
            pane.kill();
        }
    }
}
