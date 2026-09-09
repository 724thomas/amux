//! The engine: single source of truth for workspaces and panes.
//!
//! Both the Tauri UI layer and the Unix-socket automation server call the
//! same methods here. Every mutation broadcasts `StateChanged`; listeners
//! pull a fresh `Snapshot`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use std::collections::VecDeque;

use amux_protocol::{
    LayoutNode, NotificationEntry, NotifyKind, PaneId, PaneInfo, PaneNotification, PaneStatus,
    Snapshot, SplitAxis, TabId, TabInfo, WorkspaceId, WorkspaceInfo,
};
use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};
use tokio::sync::broadcast;

use crate::layout;
use crate::osc::OscEvent;
use crate::pane::{OutputSink, Pane};
use crate::session::{self, ResumeKey, ResumeMode, ResumePrefs, SavedLayout, SavedSession, SavedTab,
    SavedWorkspace};

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
    #[error("tab not found: {0}")]
    TabNotFound(TabId),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// One tab: the named unit the user sees, owning its own split tree. Panes are
/// the leaves of that tree and carry no name of their own.
struct TabState {
    name: String,
    layout: LayoutNode,
    active_pane: PaneId,
}

struct WorkspaceState {
    name: String,
    /// Insertion-ordered; tab-bar order = map order.
    tabs: IndexMap<TabId, TabState>,
    active_tab: TabId,
    /// Per-workspace, so a fresh workspace's first tab is always `탭 1` rather
    /// than continuing a global count. Never decremented — reusing a closed
    /// tab's number would put two `탭 2`s in one tab bar.
    tab_created_count: usize,
}

impl WorkspaceState {
    /// Reserve the next auto-name for a tab in this workspace.
    fn next_tab_name(&mut self) -> String {
        self.tab_created_count += 1;
        format!("탭 {}", self.tab_created_count)
    }

    fn active_tab(&self) -> Option<&TabState> {
        self.tabs.get(&self.active_tab)
    }

    /// The one pane on screen for this workspace: the active tab's active pane.
    fn visible_pane(&self) -> Option<PaneId> {
        self.active_tab().map(|t| t.active_pane)
    }

    /// Drop `tab` and hand `active_tab` to whatever slid into its slot (the
    /// right-hand neighbour, or the new last tab). Returns the tab's panes.
    fn remove_tab(&mut self, tab: TabId) -> Vec<PaneId> {
        let Some(index) = self.tabs.get_index_of(&tab) else { return Vec::new() };
        let removed = self.tabs.shift_remove(&tab).expect("index resolved above");
        if self.active_tab == tab {
            if let Some((next, _)) = self.tabs.get_index(index.min(self.tabs.len().saturating_sub(1)))
            {
                self.active_tab = *next;
            }
        }
        layout::panes(&removed.layout)
    }
}

#[derive(Default)]
struct Workspaces {
    /// Insertion-ordered; sidebar order = map order.
    map: IndexMap<WorkspaceId, WorkspaceState>,
    active: Option<WorkspaceId>,
    created_count: usize,
}

impl Workspaces {
    /// Which workspace owns `tab`. The CLI addresses tabs by id alone, so the
    /// lookup has to scan; workspace counts are tiny (tens at most).
    fn workspace_of_tab(&self, tab: TabId) -> Option<WorkspaceId> {
        self.map
            .iter()
            .find(|(_, s)| s.tabs.contains_key(&tab))
            .map(|(id, _)| *id)
    }

    /// Forget an empty workspace, moving `active` off it if needed.
    fn drop_workspace(&mut self, id: WorkspaceId) {
        self.map.shift_remove(&id);
        if self.active == Some(id) {
            self.active = self.map.keys().last().copied();
        }
    }
}

const HISTORY_CAP: usize = 200;

/// How long a restore waits for a shell to print its prompt before typing the
/// `claude --resume` line at it anyway. Generous: the shells of a big restore
/// all start at once and compete for the machine.
const RESUME_READY_TIMEOUT: Duration = Duration::from_secs(5);
/// How long to keep watching a restored pane for Claude Code's summary-or-full
/// menu. Generous because a large conversation takes a while to draw, and cheap
/// because the panes are polled together: a pane whose menu never appears (a
/// short conversation, or a machine where "Don't ask me again" was chosen)
/// costs the others nothing.
const RESUME_DIALOG_TIMEOUT: Duration = Duration::from_secs(60);

/// Scratch state threaded through a restore's walk of one tab's saved tree.
#[derive(Default)]
struct Respawn {
    /// New panes, in `layout::panes()` order.
    panes: Vec<PaneId>,
    /// Panes to hand a Claude conversation back to, once their shells are up.
    resumes: Vec<(PaneId, String)>,
}

