//! The autosave loop, driven for real against a temp file.
//!
//! This lives in `tests/` (its own process) rather than beside the unit tests
//! because it sets `AMUX_SESSION_FILE`, and an environment variable is global
//! to a process — inside `cargo test`'s shared test binary it would leak into
//! whatever else happened to be running on another thread.
//!
//! The scenario it walks is the one the feature exists for: a user with work
//! open, an amux that dies without warning, and a relaunch that must not
//! destroy what it is about to offer back.

use std::sync::Arc;
use std::time::Duration;

use amux_core::session;
use amux_core::Engine;
use amux_protocol::SplitAxis;

/// Two autosave ticks (the loop polls once a second), with slack for a slow
/// machine.
const SETTLE: Duration = Duration::from_millis(2500);

#[test]
fn autosave_survives_a_crash_and_an_empty_relaunch() {
    let path = std::env::temp_dir().join(format!("amux-session-test-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&path);
    std::env::set_var(session::ENV_SESSION_FILE, &path);

    // -- the run that dies ---------------------------------------------------
    let engine = Engine::new();
    engine.start_session_autosave();
    let (workspace, tab, pane) = engine
        .create_workspace(Some("연구".into()), Some("에이전트".into()), None, 80, 24)
        .unwrap();
    engine.split_pane(pane, SplitAxis::Vertical, 80, 24).unwrap();
    engine.set_ratio(workspace, tab, &[], 0.35).unwrap();
    engine.new_tab(workspace, Some("문서".into()), 80, 24).unwrap();
    std::thread::sleep(SETTLE);

    let saved = session::load().expect("the arrangement should be on disk by now");
    assert_eq!(saved.summary().workspaces, 1);
    assert_eq!(saved.summary().tabs, 2);
    assert_eq!(saved.summary().panes, 3);
    assert!(saved.saved_at_ms > 0, "the write should be stamped with a time");
    assert_eq!(saved.workspaces[0].name, "연구");
    assert_eq!(
        saved.workspaces[0].tabs.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        ["에이전트", "문서"],
    );

    // The app goes away. `shutdown` stands in for the kill: it drains the pane
    // map, which is exactly the half-torn-down state the autosave must not
    // write out (every pane would look like it had no directory).
    let bytes_at_death = std::fs::read(&path).unwrap();
    engine.shutdown();
    drop(engine);
    std::thread::sleep(SETTLE);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes_at_death,
        "shutting down rewrote the session file with a degraded snapshot",
    );

    // -- the relaunch --------------------------------------------------------
    // A fresh amux comes up with ZERO workspaces on purpose. Its autosave is
    // already running, and this is the moment the whole feature could eat
    // itself: one write here and the file the user is about to restore from
    // says "nothing was open".
    let relaunched: Arc<Engine> = Engine::new();
    relaunched.start_session_autosave();
    std::thread::sleep(SETTLE);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes_at_death,
        "an empty launch overwrote the saved session",
    );

    // -- the restore ---------------------------------------------------------
    let offer = session::load().expect("still on offer after the empty launch");
    assert_eq!(relaunched.restore_session(&offer, 80, 24, &session::ResumePrefs::default()).unwrap(), 1);
    let snapshot = relaunched.snapshot();
    assert_eq!(snapshot.workspaces.len(), 1);
    assert_eq!(snapshot.workspaces[0].name, "연구");
    assert_eq!(
        snapshot.workspaces[0].tabs.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        ["에이전트", "문서"],
    );
    assert_eq!(snapshot.panes.len(), 3);

    // Now that panes are live again, the autosave arms and keeps mirroring —
    // the restored arrangement is itself protected from the next crash.
    std::thread::sleep(SETTLE);
    let after_restore = session::load().expect("the restored arrangement is saved in turn");
    assert_eq!(after_restore.summary().panes, 3);

    relaunched.shutdown();
    let _ = std::fs::remove_file(&path);
}
