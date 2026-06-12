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
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(Arc::clone(&engine))
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::create_workspace,
            commands::close_workspace,
            commands::focus_workspace,
            commands::rename_workspace,
            commands::move_workspace,
            commands::set_ratio,
            commands::split_pane,
            commands::focus_pane,
            commands::rename_pane,
            commands::write_pane,
            commands::resize_pane,
            commands::close_pane,
            commands::pane_subscribe,
        ])
        .setup(move |app| {
            engine.start_meta_sweeper();
            // Initial workspace lives engine-side so webview reloads can't
            // race a duplicate into existence. The first resize fixes 80x24.
            if let Err(e) = engine.create_workspace(None, None, 80, 24) {
                tracing::error!("initial workspace: {e}");
            }
            // Automation socket: same engine the UI uses.
            tauri::async_runtime::spawn({
                let engine = Arc::clone(&engine);
                async move {
                    if let Err(e) = cmux_core::server::run(engine).await {
                        tracing::error!("socket server: {e}");
                    }
                }
            });
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
                        EngineEvent::PaneRing(pane) => {
                            let _ = handle.emit("notify:ring", pane);
                        }
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::Destroyed => {
                window.state::<Arc<Engine>>().shutdown();
            }
            tauri::WindowEvent::Focused(focused) => {
                window.state::<Arc<Engine>>().set_window_focused(*focused);
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
