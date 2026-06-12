use std::sync::Arc;

use cmux_core::Engine;
use cmux_protocol::Snapshot;
use tauri::State;

#[tauri::command]
pub fn get_snapshot(engine: State<'_, Arc<Engine>>) -> Snapshot {
    engine.snapshot()
}
