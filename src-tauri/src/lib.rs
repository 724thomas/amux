use std::sync::Arc;

use cmux_core::Engine;

mod commands;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let engine: Arc<Engine> = Engine::new();

    tauri::Builder::default()
        .manage(engine)
        .invoke_handler(tauri::generate_handler![commands::get_snapshot])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
