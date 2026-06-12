use std::sync::Arc;

use cmux_core::Engine;
use cmux_protocol::{PaneId, Snapshot};
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

#[tauri::command]
pub fn create_pane(engine: Eng<'_>, cols: u16, rows: u16) -> Result<PaneId, String> {
    engine.create_pane(cols, rows, None).map_err(err)
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
