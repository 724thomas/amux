use std::sync::Arc;

use amux_core::{Engine, EngineEvent};
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
            commands::new_tab,
            commands::close_tab,
            commands::focus_tab,
            commands::rename_tab,
            commands::move_tab,
            commands::split_pane,
            commands::focus_pane,
            commands::rename_pane,
            commands::set_pane_done,
            commands::move_pane,
            commands::clear_notification_history,
            commands::write_pane,
            commands::resize_pane,
            commands::close_pane,
            commands::pane_subscribe,
        ])
        .setup(move |app| {
            engine.start_meta_sweeper();
            // Launch with zero workspaces — the user opens the first one via
            // "+ 새 워크스페이스" (which prompts for a title). We intentionally
            // do NOT auto-create a workspace here; the frontend renders an empty
            // main area and the sidebar's add button is the entry point.
            // Automation socket: same engine the UI uses.
            tauri::async_runtime::spawn({
                let engine = Arc::clone(&engine);
                async move {
                    if let Err(e) = amux_core::server::run(engine).await {
                        tracing::error!("socket server: {e}");
                    }
                }
            });
            // Forward engine state changes to the webview as fresh snapshots.
            let handle = app.handle().clone();
            let engine = Arc::clone(&engine);
            tauri::async_runtime::spawn(async move {
                let mut events = engine.subscribe();
                // Dock icon badge = processed (finished, not yet seen) count.
                // Rendered by Ubuntu Dock via the Unity LauncherEntry D-Bus
                // API; harmless no-op on desktops without a dock.
                let mut badge = i64::MIN; // sentinel: always set on first event
                while let Ok(event) = events.recv().await {
                    match event {
                        EngineEvent::StateChanged => {
                            let snapshot = engine.snapshot();
                            let processed = snapshot
                                .panes
                                .iter()
                                .filter(|p| p.status == amux_protocol::PaneStatus::Processed)
                                .count() as i64;
                            if processed != badge {
                                badge = processed;
                                if let Some(window) = handle.webview_windows().values().next() {
                                    let _ = window
                                        .set_badge_count((processed > 0).then_some(processed));
                                }
                            }
                            let _ = handle.emit("state:snapshot", snapshot);
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
