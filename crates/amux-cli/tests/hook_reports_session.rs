//! The half of "restore brings Claude back" that happens *before* a crash:
//! a Claude Code hook telling amux which conversation a pane is running.
//!
//! Worth an end-to-end test rather than a unit one, because the value crosses
//! three boundaries on its way in and every one of them has its own way of
//! silently dropping it: Claude pipes JSON to the hook's **stdin** (not argv),
//! the hook runs the real `amux` binary which must find its pane from the
//! `AMUX_PANE_ID` environment variable, and the id then travels over the local
//! socket as a JSON-RPC call. A unit test of any single hop would pass while
//! the chain stayed broken, and the breakage is invisible until a crash, when
//! the panes come back as bare shells.
//!
//! So this starts a real engine and socket server, runs the real binary the
//! hooks run, and then asks the engine what it ended up believing.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

const SESSION: &str = "aaa130ca-cf95-441c-9812-a6587b6dfced";

#[tokio::test]
async fn a_hook_payload_teaches_the_engine_which_conversation_a_pane_runs() {
    let socket = std::env::temp_dir()
        .join(format!("amux-hooktest-{}.sock", std::process::id()))
        .to_string_lossy()
        .into_owned();
    std::env::set_var(amux_protocol::env_keys::SOCKET, &socket);

    let engine = amux_core::Engine::new();
    tokio::spawn(amux_core::server::run(std::sync::Arc::clone(&engine)));
    let (_ws, _tab, pane) = engine
        .create_workspace(Some("연구".into()), Some("에이전트".into()), None, 80, 24)
        .unwrap();

    // Nothing has spoken up yet, so the engine knows of no conversation.
    assert_eq!(engine.snapshot().panes[0].claude_session, None);

    // Exactly what Claude Code does at `SessionStart`: run the hook command
    // with the pane's environment, and pipe it the event as JSON on stdin.
    let payload =
        format!(r#"{{"session_id":"{SESSION}","hook_event_name":"SessionStart","cwd":"/tmp"}}"#);
    let ran = tokio::task::spawn_blocking(move || {
        for _ in 0..50 {
            let mut child = Command::new(env!("CARGO_BIN_EXE_amux"))
                .args(["notify", "--kind", "idle", "--from-claude-hook"])
                .env(amux_protocol::env_keys::PANE_ID, pane.to_string())
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("run the amux binary the hooks run");
            child.stdin.take().unwrap().write_all(payload.as_bytes()).unwrap();
            if child.wait().unwrap().success() {
                return true;
            }
            // The server may not have bound the socket yet.
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    })
    .await
    .unwrap();
    assert!(ran, "the hook command never succeeded against the test server");

    assert_eq!(
        engine.snapshot().panes[0].claude_session.as_deref(),
        Some(SESSION),
        "the hook ran but the engine never learned the conversation",
    );

    // And the id has to reach the *saved* shape, since that is what a restore
    // reads back. Anything less and the chain still ends at a bare shell.
    let saved = engine.session_snapshot();
    assert!(
        format!("{:?}", saved.workspaces[0].tabs[0].layout).contains(SESSION),
        "the conversation never reached the saved layout: {:?}",
        saved.workspaces[0].tabs[0].layout,
    );

    engine.shutdown();
    let _ = std::fs::remove_file(&socket);
}