pub struct Engine {
    panes: RwLock<HashMap<PaneId, Arc<Pane>>>,
    workspaces: RwLock<Workspaces>,
    events: broadcast::Sender<EngineEvent>,
    window_focused: AtomicBool,
    history: Mutex<VecDeque<NotificationEntry>>,
    /// Set by `shutdown`. Stops the session autosave from writing the
    /// half-torn-down state it would otherwise see on the way out.
    shutting_down: AtomicBool,
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
            shutting_down: AtomicBool::new(false),
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
        tab: TabId,
        cols: u16,
        rows: u16,
        cwd: Option<std::path::PathBuf>,
    ) -> Result<Arc<Pane>, EngineError> {
        let id = PaneId::new();
        let engine = Arc::downgrade(self);
        let engine_for_osc = Arc::downgrade(self);
        let pane = Pane::spawn(
            id,
            workspace,
            tab,
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

    /// A workspace is never empty: it is born with one tab holding one pane.
    /// `tab_name` names that first tab — the UI asks for it in the same breath
    /// as the workspace title, so the tab you land in isn't a nameless `탭 1`.
    pub fn create_workspace(
        self: &Arc<Self>,
        name: Option<String>,
        tab_name: Option<String>,
        cwd: Option<std::path::PathBuf>,
        cols: u16,
        rows: u16,
    ) -> Result<(WorkspaceId, TabId, PaneId), EngineError> {
        let ws_id = WorkspaceId::new();
        let tab_id = TabId::new();
        let pane = self.spawn_pane(ws_id, tab_id, cols, rows, cwd)?;
        let mut ws = self.workspaces.write();
        ws.created_count += 1;
        let name = name.unwrap_or_else(|| format!("워크스페이스 {}", ws.created_count));
        let mut tabs = IndexMap::new();
        tabs.insert(
            tab_id,
            TabState {
                name: tab_name.unwrap_or_else(|| "탭 1".into()),
                layout: LayoutNode::Leaf { pane: pane.id },
                active_pane: pane.id,
            },
        );
        ws.map.insert(
            ws_id,
            WorkspaceState { name, tabs, active_tab: tab_id, tab_created_count: 1 },
        );
        ws.active = Some(ws_id);
        drop(ws);
        self.notify_state_changed();
        Ok((ws_id, tab_id, pane.id))
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
        // Every tab in the workspace goes, and with it every pane in each tab.
        for tab in removed.tabs.values() {
            for pane_id in layout::panes(&tab.layout) {
                if let Some(pane) = panes.remove(&pane_id) {
                    pane.kill();
                }
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
        let visible_pane = ws.map.get(&id).and_then(|s| s.visible_pane());
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

    // -- tabs -----------------------------------------------------------------

    /// Open a tab in `workspace` with one fresh pane. The pane inherits the
    /// directory the workspace's currently visible pane sits in, so a new tab
    /// starts where the user is working rather than at `$HOME`.
    pub fn new_tab(
        self: &Arc<Self>,
        workspace: WorkspaceId,
        name: Option<String>,
        cols: u16,
        rows: u16,
    ) -> Result<(TabId, PaneId), EngineError> {
        let cwd = {
            let ws = self.workspaces.read();
            let visible = ws
                .map
                .get(&workspace)
                .ok_or(EngineError::WorkspaceNotFound(workspace))?
                .visible_pane();
            drop(ws);
            visible
                .and_then(|p| self.pane(p).ok())
                .and_then(|p| p.meta.lock().cwd.clone())
                .map(Into::into)
        };

        let tab_id = TabId::new();
        let pane = self.spawn_pane(workspace, tab_id, cols, rows, cwd)?;
        let mut ws = self.workspaces.write();
        if !ws.map.contains_key(&workspace) {
            drop(ws);
            // Workspace vanished mid-create: don't leak the new pane.
            if let Some(p) = self.panes.write().remove(&pane.id) {
                p.kill();
            }
            return Err(EngineError::WorkspaceNotFound(workspace));
        }
        let state = ws.map.get_mut(&workspace).expect("checked above");
        let tab_name = name.unwrap_or_else(|| state.next_tab_name());
        state.tabs.insert(
            tab_id,
            TabState {
                name: tab_name,
                layout: LayoutNode::Leaf { pane: pane.id },
                active_pane: pane.id,
            },
        );
        state.active_tab = tab_id;
        ws.active = Some(workspace);
        drop(ws);
        self.notify_state_changed();
        Ok((tab_id, pane.id))
    }

    /// Close a tab and every pane in it. Emptying the workspace closes that too.
    pub fn close_tab(&self, tab: TabId) -> Result<(), EngineError> {
        let doomed = {
            let mut ws = self.workspaces.write();
            let ws_id = ws.workspace_of_tab(tab).ok_or(EngineError::TabNotFound(tab))?;
            let state = ws.map.get_mut(&ws_id).expect("resolved above");
            let panes = state.remove_tab(tab);
            if state.tabs.is_empty() {
                ws.drop_workspace(ws_id);
            }
            panes
        };
        let mut panes = self.panes.write();
        for pane_id in doomed {
            if let Some(pane) = panes.remove(&pane_id) {
                pane.kill();
            }
        }
        drop(panes);
        self.notify_state_changed();
        Ok(())
    }

    /// Bring a tab on screen (and its workspace with it).
    pub fn focus_tab(&self, tab: TabId) -> Result<(), EngineError> {
        let visible_pane = {
            let mut ws = self.workspaces.write();
            let ws_id = ws.workspace_of_tab(tab).ok_or(EngineError::TabNotFound(tab))?;
            ws.active = Some(ws_id);
            let state = ws.map.get_mut(&ws_id).expect("resolved above");
            state.active_tab = tab;
            state.visible_pane()
        };
        if let Some(pane) = visible_pane {
            if let Ok(pane) = self.pane(pane) {
                *pane.notification.lock() = None;
            }
            self.mark_pane_seen(pane);
        }
        self.notify_state_changed();
        Ok(())
    }

    pub fn rename_tab(&self, tab: TabId, name: String) -> Result<(), EngineError> {
        let mut ws = self.workspaces.write();
        let ws_id = ws.workspace_of_tab(tab).ok_or(EngineError::TabNotFound(tab))?;
        let state = ws.map.get_mut(&ws_id).expect("resolved above");
        state.tabs.get_mut(&tab).expect("resolved above").name = name;
        drop(ws);
        self.notify_state_changed();
        Ok(())
    }

    /// Reorder for tab-bar drag & drop.
    pub fn move_tab(&self, tab: TabId, to_index: usize) -> Result<(), EngineError> {
        let mut ws = self.workspaces.write();
        let ws_id = ws.workspace_of_tab(tab).ok_or(EngineError::TabNotFound(tab))?;
        let state = ws.map.get_mut(&ws_id).expect("resolved above");
        let from = state.tabs.get_index_of(&tab).expect("resolved above");
        let to = to_index.min(state.tabs.len() - 1);
        state.tabs.move_index(from, to);
        drop(ws);
        self.notify_state_changed();
        Ok(())
    }

    /// The tab a pane belongs to (its user-visible name lives there).
    pub fn tab_of_pane(&self, id: PaneId) -> Result<TabId, EngineError> {
        Ok(*self.pane(id)?.tab.lock())
    }

    // -- panes ---------------------------------------------------------------

    /// Split within the target's tab — the split tree is per-tab, so a split
    /// never reaches across tabs.
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
        let tab_id = *target_pane.tab.lock();

        let pane = self.spawn_pane(ws_id, tab_id, cols, rows, cwd)?;
        let mut ws = self.workspaces.write();
        let Some(wstate) = ws.map.get_mut(&ws_id) else {
            drop(ws);
            // Workspace vanished mid-split: don't leak the new pane.
            if let Some(p) = self.panes.write().remove(&pane.id) {
                p.kill();
            }
            return Err(EngineError::WorkspaceNotFound(ws_id));
        };
        let Some(tstate) = wstate.tabs.get_mut(&tab_id) else {
            drop(ws);
            if let Some(p) = self.panes.write().remove(&pane.id) {
                p.kill();
            }
            return Err(EngineError::TabNotFound(tab_id));
        };
        layout::split(&mut tstate.layout, target, axis, pane.id);
        tstate.active_pane = pane.id;
        wstate.active_tab = tab_id;
        ws.active = Some(ws_id);
        drop(ws);
        self.notify_state_changed();
        Ok(pane.id)
    }

    /// Close one pane. Emptying its tab closes the tab, and emptying the
    /// workspace closes that in turn — the same cascade a shell exit triggers.
    pub fn close_pane(&self, id: PaneId) -> Result<(), EngineError> {
        let pane = {
            let mut panes = self.panes.write();
            panes.remove(&id).ok_or(EngineError::PaneNotFound(id))?
        };
        pane.kill();

        let ws_id = pane.workspace;
        let tab_id = *pane.tab.lock();
        let mut ws = self.workspaces.write();
        let mut workspace_emptied = false;
        if let Some(wstate) = ws.map.get_mut(&ws_id) {
            let tab_emptied = match wstate.tabs.get_mut(&tab_id) {
                Some(tstate) => match layout::remove(tstate.layout.clone(), id) {
                    Some(layout) => {
                        if tstate.active_pane == id {
                            tstate.active_pane =
                                *layout::panes(&layout).first().expect("non-empty");
                        }
                        tstate.layout = layout;
                        false
                    }
                    None => true, // last pane in the tab
                },
                None => false,
            };
            if tab_emptied {
                wstate.remove_tab(tab_id);
                workspace_emptied = wstate.tabs.is_empty();
            }
        }
        if workspace_emptied {
            ws.drop_workspace(ws_id);
        }
        drop(ws);
        self.notify_state_changed();
        Ok(())
    }

    // -- notifications --------------------------------------------------------

    /// Is this pane the one the user is looking at right now? Three things
    /// have to line up: its workspace is on screen, its tab is the one showing
    /// in that workspace, and it is the focused pane inside that tab.
    fn pane_visible_and_focused(&self, id: PaneId) -> bool {
        if !self.window_focused.load(Ordering::SeqCst) {
            return false;
        }
        let Ok(pane) = self.pane(id) else { return false };
        let tab_id = *pane.tab.lock();
        let ws = self.workspaces.read();
        ws.active == Some(pane.workspace)
            && ws.map.get(&pane.workspace).is_some_and(|s| {
                s.active_tab == tab_id && s.visible_pane() == Some(id)
            })
    }

    /// Name of the tab a pane lives in — the label the user sees for it.
    fn tab_name_of(&self, pane: &Pane) -> String {
        let tab_id = *pane.tab.lock();
        let ws = self.workspaces.read();
        ws.map
            .get(&pane.workspace)
            .and_then(|s| s.tabs.get(&tab_id))
            .map(|t| t.name.clone())
            .unwrap_or_default()
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
        // Hooks are fire-and-forget shell commands, so the final PostToolUse
        // `progress` of a turn can be *delivered* just after that turn's `Stop`
        // `done`. A `progress` within this window after a `done` is a same-turn
        // straggler and must not resurrect `processing`.
        const DONE_GRACE: Duration = Duration::from_secs(2);

        match kind {
            NotifyKind::Attention | NotifyKind::Bell => {
                // Mid-turn → a genuine permission/input request (waiting). But on
                // a hook-managed pane whose turn already ended, this is Claude's
                // idle "waiting for your input" notification firing after the
                // task finished — it must not flip `processed` back to waiting.
                if pane.hook_managed.load(Ordering::SeqCst)
                    && !pane.turn_active.load(Ordering::SeqCst)
                {
                    self.notify_state_changed();
                    return;
                }
                *pane.waiting_since.lock() = Some(std::time::Instant::now());
                self.set_pane_status(&pane, PaneStatus::Waiting);
            }
            NotifyKind::Progress => {
                // Drop a same-turn straggler arriving just after `done`.
                let straggler =
                    (*pane.last_done_at.lock()).is_some_and(|t| t.elapsed() < DONE_GRACE);
                if straggler {
                    return;
                }
                pane.hook_managed.store(true, Ordering::SeqCst);
                pane.turn_active.store(true, Ordering::SeqCst);
                *pane.last_done_at.lock() = None;
                *pane.waiting_since.lock() = None;
                self.set_pane_status(&pane, PaneStatus::Processing);
                self.notify_state_changed();
                return; // quiet: no desktop notification, no ring, no history
            }
            NotifyKind::Done => {
                pane.hook_managed.store(true, Ordering::SeqCst);
                pane.turn_active.store(false, Ordering::SeqCst);
                *pane.last_done_at.lock() = Some(std::time::Instant::now());
                *pane.waiting_since.lock() = None;
                let resolved = if visible { PaneStatus::Idle } else { PaneStatus::Processed };
                self.set_pane_status(&pane, resolved);
            }
            NotifyKind::Idle => {
                // SessionStart hook: the app declares itself idle and
                // hook-managed — no work in flight, heuristic stands down.
                pane.hook_managed.store(true, Ordering::SeqCst);
                pane.turn_active.store(false, Ordering::SeqCst);
                *pane.last_done_at.lock() = None;
                *pane.waiting_since.lock() = None;
                self.set_pane_status(&pane, PaneStatus::Idle);
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
            tab_name: self.tab_name_of(&pane),
            kind,
            title: Some(title),
            body,
            at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        };
        let mut history = self.history.lock();
        // At most one entry per pane in the panel: a new notification for a
        // pane supersedes its previous one, so finishing a task leaves exactly
        // one notification per terminal instead of stacking attention + done.
        history.retain(|e| e.pane != id);
        history.push_front(entry);
        history.truncate(HISTORY_CAP);
        drop(history);
        self.notify_state_changed();
    }

    /// The one funnel for every *automatic* status write (hooks, bell, the
    /// silence heuristic, focus). A pinned status is the user's explicit call,
    /// so all of them no-op on it — only `set_pane_done` moves a pane out.
    ///
    /// Callers must not already hold `pane.status`; `sweep_status` does, and
    /// therefore checks `is_pinned` inline instead of going through here.
    fn set_pane_status(&self, pane: &Pane, new: PaneStatus) {
        let mut status = pane.status.lock();
        if !status.is_pinned() {
            *status = new;
        }
    }

    /// Pin/unpin `done` (finished, under review) on a pane — the toolbar's
    /// check button. Pinning overrides whatever the automatic writers left
    /// there and freezes it; unpinning drops the pane to `idle` and hands it
    /// back to hooks/heuristic, which re-derive from live activity within a
    /// sweep tick. Unpinning a pane that was not pinned does nothing.
    pub fn set_pane_done(&self, id: PaneId, done: bool) -> Result<(), EngineError> {
        let pane = self.pane(id)?;
        {
            let mut status = pane.status.lock();
            // `done` IS the pin, so the pane is already in the requested shape
            // whenever pinned-ness matches the request — nothing to do.
            if status.is_pinned() == done {
                return Ok(());
            }
            *status = if done { PaneStatus::Done } else { PaneStatus::Idle };
        }
        // A pinned pane is settled: drop the in-flight waiting signal so an
        // old one can't resolve into `waiting` the moment the pin lifts.
        *pane.waiting_since.lock() = None;
        self.notify_state_changed();
        Ok(())
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
                ws.active.and_then(|a| ws.map.get(&a)).and_then(|s| s.visible_pane())
            };
            if let Some(pane) = visible {
                self.clear_notification(pane);
                self.mark_pane_seen(pane);
                self.notify_state_changed();
            }
        }
    }

    /// Kept for script compatibility (`amux pane rename`): a pane has no name
    /// of its own, so this renames the tab it lives in.
    pub fn rename_pane(&self, id: PaneId, name: String) -> Result<(), EngineError> {
        self.rename_tab(self.tab_of_pane(id)?, name)
    }

    /// Focus a pane, bringing its tab and workspace on screen with it.
    pub fn focus_pane(&self, id: PaneId) -> Result<(), EngineError> {
        let pane = self.pane(id)?;
        let tab_id = *pane.tab.lock();
        let mut ws = self.workspaces.write();
        let state = ws
            .map
            .get_mut(&pane.workspace)
            .ok_or(EngineError::WorkspaceNotFound(pane.workspace))?;
        state
            .tabs
            .get_mut(&tab_id)
            .ok_or(EngineError::TabNotFound(tab_id))?
            .active_pane = id;
        state.active_tab = tab_id;
        ws.active = Some(pane.workspace);
        drop(ws);
        // Looking at it now — its pending notification is acknowledged.
        *pane.notification.lock() = None;
        self.mark_pane_seen(id);
        self.notify_state_changed();
        Ok(())
    }

    /// Drag-rearrange: detach `pane` from its position and re-insert it as a
    /// split of `target`. `before` puts it left/top. Both panes must live in
    /// the same tab — each tab owns a separate tree, so a cross-tab move would
    /// silently find nothing to remove.
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
        let tab_id = *pane.tab.lock();
        if pane.workspace != target_pane.workspace || tab_id != *target_pane.tab.lock() {
            return Err(EngineError::Other(anyhow::anyhow!(
                "panes are in different tabs"
            )));
        }
        let mut ws = self.workspaces.write();
        let state = ws
            .map
            .get_mut(&pane.workspace)
            .ok_or(EngineError::WorkspaceNotFound(pane.workspace))?
            .tabs
            .get_mut(&tab_id)
            .ok_or(EngineError::TabNotFound(tab_id))?;
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

    /// Move the divider at `path` inside one tab's tree. `tab` is not optional:
    /// the path is only meaningful relative to the tree that owns it.
    pub fn set_ratio(
        &self,
        workspace: WorkspaceId,
        tab: TabId,
        path: &[bool],
        ratio: f32,
    ) -> Result<(), EngineError> {
        let mut ws = self.workspaces.write();
        let state = ws
            .map
            .get_mut(&workspace)
            .ok_or(EngineError::WorkspaceNotFound(workspace))?
            .tabs
            .get_mut(&tab)
            .ok_or(EngineError::TabNotFound(tab))?;
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

    /// Record which Claude Code conversation is running in a pane. Reported by
    /// the hooks (`amux notify --from-claude-hook` reads `session_id` off the
    /// hook payload), and saved with the layout so a restore can hand the pane
    /// back that conversation.
    ///
    /// Deduplicated on purpose: the `UserPromptSubmit` hook fires on every
    /// prompt and would otherwise broadcast a state change per keystroke-turn
    /// for a value that almost never changes.
    pub fn set_claude_session(
        &self,
        id: PaneId,
        session: Option<String>,
    ) -> Result<(), EngineError> {
        let pane = self.pane(id)?;
        let mut slot = pane.claude_session.lock();
        if *slot == session {
            return Ok(());
        }
        *slot = session;
        drop(slot);
        self.notify_state_changed();
        Ok(())
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
                    tabs: s
                        .tabs
                        .iter()
                        .map(|(tid, t)| TabInfo {
                            id: *tid,
                            name: t.name.clone(),
                            layout: t.layout.clone(),
                            active_pane: Some(t.active_pane),
                        })
                        .collect(),
                    active_tab: Some(s.active_tab),
                })
                .collect(),
            panes: panes
                .values()
                .map(|p| PaneInfo {
                    id: p.id,
                    workspace: p.workspace,
                    tab: *p.tab.lock(),
                    meta: p.meta.lock().clone(),
                    notification: p.notification.lock().clone(),
                    status: *p.status.lock(),
                    exited: p.has_exited(),
                    claude_session: p.claude_session.lock().clone(),
                })
                .collect(),
            active_workspace: ws.active,
            notifications: self.history.lock().iter().cloned().collect(),
        }
    }

    // -- session persistence ---------------------------------------------------

    /// The saveable shape of the current state: workspace and tab names, their
    /// order, every split tree with its ratios, and the directory each pane's
    /// shell is sitting in. Written to disk continuously by
    /// `start_session_autosave` so an unexpected exit costs at most a second of
    /// arrangement.
    ///
    /// Carries no ids on purpose (see `crate::session`): they are re-minted on
    /// restore, so keeping them would rewrite the file on every single save.
    pub fn session_snapshot(&self) -> SavedSession {
        let ws = self.workspaces.read();
        let panes = self.panes.read();
        // What a leaf of the saved tree holds: where the shell was, and which
        // Claude conversation was running there.
        let leaf = |id: PaneId| match panes.get(&id) {
            Some(pane) => SavedLayout::Leaf {
                cwd: pane.meta.lock().cwd.clone(),
                claude_session: pane.claude_session.lock().clone(),
            },
            None => SavedLayout::Leaf { cwd: None, claude_session: None },
        };
        SavedSession {
            version: session::FORMAT_VERSION,
            // Stamped by `session::save`; left at 0 here so two snapshots of an
            // unchanged layout compare equal and the autosave can skip the write.
            saved_at_ms: 0,
            workspaces: ws
                .map
                .values()
                .map(|w| SavedWorkspace {
                    name: w.name.clone(),
                    tabs: w
                        .tabs
                        .values()
                        .map(|t| SavedTab {
                            name: t.name.clone(),
                            layout: session::to_saved(&t.layout, &leaf),
                            active_pane: layout::panes(&t.layout)
                                .iter()
                                .position(|p| *p == t.active_pane)
                                .unwrap_or(0),
                        })
                        .collect(),
                    active_tab: w.tabs.get_index_of(&w.active_tab).unwrap_or(0),
                    tab_created_count: w.tab_created_count,
                })
                .collect(),
            active_workspace: ws.active.and_then(|id| ws.map.get_index_of(&id)),
            workspace_created_count: ws.created_count,
        }
    }

    /// Rebuild `saved` as live workspaces, *appending* to whatever is already
    /// open — a restore never disturbs a pane that is already running.
    ///
    /// The panes come back as fresh shells started in the recorded directories;
    /// no process, scrollback or shell history survives a restart, and nothing
    /// pretends otherwise. Returns how many workspaces came back.
    pub fn restore_session(
        self: &Arc<Self>,
        saved: &SavedSession,
        cols: u16,
        rows: u16,
        prefs: &ResumePrefs,
    ) -> Result<usize, EngineError> {
        let mut built: Vec<(WorkspaceId, WorkspaceState)> = Vec::new();
        // Everything spawned so far, so a failure part-way through can undo
        // itself instead of leaking shells nobody can see or close.
        let mut spawned: Vec<PaneId> = Vec::new();
        // Panes whose Claude conversation is to be handed back once their
        // shells are up (see `resume_claude_sessions`).
        let mut resumes: Vec<(PaneId, String)> = Vec::new();

        for sw in &saved.workspaces {
            let ws_id = WorkspaceId::new();
            let mut tabs = IndexMap::new();
            for st in &sw.tabs {
                let tab_id = TabId::new();
                let mut walk = Respawn::default();
                let layout =
                    match self.respawn_layout(&st.layout, ws_id, tab_id, cols, rows, &mut walk) {
                        Ok(node) => node,
                        Err(e) => {
                            spawned.extend(&walk.panes);
                            self.kill_panes(&spawned);
                            return Err(e);
                        }
                    };
                let tab_panes = walk.panes;
                resumes.extend(walk.resumes);
                let Some(&first) = tab_panes.first() else { continue };
                // The saved position addresses the tab's in-order pane list; a
                // file listing a position that no longer exists falls back to
                // the first pane rather than failing the whole restore.
                let active_pane = tab_panes.get(st.active_pane).copied().unwrap_or(first);
                spawned.extend(&tab_panes);
                tabs.insert(tab_id, TabState { name: st.name.clone(), layout, active_pane });
            }
            // A workspace is never empty; one that somehow saved with no tabs
            // is skipped rather than restored into an unusable state.
            if tabs.is_empty() {
                continue;
            }
            let active_index = sw.active_tab.min(tabs.len() - 1);
            let active_tab = *tabs.get_index(active_index).expect("index clamped above").0;
            built.push((
                ws_id,
                WorkspaceState {
                    name: sw.name.clone(),
                    tabs,
                    active_tab,
                    // Verbatim, not `max(tabs.len())`: the counter tracks how
                    // many `탭 N` auto-names were handed out, not how many tabs
                    // exist — hand-named tabs never advance it. Restoring it as
                    // saved makes the next Ctrl+T produce exactly the name it
                    // would have produced had the app never died.
                    tab_created_count: sw.tab_created_count,
                },
            ));
        }

        if built.is_empty() {
            return Ok(0);
        }
        let restored = built.len();
        let ids: Vec<WorkspaceId> = built.iter().map(|(id, _)| *id).collect();
        {
            let mut ws = self.workspaces.write();
            for (id, state) in built {
                ws.map.insert(id, state);
            }
            // Never lower than the saved count, or the next auto-named
            // workspace would collide with a `워크스페이스 N` that just came back.
            ws.created_count = ws.created_count.max(saved.workspace_created_count);
            // Land the user back on the workspace they were last looking at.
            ws.active = saved
                .active_workspace
                .and_then(|i| ids.get(i).copied())
                .or_else(|| ids.last().copied());
        }
        self.notify_state_changed();
        self.resume_claude_sessions(resumes, prefs.clone());
        Ok(restored)
    }

    /// Type `claude --resume <id>` into every restored pane that was running a
    /// Claude conversation, so the agents come back with the layout instead of
    /// leaving the user 17 bare shells to restart by hand.
    ///
    /// Typed into the pane rather than made the pane's own command: a normal
    /// shell stays underneath, so quitting Claude leaves a usable terminal
    /// instead of closing the pane.
    ///
    /// Runs on its own thread because it has to *wait*. A shell that is still
    /// sourcing its rc files will swallow a line typed at it, and there is no
    /// signal for "ready" — the closest honest one is "it has printed
    /// something", which is what `Pane::has_output` reports. Restoring must not
    /// block on that, hence the thread.
    fn resume_claude_sessions(self: &Arc<Self>, resumes: Vec<(PaneId, String)>, prefs: ResumePrefs) {
        if resumes.is_empty() {
            return;
        }
        let engine = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("claude-resume".into())
            .spawn(move || {
                let mut typed: Vec<PaneId> = Vec::new();
                for (id, session) in resumes {
                    let Some(engine) = engine.upgrade() else { return };
                    let Ok(pane) = engine.pane(id) else { continue };
                    // One budget per pane. A single deadline shared by the whole
                    // batch let a slow first shell spend all of it, after which
                    // every remaining pane was typed at before its shell was
                    // listening and swallowed the line.
                    let deadline = std::time::Instant::now() + RESUME_READY_TIMEOUT;
                    while !pane.has_output() && std::time::Instant::now() < deadline {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    // `session` was validated by `claude_transcript_exists`
                    // before it got here, so it cannot carry shell syntax, and
                    // `resume_command` vets the effort level for the same reason.
                    let line = format!("{}\r", prefs.resume_command(&session));
                    let _ = engine.write_pane(id, line.as_bytes());
                    typed.push(id);
                }
                answer_resume_dialogs(&engine, typed, prefs.mode);
            })
            .expect("spawn claude resume");
    }

    /// Walk a saved tree, spawning one shell per leaf, and hand back the live
    /// `LayoutNode` with its ratios intact. `out.panes` collects the new panes
    /// in the same in-order sequence `layout::panes()` produces, which is what
    /// the saved `active_pane` position is an index into.
    fn respawn_layout(
        self: &Arc<Self>,
        node: &SavedLayout,
        workspace: WorkspaceId,
        tab: TabId,
        cols: u16,
        rows: u16,
        out: &mut Respawn,
    ) -> Result<LayoutNode, EngineError> {
        match node {
            SavedLayout::Leaf { cwd, claude_session } => {
                // A directory that has since been deleted or renamed must not
                // sink the restore: drop it and let the shell start at $HOME.
                let cwd = cwd
                    .as_ref()
                    .map(std::path::PathBuf::from)
                    .filter(|path| path.is_dir());
                let pane = self.spawn_pane(workspace, tab, cols, rows, cwd)?;
                out.panes.push(pane.id);
                // Only offer to resume a conversation Claude can still find.
                // A deleted transcript would leave the pane showing an error
                // where the user expected their agent.
                if let Some(session) = claude_session
                    .as_ref()
                    .filter(|id| session::claude_transcript_exists(id))
                {
                    // Carry the id forward immediately rather than waiting for
                    // the resumed session's `SessionStart` hook to report it
                    // back. Without this the pane's conversation is forgotten
                    // the moment it is restored, so a second crash would drop
                    // back to a bare shell — and on a machine whose hooks are
                    // not up to date yet, it would never be remembered at all.
                    *pane.claude_session.lock() = Some(session.clone());
                    out.resumes.push((pane.id, session.clone()));
                }
                Ok(LayoutNode::Leaf { pane: pane.id })
            }
            SavedLayout::Split { axis, ratio, first, second } => Ok(LayoutNode::Split {
                axis: *axis,
                ratio: *ratio,
                first: Box::new(self.respawn_layout(first, workspace, tab, cols, rows, out)?),
                second: Box::new(self.respawn_layout(second, workspace, tab, cols, rows, out)?),
            }),
        }
    }

    /// Kill panes that never made it into a workspace (restore rollback).
    fn kill_panes(&self, ids: &[PaneId]) {
        let mut panes = self.panes.write();
        for id in ids {
            if let Some(pane) = panes.remove(id) {
                pane.kill();
            }
        }
    }

    /// Mirror the session shape to disk once a second, for as long as the app
    /// runs. amux usually dies without getting to say goodbye, so there is no
    /// save-on-exit hook to rely on — the file has to be current at all times.
    ///
    /// A plain polling thread rather than a subscription to `StateChanged`, for
    /// the same reason `start_meta_sweeper` is one: the work is synchronous,
    /// and a poll cannot be starved by a pane that flips status every tick.
    /// Almost every tick costs nothing, because the *shape* only changes when a
    /// workspace, tab, split, name or directory does — typing into a terminal
    /// leaves it identical and the disk stays untouched.
    pub fn start_session_autosave(self: &Arc<Self>) {
        let engine = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("session-autosave".into())
            .spawn(move || {
                let mut armed = false;
                let mut last: Option<SavedSession> = None;
                loop {
                    std::thread::sleep(Duration::from_secs(1));
                    let Some(engine) = engine.upgrade() else { break };
                    if engine.shutting_down.load(Ordering::SeqCst) {
                        break;
                    }
                    let snapshot = engine.session_snapshot();
                    let was_armed = armed;
                    if !session::arm_and_check(&mut armed, &snapshot) {
                        continue;
                    }
                    if !was_armed {
                        // First write of this run: keep one generation of the
                        // file we are about to start replacing.
                        session::keep_previous_generation();
                    }
                    if last.as_ref() == Some(&snapshot) {
                        continue;
                    }
                    match session::save(&snapshot) {
                        Ok(()) => last = Some(snapshot),
                        Err(e) => tracing::warn!("session autosave failed: {e}"),
                    }
                }
            })
            .expect("spawn session autosave");
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

        // The user pinned this pane (done/under review) — the heuristic has no
        // say until they unpin it. Checked inline because we hold the lock that
        // `set_pane_status` would take.
        if old.is_pinned() {
            return false;
        }

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
            // Whitelist, not a blacklist: a pinned `done` is deliberately not
            // listed, so merely looking at a pane never lifts the user's pin.
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
        // Flag first: draining the pane map leaves the workspaces holding
        // layouts full of dangling pane ids, and a session autosave that ran
        // in that gap would save every pane with no directory — overwriting a
        // perfectly good file with a degraded one on the way out the door.
        self.shutting_down.store(true, Ordering::SeqCst);
        for pane in self.panes.write().drain().map(|(_, p)| p) {
            pane.kill();
        }
    }
}

/// Answer Claude Code's summary-or-full menu on the panes a restore just typed
/// `claude --resume` into.
///
/// Runs after every line is typed, not between them, and polls the panes as one
/// group. Waiting on each pane in turn would multiply the timeout by the number
/// of panes, and most restores have panes that never show the menu at all.
///
/// Deliberately timid. It types only when `resume_dialog_step` recognises the
/// cursor on a menu row, it re-reads the screen between moving the cursor and
/// confirming, and when the menu never appears it gives up without touching the
/// pane — which leaves the person exactly the prompt they would have had.
fn answer_resume_dialogs(engine: &Weak<Engine>, panes: Vec<PaneId>, mode: ResumeMode) {
    if mode == ResumeMode::Ask || panes.is_empty() {
        return;
    }
    let mut pending = panes;
    let deadline = std::time::Instant::now() + RESUME_DIALOG_TIMEOUT;
    while !pending.is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(300));
        let Some(engine) = engine.upgrade() else { return };
        pending.retain(|&id| {
            // A pane that has gone away is simply dropped from the watch list.
            let Ok(screen) = engine.read_screen(id) else { return false };
            match session::resume_dialog_step(&screen, mode) {
                // Enter settles the choice; a cursor move does not, so that pane
                // stays on the list for the next round.
                Some(key) => {
                    let _ = engine.write_pane(id, key.bytes());
                    key != ResumeKey::Enter
                }
                None => true,
            }
        });
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// The pin's entire point: `done` belongs to the user, so every *automatic*
    /// status writer must bail on it. Enumerated on purpose — each of these is
    /// a separate code path that has taken the status in the past (hooks/bell
    /// via `notify_pane`, focus via `mark_pane_seen`, silence via
    /// `sweep_status`), and missing any one of them is invisible until a user
    /// watches their DONE pane silently flip back.
    #[test]
    fn pinned_done_survives_every_automatic_writer() {
        let engine = Engine::new();
        let (_ws, _tab, id) = engine.create_workspace(None, None, None, 80, 24).unwrap();
        let pane = engine.pane(id).unwrap();

        engine.set_pane_done(id, true).unwrap();
        assert_eq!(*pane.status.lock(), PaneStatus::Done);

        for kind in [
            NotifyKind::Attention,
            NotifyKind::Bell,
            NotifyKind::Progress,
            NotifyKind::Done,
            NotifyKind::Idle,
        ] {
            engine.notify_pane(id, kind, None, None);
            assert_eq!(*pane.status.lock(), PaneStatus::Done, "{kind:?} overwrote the pin");
        }

        engine.mark_pane_seen(id);
        assert_eq!(*pane.status.lock(), PaneStatus::Done, "focusing overwrote the pin");

        // The heuristic is covered by its own test below: these notifications
        // leave the pane hook-managed, which is exactly the state in which
        // `sweep_status` stands down anyway, so asserting on it here would
        // pass no matter what the pin does.

        // Unpinning hands the pane back to the automatic writers, from `idle`.
        engine.set_pane_done(id, false).unwrap();
        assert_eq!(*pane.status.lock(), PaneStatus::Idle);
        engine.notify_pane(id, NotifyKind::Progress, None, None);
        assert_eq!(*pane.status.lock(), PaneStatus::Processing, "writers stayed locked out");

        engine.shutdown();
    }

    /// The riskiest overwrite path, and the one the test above cannot reach:
    /// a user with no Claude hooks installed pins DONE on a pane where an app
    /// is live and painting. The silence heuristic runs every second and would
    /// call that `processing`.
    ///
    /// Unix-only: it drives a real `sleep 5` and waits for the PTY's foreground
    /// process group leader to differ from the shell — foreground-process-group
    /// semantics that don't map to Windows ConPTY (PowerShell's `sleep` is an
    /// in-process cmdlet, so no distinct foreground PID appears). On Windows the
    /// loop would spin to its deadline and fail, so we skip it there; the two
    /// tests around it are platform-neutral and still cover the pin.
    #[cfg(unix)]
    #[test]
    fn pinned_done_survives_the_silence_heuristic_while_an_app_paints() {
        let engine = Engine::new();
        let (_ws, _tab, id) = engine.create_workspace(None, None, None, 80, 24).unwrap();
        let pane = engine.pane(id).unwrap();

        // The heuristic's live-work branches only engage while a foreground
        // app (not the shell itself) holds the terminal.
        engine.write_pane(id, b"sleep 5\n").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let fg = pane.shell_pid();
            if fg.is_some() && fg != pane.child_pid() {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "`sleep` never took the foreground");
            std::thread::sleep(Duration::from_millis(50));
        }

        engine.set_pane_done(id, true).unwrap();
        assert!(
            !pane.hook_managed.load(Ordering::SeqCst),
            "this test is only meaningful on the hookless path",
        );

        // Fresh work output: unpinned, this pane is squarely `processing`.
        pane.activity.lock().last_output = std::time::Instant::now();
        assert!(!engine.sweep_status(&pane), "the heuristic reported a change on a pinned pane");
        assert_eq!(*pane.status.lock(), PaneStatus::Done, "the heuristic overwrote the pin");

        engine.shutdown();
    }

    /// Each tab owns a separate split tree, so a divider path means nothing
    /// without the tab that owns it. Before tabs, `set_ratio(workspace, path)`
    /// walked the workspace's single tree; the same `path` now addresses a
    /// *different* divider in every tab, and getting this wrong is silent —
    /// no error, the wrong divider just moves. Hence a test.
    #[test]
    fn set_ratio_only_touches_the_tab_it_names() {
        let engine = Engine::new();
        let (ws, tab_a, pane_a) = engine.create_workspace(None, None, None, 80, 24).unwrap();
        engine.split_pane(pane_a, SplitAxis::Horizontal, 80, 24).unwrap();
        let (tab_b, pane_b) = engine.new_tab(ws, None, 80, 24).unwrap();
        engine.split_pane(pane_b, SplitAxis::Horizontal, 80, 24).unwrap();

        let ratio_of = |tab: TabId| -> f32 {
            let snap = engine.snapshot();
            let t = snap.workspaces[0].tabs.iter().find(|t| t.id == tab).expect("tab");
            match &t.layout {
                LayoutNode::Split { ratio, .. } => *ratio,
                LayoutNode::Leaf { .. } => panic!("expected a split"),
            }
        };
        assert_eq!(ratio_of(tab_a), 0.5);
        assert_eq!(ratio_of(tab_b), 0.5);

        // Root divider of tab A only.
        engine.set_ratio(ws, tab_a, &[], 0.8).unwrap();
        assert!((ratio_of(tab_a) - 0.8).abs() < 1e-6, "tab A's divider did not move");
        assert_eq!(ratio_of(tab_b), 0.5, "tab B's divider moved along with A's");

        // A tab id that isn't in this workspace is an error, not a silent no-op.
        assert!(engine.set_ratio(ws, TabId::new(), &[], 0.3).is_err());

        engine.shutdown();
    }

    /// Drag-rearrange re-inserts a pane next to a target. Across tabs that would
    /// find nothing to detach and quietly do nothing (or worse, duplicate the
    /// pane into a second tree), so it has to be refused outright.
    #[test]
    fn move_pane_refuses_to_cross_tabs() {
        let engine = Engine::new();
        let (ws, _tab_a, pane_a) = engine.create_workspace(None, None, None, 80, 24).unwrap();
        let sibling = engine.split_pane(pane_a, SplitAxis::Horizontal, 80, 24).unwrap();
        let (_tab_b, pane_b) = engine.new_tab(ws, None, 80, 24).unwrap();

        assert!(
            engine.move_pane(pane_a, pane_b, SplitAxis::Vertical, false).is_err(),
            "a pane was allowed to move into another tab's tree",
        );
        // The same move inside one tab is fine.
        assert!(engine.move_pane(pane_a, sibling, SplitAxis::Vertical, true).is_ok());

        engine.shutdown();
    }

    /// The close cascade: pane → (empty) tab → (empty) workspace.
    #[test]
    fn closing_the_last_pane_folds_the_tab_then_the_workspace() {
        let engine = Engine::new();
        let (ws, tab_a, pane_a) = engine.create_workspace(None, None, None, 80, 24).unwrap();
        let sibling = engine.split_pane(pane_a, SplitAxis::Horizontal, 80, 24).unwrap();
        let (tab_b, pane_b) = engine.new_tab(ws, None, 80, 24).unwrap();

        let tabs = || engine.snapshot().workspaces.first().map(|w| w.tabs.len()).unwrap_or(0);

        // One of two panes: the tab survives and focus lands on what's left.
        engine.close_pane(sibling).unwrap();
        assert_eq!(tabs(), 2);
        let snap = engine.snapshot();
        let a = snap.workspaces[0].tabs.iter().find(|t| t.id == tab_a).expect("tab A");
        assert_eq!(a.active_pane, Some(pane_a));

        // Last pane of tab A: the tab goes, and tab B takes the screen.
        engine.close_pane(pane_a).unwrap();
        assert_eq!(tabs(), 1);
        assert_eq!(engine.snapshot().workspaces[0].active_tab, Some(tab_b));

        // Last pane of the last tab: the workspace goes with it.
        engine.close_pane(pane_b).unwrap();
        assert!(engine.snapshot().workspaces.is_empty(), "empty workspace outlived its tabs");

        engine.shutdown();
    }

    /// Both toggle directions are idempotent, and unpinning a pane that was
    /// never pinned must not knock a live status back to `idle`.
    #[test]
    fn unpinning_an_unpinned_pane_leaves_its_status_alone() {
        let engine = Engine::new();
        let (_ws, _tab, id) = engine.create_workspace(None, None, None, 80, 24).unwrap();
        let pane = engine.pane(id).unwrap();

        engine.notify_pane(id, NotifyKind::Progress, None, None);
        assert_eq!(*pane.status.lock(), PaneStatus::Processing);

        engine.set_pane_done(id, false).unwrap();
        assert_eq!(*pane.status.lock(), PaneStatus::Processing);

        engine.set_pane_done(id, true).unwrap();
        engine.set_pane_done(id, true).unwrap();
        assert_eq!(*pane.status.lock(), PaneStatus::Done);

        engine.shutdown();
    }

    /// A crash costs the shells; it must not cost the arrangement. Build two
    /// workspaces — the first holding a tab split twice with a hand-dragged
    /// divider — write the shape out as JSON, then restore it into a *fresh*
    /// engine: names, tab order, split geometry, ratios, the active tab and the
    /// focused pane's position all have to come back.
    #[test]
    fn session_round_trip_rebuilds_the_arrangement() {
        let engine = Engine::new();
        let (ws1, tab1, pane1) = engine
            .create_workspace(Some("첫 워크스페이스".into()), Some("작업".into()), None, 80, 24)
            .unwrap();
        // 작업 tab: three panes, in-order [pane1, pane2, pane3].
        let pane2 = engine.split_pane(pane1, SplitAxis::Horizontal, 80, 24).unwrap();
        let _pane3 = engine.split_pane(pane2, SplitAxis::Vertical, 80, 24).unwrap();
        engine.set_ratio(ws1, tab1, &[], 0.7).unwrap();
        engine.new_tab(ws1, Some("로그".into()), 80, 24).unwrap();
        engine
            .create_workspace(Some("두 번째 워크스페이스".into()), Some("빌드".into()), None, 80, 24)
            .unwrap();
        // Leave the user looking at the first workspace, 작업 tab, middle pane.
        engine.focus_tab(tab1).unwrap();
        engine.focus_pane(pane2).unwrap();

        // Through JSON, exactly as the autosave writes it and a restore reads it.
        let json = serde_json::to_string(&engine.session_snapshot()).unwrap();
        let saved: SavedSession = serde_json::from_str(&json).unwrap();
        engine.shutdown();

        let restored_engine = Engine::new();
        assert_eq!(restored_engine.restore_session(&saved, 80, 24, &ResumePrefs::default()).unwrap(), 2);
        let snap = restored_engine.snapshot();

        assert_eq!(
            snap.workspaces.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(),
            ["첫 워크스페이스", "두 번째 워크스페이스"],
        );
        assert_eq!(snap.active_workspace, Some(snap.workspaces[0].id), "landed on the wrong workspace");

        let first = &snap.workspaces[0];
        assert_eq!(
            first.tabs.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["작업", "로그"],
            "tab order or names drifted",
        );
        assert_eq!(first.active_tab, Some(first.tabs[0].id));

        // The split tree: a 0.7 divider with a nested split on its second side.
        let work = &first.tabs[0];
        let panes = layout::panes(&work.layout);
        assert_eq!(panes.len(), 3);
        match &work.layout {
            LayoutNode::Split { ratio, second, .. } => {
                assert!((ratio - 0.7).abs() < 1e-6, "dragged ratio lost: {ratio}");
                assert!(matches!(**second, LayoutNode::Split { .. }), "nested split lost");
            }
            _ => panic!("expected a split at the root of 작업"),
        }
        // Focus follows the *position* the pane held, since its id is long dead.
        assert_eq!(work.active_pane, Some(panes[1]), "focus landed on the wrong pane");

        restored_engine.shutdown();
    }

    /// Restoring appends: whatever is already open keeps running untouched.
    #[test]
    fn restore_appends_and_never_replaces() {
        let engine = Engine::new();
        let (_ws, _tab, live) = engine
            .create_workspace(Some("이미 열려 있던 것".into()), None, None, 80, 24)
            .unwrap();

        let saved = SavedSession {
            version: session::FORMAT_VERSION,
            saved_at_ms: 0,
            workspaces: vec![SavedWorkspace {
                name: "복구된 것".into(),
                tabs: vec![SavedTab {
                    name: "탭 1".into(),
                    layout: SavedLayout::Leaf { cwd: None, claude_session: None },
                    active_pane: 0,
                }],
                active_tab: 0,
                tab_created_count: 1,
            }],
            active_workspace: Some(0),
            workspace_created_count: 1,
        };
        assert_eq!(engine.restore_session(&saved, 80, 24, &ResumePrefs::default()).unwrap(), 1);

        let snap = engine.snapshot();
        assert_eq!(
            snap.workspaces.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(),
            ["이미 열려 있던 것", "복구된 것"],
        );
        assert!(engine.pane(live).is_ok(), "restore killed a pane that was already running");
        engine.shutdown();
    }

    /// The record → save half of "the Claude conversation comes back too": what
    /// the hooks report has to survive into the file, at the right leaf.
    ///
    /// The restore half is deliberately split: a transcript that exists would
    /// make the engine type `claude --resume` into a real shell, launching a
    /// real agent inside `cargo test`. So the guard is tested here from the
    /// other side (an unknown conversation must be dropped, quietly), the
    /// guard itself in `tests/claude_resume.rs`, and the actual resume by
    /// running it in the app.
    #[test]
    fn a_reported_claude_session_reaches_the_saved_leaf() {
        let engine = Engine::new();
        let (_ws, _tab, left) = engine
            .create_workspace(Some("연구".into()), Some("에이전트".into()), None, 80, 24)
            .unwrap();
        let right = engine.split_pane(left, SplitAxis::Horizontal, 80, 24).unwrap();

        let session = "aaa130ca-cf95-441c-9812-a6587b6dfced";
        engine.set_claude_session(left, Some(session.into())).unwrap();

        // Only the pane that reported one carries it; its neighbour stays bare.
        let saved = engine.session_snapshot();
        match &saved.workspaces[0].tabs[0].layout {
            SavedLayout::Split { first, second, .. } => {
                assert!(
                    matches!(&**first, SavedLayout::Leaf { claude_session: Some(s), .. } if s == session),
                    "the reported conversation did not reach the leaf: {first:?}",
                );
                assert!(matches!(&**second, SavedLayout::Leaf { claude_session: None, .. }));
            }
            other => panic!("expected a split, got {other:?}"),
        }

        // The engine also exposes it live, which is how the sidebar and the
        // capture script see it.
        let snapshot = engine.snapshot();
        let pane = snapshot.panes.iter().find(|p| p.id == left).unwrap();
        assert_eq!(pane.claude_session.as_deref(), Some(session));
        assert_eq!(
            snapshot.panes.iter().find(|p| p.id == right).unwrap().claude_session,
            None,
        );
        engine.shutdown();
    }

    /// A conversation Claude can no longer find must be dropped silently. If it
    /// were not, the restored pane would greet the user with a resume error
    /// where they expected their agent — and, worse, the same rejection is what
    /// keeps anything that is not a plain id off a shell command line.
    #[test]
    fn a_vanished_claude_session_is_not_carried_into_a_restore() {
        let engine = Engine::new();
        let saved = SavedSession {
            version: session::FORMAT_VERSION,
            saved_at_ms: 0,
            workspaces: vec![SavedWorkspace {
                name: "복구".into(),
                tabs: vec![SavedTab {
                    name: "탭 1".into(),
                    layout: SavedLayout::Leaf {
                        cwd: None,
                        // No such transcript, so nothing may be typed.
                        claude_session: Some("00000000-0000-4000-8000-000000000000".into()),
                    },
                    active_pane: 0,
                }],
                active_tab: 0,
                tab_created_count: 0,
            }],
            active_workspace: Some(0),
            workspace_created_count: 0,
        };
        assert_eq!(engine.restore_session(&saved, 80, 24, &ResumePrefs::default()).unwrap(), 1);

        let snapshot = engine.snapshot();
        assert_eq!(snapshot.panes.len(), 1);
        assert_eq!(
            snapshot.panes[0].claude_session, None,
            "a conversation that no longer exists was kept, and would be typed at a shell",
        );
        engine.shutdown();
    }
}
