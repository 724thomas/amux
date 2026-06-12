use std::sync::Arc;

use cmux_core::{Engine, EngineEvent};
use tauri::{Emitter, Manager};

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
        .plugin(tauri_plugin_opener::init())
        .manage(Arc::clone(&engine))
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::create_pane,
            commands::write_pane,
            commands::resize_pane,
            commands::close_pane,
            commands::pane_subscribe,
        ])
        .setup(move |app| {
            // Forward engine state changes to the webview as fresh snapshots.
            let handle = app.handle().clone();
            let engine = Arc::clone(&engine);
            tauri::async_runtime::spawn(async move {
                let mut events = engine.subscribe();
                while let Ok(event) = events.recv().await {
                    match event {
                        EngineEvent::StateChanged => {
                            let _ = handle.emit("state:snapshot", engine.snapshot());
                        }
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                window.state::<Arc<Engine>>().shutdown();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
