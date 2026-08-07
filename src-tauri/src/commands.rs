use std::sync::Arc;

use amux_core::Engine;
use amux_protocol::{PaneId, Snapshot, SplitAxis, TabId, WorkspaceId};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::State;

type Eng<'a> = State<'a, Arc<Engine>>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[tauri::command]
pub fn get_snapshot(engine: Eng<'_>) -> Snapshot {
    engine.snapshot()
}

// -- workspaces --------------------------------------------------------------

#[tauri::command]
pub fn create_workspace(
    engine: Eng<'_>,
    name: Option<String>,
    tab_name: Option<String>,
    cols: u16,
    rows: u16,
) -> Result<WorkspaceId, String> {
    engine
        .create_workspace(name, tab_name, None, cols, rows)
        .map(|(ws, _tab, _pane)| ws)
        .map_err(err)
}

#[tauri::command]
pub fn close_workspace(engine: Eng<'_>, workspace: WorkspaceId) -> Result<(), String> {
    engine.close_workspace(workspace).map_err(err)
}

#[tauri::command]
pub fn focus_workspace(engine: Eng<'_>, workspace: WorkspaceId) -> Result<(), String> {
    engine.focus_workspace(workspace).map_err(err)
}

#[tauri::command]
pub fn rename_workspace(
    engine: Eng<'_>,
    workspace: WorkspaceId,
    name: String,
) -> Result<(), String> {
    engine.rename_workspace(workspace, name).map_err(err)
}

#[tauri::command]
pub fn move_workspace(
    engine: Eng<'_>,
    workspace: WorkspaceId,
    index: usize,
) -> Result<(), String> {
    engine.move_workspace(workspace, index).map_err(err)
}

/// `path` walks the split tree of one *tab*, so the tab has to come along —
/// the same path means a different divider in a different tab.
#[tauri::command]
pub fn set_ratio(
    engine: Eng<'_>,
    workspace: WorkspaceId,
    tab: TabId,
    path: Vec<bool>,
    ratio: f32,
) -> Result<(), String> {
    engine.set_ratio(workspace, tab, &path, ratio).map_err(err)
}

// -- tabs ---------------------------------------------------------------------

#[tauri::command]
pub fn new_tab(
    engine: Eng<'_>,
    workspace: WorkspaceId,
    name: Option<String>,
    cols: u16,
    rows: u16,
) -> Result<TabId, String> {
    engine
        .new_tab(workspace, name, cols, rows)
        .map(|(tab, _pane)| tab)
        .map_err(err)
}

#[tauri::command]
pub fn close_tab(engine: Eng<'_>, tab: TabId) -> Result<(), String> {
    engine.close_tab(tab).map_err(err)
}

#[tauri::command]
pub fn focus_tab(engine: Eng<'_>, tab: TabId) -> Result<(), String> {
    engine.focus_tab(tab).map_err(err)
}

#[tauri::command]
pub fn rename_tab(engine: Eng<'_>, tab: TabId, name: String) -> Result<(), String> {
    engine.rename_tab(tab, name).map_err(err)
}

#[tauri::command]
pub fn move_tab(engine: Eng<'_>, tab: TabId, index: usize) -> Result<(), String> {
    engine.move_tab(tab, index).map_err(err)
}

// -- panes --------------------------------------------------------------------

#[tauri::command]
pub fn split_pane(
    engine: Eng<'_>,
    pane: PaneId,
    axis: SplitAxis,
    cols: u16,
    rows: u16,
) -> Result<PaneId, String> {
    engine.split_pane(pane, axis, cols, rows).map_err(err)
}

#[tauri::command]
pub fn focus_pane(engine: Eng<'_>, pane: PaneId) -> Result<(), String> {
    engine.focus_pane(pane).map_err(err)
}

#[tauri::command]
pub fn rename_pane(engine: Eng<'_>, pane: PaneId, name: String) -> Result<(), String> {
    engine.rename_pane(pane, name).map_err(err)
}

/// Pin/unpin the `done` (under review) status — the pane toolbar's check button.
#[tauri::command]
pub fn set_pane_done(engine: Eng<'_>, pane: PaneId, done: bool) -> Result<(), String> {
    engine.set_pane_done(pane, done).map_err(err)
}

#[tauri::command]
pub fn move_pane(
    engine: Eng<'_>,
    pane: PaneId,
    target: PaneId,
    axis: SplitAxis,
    before: bool,
) -> Result<(), String> {
    engine.move_pane(pane, target, axis, before).map_err(err)
}

#[tauri::command]
pub fn clear_notification_history(engine: Eng<'_>) {
    engine.clear_notification_history()
}

#[tauri::command]
pub fn write_pane(engine: Eng<'_>, pane: PaneId, data: String) -> Result<(), String> {
    engine.write_pane(pane, data.as_bytes()).map_err(err)
}

#[tauri::command]
pub fn resize_pane(engine: Eng<'_>, pane: PaneId, cols: u16, rows: u16) -> Result<(), String> {
    engine.resize_pane(pane, cols, rows).map_err(err)
}

#[tauri::command]
pub fn close_pane(engine: Eng<'_>, pane: PaneId) -> Result<(), String> {
    engine.close_pane(pane).map_err(err)
}

/// Stream a pane's raw output bytes to the webview. The channel carries raw
/// binary frames (no JSON, no base64) — they arrive as ArrayBuffer in JS.
#[tauri::command]
pub fn pane_subscribe(
    engine: Eng<'_>,
    pane: PaneId,
    channel: Channel<InvokeResponseBody>,
) -> Result<(), String> {
    engine
        .subscribe_pane(
            pane,
            Box::new(move |chunk: &[u8]| {
                let _ = channel.send(InvokeResponseBody::Raw(chunk.to_vec()));
            }),
        )
        .map_err(err)
}
