use std::sync::Arc;

use amux_core::{Engine, EngineEvent};
use tauri::{Emitter, Manager};

mod chat;
mod commands;
mod handoff;
mod popout;
mod subagents;
mod workdir;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let engine: Arc<Engine> = Engine::new();
    // Read the previous session BEFORE anything can touch the file, and hold it
    // in memory: the autosave below starts writing as soon as the user opens a
    // workspace, and the restore offer has to survive that.
    let previous = commands::PreviousSession::new(amux_core::session::load());
    // Structured Claude chats (`claude -p` stream-json children), one per pane.
    let chats = Arc::new(chat::Chats::default());

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(Arc::clone(&engine))
        .manage(previous)
        .manage(commands::History::default())
        .manage(Arc::clone(&chats))
        .manage(popout::Popout::default())
        .manage(subagents::Subagents::default())
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::saved_session,
            commands::restore_session,
            commands::search_history,
            commands::history_conversation,
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
            chat::chat_start,
            chat::chat_write,
            chat::chat_stop,
            chat::chat_running,
            workdir::work_root,
            workdir::work_list,
            workdir::work_status,
            workdir::work_read,
            workdir::work_head,
            popout::files_pop,
            popout::files_state,
            popout::files_look,
            subagents::claude_links,
            handoff::term_claude_link,
            handoff::term_claude_end,
            handoff::session_history,
            chat::chat_handback,
            subagents::subagents_list,
            subagents::subagent_read,
            subagents::session_prompts,
        ])
        .setup(move |app| {
            engine.start_meta_sweeper();
            // Keep ~/.config/amux/session.json current so an unexpected exit
            // (a crash, an OOM kill) costs at most a second of arrangement.
            // The saver holds its fire until it has seen a non-empty state, so
            // the empty launch below cannot erase what it is meant to protect.
            engine.start_session_autosave();
            // Ask Anthropic for the Claude subscription usage every five minutes
            // and share it with every Claude status line through one file, so
            // all sessions show the same current numbers instead of whatever
            // each one saw on its own last reply.
            amux_core::usage::start_poller();
            // Launch with zero workspaces — the user opens the first one via
            // "+ 새 워크스페이스" (which prompts for a title), or restores the
            // previous session in one click from the card the frontend shows
            // over the empty main area. We intentionally do NOT auto-create a
            // workspace here, and do NOT auto-restore: bringing a dozen shells
            // back to life is the user's call to make, not a launch side effect.
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
            let chats = Arc::clone(&chats);
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
                                if let Some(window) = handle.get_webview_window(popout::MAIN) {
                                    let _ = window
                                        .set_badge_count((processed > 0).then_some(processed));
                                }
                            }
                            // A chat dies with its pane (closed pane, tab or
                            // workspace) instead of lingering as an orphan.
                            let alive: std::collections::HashSet<_> =
                                snapshot.panes.iter().map(|p| p.id).collect();
                            chats.retain_panes(&alive);
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
            // Only the main window's end is the app's end; the files pop-out
            // window closing just puts its panels back (see popout.rs).
            tauri::WindowEvent::Destroyed => match popout::on_destroyed(window.label()) {
                popout::OnDestroyed::ShutDown => {
                    // The pop-out cannot outlive the main window.
                    if let Some(files) = window.app_handle().get_webview_window(popout::FILES) {
                        let _ = files.destroy();
                    }
                    window.state::<Arc<chat::Chats>>().stop_all();
                    window.state::<Arc<Engine>>().shutdown();
                }
                popout::OnDestroyed::PanelsBack => {
                    let state = window.state::<popout::Popout>().clear_panes();
                    popout::broadcast(window.app_handle(), &state);
                }
                popout::OnDestroyed::Nothing => {}
            },
            // The engine stays quiet about panes the user is looking at; only
            // the main window shows panes.
            tauri::WindowEvent::Focused(focused) if popout::tracks_focus(window.label()) => {
                window.state::<Arc<Engine>>().set_window_focused(*focused);
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
