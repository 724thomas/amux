//! Session persistence: the *shape* of the user's work, mirrored to disk.
//!
//! amux can go away without warning (a crash, an OOM kill, a stray SIGKILL).
//! What hurts then is not the lost shells — those come back with one command —
//! but the lost arrangement: which workspaces existed, what the user named
//! them, which tabs sat inside each one and how every tab was split. This
//! module writes that arrangement to `~/.config/amux/session.json`
//! continuously, so the next launch can rebuild it in a single click.
//!
//! What is deliberately NOT saved, because it cannot be: running processes,
//! terminal scrollback, shell history. A restored pane is a *fresh* shell,
//! started in the directory the old one was sitting in.
//!
//! The file stores no IDs. Every `PaneId`/`TabId`/`WorkspaceId` is re-minted on
//! restore anyway (the old ones name dead PTYs), so keeping them would only
//! churn the file on every save. Positions do the addressing instead: the
//! active tab is an index into `tabs`, and the active pane is a position in
//! the tab's in-order pane list — the same order `layout::panes()` produces
//! and the frontend's `layoutPanes()` mirrors.

use std::io::Write;
use std::path::PathBuf;

use amux_protocol::{LayoutNode, PaneId, SplitAxis};
use serde::{Deserialize, Serialize};

/// Bumped when the on-disk shape changes incompatibly. A file carrying a
/// version this build doesn't understand is left alone rather than restored.
pub const FORMAT_VERSION: u32 = 1;

/// Overrides the session file path. Set it when running a second amux (a dev
/// build alongside the real one) so the dev instance's autosave cannot
/// overwrite the session the user actually cares about.
pub const ENV_SESSION_FILE: &str = "AMUX_SESSION_FILE";

// ---------------------------------------------------------------------------
// On-disk shape
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedSession {
    pub version: u32,
    /// Unix epoch milliseconds of the last write. Stamped by `save`, so
    /// comparing two in-memory sessions ignores it (both carry 0) and an
    /// unchanged layout never triggers a pointless disk write.
    #[serde(default)]
    pub saved_at_ms: u64,
    /// Sidebar order.
    pub workspaces: Vec<SavedWorkspace>,
    /// Index into `workspaces` of the one that was on screen.
    #[serde(default)]
    pub active_workspace: Option<usize>,
    /// How many workspace auto-names (`워크스페이스 N`) had been handed out.
    /// Restored so the next auto-named workspace doesn't collide with one that
    /// just came back.
    #[serde(default)]
    pub workspace_created_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedWorkspace {
    pub name: String,
    /// Tab-bar order.
    pub tabs: Vec<SavedTab>,
    /// Index into `tabs` of the tab that was on screen.
    #[serde(default)]
    pub active_tab: usize,
    /// Per-workspace count behind the `탭 N` auto-name, restored for the same
    /// reason as `workspace_created_count`.
    #[serde(default)]
    pub tab_created_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedTab {
    pub name: String,
    pub layout: SavedLayout,
    /// Position of the focused pane in the tab's in-order pane list.
    #[serde(default)]
    pub active_pane: usize,
}

/// The split tree with directories where the pane IDs used to be.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SavedLayout {
    Leaf {
        /// Where that pane's shell was working, so the restored one starts
        /// there instead of at `$HOME`.
        #[serde(default)]
        cwd: Option<String>,
        /// The Claude Code conversation the pane was running, if the hooks
        /// reported one. A restore types `claude --resume <id>` into the pane
        /// so the conversation comes back with the layout. Absent on files
        /// written before this field existed, hence `serde(default)`.
        #[serde(default)]
        claude_session: Option<String>,
    },
    Split {
        axis: SplitAxis,
        ratio: f32,
        first: Box<SavedLayout>,
        second: Box<SavedLayout>,
    },
}

/// What the restore offer shows the user before they commit to it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub workspaces: usize,
    pub tabs: usize,
    pub panes: usize,
    pub saved_at_ms: u64,
}

impl SavedSession {
    pub fn is_empty(&self) -> bool {
        self.workspaces.is_empty()
    }

    pub fn summary(&self) -> SessionSummary {
        let tabs = self.workspaces.iter().map(|w| w.tabs.len()).sum();
        let panes = self
            .workspaces
            .iter()
            .flat_map(|w| w.tabs.iter())
            .map(|t| t.layout.pane_count())
            .sum();
        SessionSummary {
            workspaces: self.workspaces.len(),
            tabs,
            panes,
            saved_at_ms: self.saved_at_ms,
        }
    }
}

impl SavedLayout {
    pub fn pane_count(&self) -> usize {
        match self {
            SavedLayout::Leaf { .. } => 1,
            SavedLayout::Split { first, second, .. } => first.pane_count() + second.pane_count(),
        }
    }
}

/// Convert a live split tree into its saveable form. `leaf` decides what gets
/// recorded for each pane (its directory and Claude conversation), so this
/// function never has to know how to reach a live `Pane`.
pub fn to_saved(node: &LayoutNode, leaf: &dyn Fn(PaneId) -> SavedLayout) -> SavedLayout {
    match node {
        LayoutNode::Leaf { pane } => leaf(*pane),
        LayoutNode::Split { axis, ratio, first, second } => SavedLayout::Split {
            axis: *axis,
            ratio: *ratio,
            first: Box::new(to_saved(first, leaf)),
            second: Box::new(to_saved(second, leaf)),
        },
    }
}

/// Does Claude Code still hold the transcript for this conversation?
///
/// Found by scanning `~/.claude/projects/*/` rather than by computing the
/// directory name. Claude encodes a project's path into that name lossily —
/// `/home/me/proj` becomes `-home-me-proj`, and spaces and dots collapse into
/// dashes too — so rebuilding it from a directory is guesswork. A session id
/// is a UUID and unique across projects, so scanning is unambiguous.
///
/// Doubles as the gate on what may be typed into a pane: the id ends up on a
/// shell command line, so anything but a plain id is refused outright.
pub fn claude_transcript_exists(session_id: &str) -> bool {
    if session_id.is_empty()
        || !session_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return false;
    }
    let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) else {
        return false;
    };
    let projects = PathBuf::from(home).join(".claude").join("projects");
    let Ok(entries) = std::fs::read_dir(projects) else { return false };
    entries
        .filter_map(Result::ok)
        .any(|entry| entry.path().join(format!("{session_id}.jsonl")).is_file())
}

// ---------------------------------------------------------------------------
// What a restore does to the conversations it brings back
// ---------------------------------------------------------------------------

/// How to answer Claude Code's "resume from a summary, or the full session?"
/// prompt on every pane a restore brings back.
///
/// Claude Code has no flag for this — the choice only exists as a menu drawn in
/// the terminal — so `Full` and `Summary` are carried out by watching the pane
/// and typing at the menu. `Ask` is what amux did before this existed and stays
/// the default: the other two spend usage limits on the user's behalf, which is
/// exactly what that prompt is there to warn about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResumeMode {
    #[default]
    Ask,
    Full,
    Summary,
}

/// One set of choices applied to every Claude pane in a restore, made once on
/// the restore card rather than per pane.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResumePrefs {
    /// Level for `claude --effort`, or `None` to leave it to the user's
    /// `~/.claude/settings.json`. The flag outranks that file, so a value here
    /// wins for the restored panes and changes nothing for other sessions.
    pub effort: Option<String>,
    pub mode: ResumeMode,
}

/// The levels `claude --effort` accepts. Anything else is dropped rather than
/// passed along: this string is pasted onto a command line typed into a live
/// terminal, the same reason `claude_transcript_exists` vets session ids.
const EFFORT_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

impl ResumePrefs {
    /// The command line for one restored pane, without its trailing newline.
    pub fn resume_command(&self, session_id: &str) -> String {
        let mut line = format!("claude --resume {session_id}");
        if let Some(level) = self
            .effort
            .as_deref()
            .filter(|level| EFFORT_LEVELS.contains(level))
        {
            line.push_str(" --effort ");
            line.push_str(level);
        }
        line
    }
}

/// A keystroke for the resume menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeKey {
    Up,
    Down,
    Enter,
}

impl ResumeKey {
    pub fn bytes(self) -> &'static [u8] {
        match self {
            ResumeKey::Up => b"\x1b[A",
            ResumeKey::Down => b"\x1b[B",
            ResumeKey::Enter => b"\r",
        }
    }
}

/// What to type at a pane that may be showing the resume menu, given what is on
/// its screen right now. `None` means "nothing to do yet" — the menu has not
/// appeared, or never will for this conversation.
///
/// Matching is anchored on the cursor marker rather than the menu's prose,
/// because a restored conversation's own scrollback can contain that prose. The
/// caller keeps asking until it gets `Enter`, so moving the cursor and
/// confirming it are two separate rounds with a fresh screen read between them:
/// amux never presses Enter on a menu it has not just seen the cursor on.
pub fn resume_dialog_step(screen: &str, mode: ResumeMode) -> Option<ResumeKey> {
    let on_summary = screen.contains("\u{276f} 1. Resume from summary");
    let on_full = screen.contains("\u{276f} 2. Resume full session");
    match mode {
        ResumeMode::Ask => None,
        ResumeMode::Summary if on_summary => Some(ResumeKey::Enter),
        ResumeMode::Summary if on_full => Some(ResumeKey::Up),
        ResumeMode::Full if on_full => Some(ResumeKey::Enter),
        ResumeMode::Full if on_summary => Some(ResumeKey::Down),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Disk I/O
// ---------------------------------------------------------------------------

pub fn session_path() -> PathBuf {
    if let Some(explicit) = std::env::var_os(ENV_SESSION_FILE) {
        return PathBuf::from(explicit);
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .or_else(|| {
            // Windows has no $HOME; %APPDATA% is the equivalent per-user spot.
            std::env::var_os("APPDATA").map(PathBuf::from)
        })
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("amux").join("session.json")
}

/// Read the previous session, or `None` when there is nothing to restore.
///
/// A file we cannot parse is moved aside rather than deleted or overwritten:
/// the user may want to look at it, and silently destroying the one record of
/// their layout is exactly the failure this whole module exists to prevent.
pub fn load() -> Option<SavedSession> {
    let path = session_path();
    let raw = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str::<SavedSession>(&raw) {
        Ok(session) if session.version != FORMAT_VERSION => {
            tracing::warn!(
                "session file version {} != {FORMAT_VERSION}; not restoring",
                session.version
            );
            None
        }
        // Saved-but-empty is a legitimate state (the user closed everything);
        // there is simply nothing to offer.
        Ok(session) if session.is_empty() => None,
        Ok(session) => Some(session),
        Err(e) => {
            tracing::warn!("session file unreadable ({e}); moving it aside");
            let _ = std::fs::rename(&path, path.with_extension("json.bad"));
            None
        }
    }
}

/// Write the session atomically: a full temp file, flushed to the platter,
/// then renamed over the real one. A crash mid-write therefore leaves either
/// the old complete file or the new complete file, never a half of either.
pub fn save(session: &SavedSession) -> anyhow::Result<()> {
    let path = session_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let stamped = SavedSession { saved_at_ms: now_ms(), ..session.clone() };
    let json = serde_json::to_string_pretty(&stamped)?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Copy the file we are about to start replacing to `session.prev.json`,
/// keeping exactly one generation of history.
///
/// The path that needs it: the user declines the restore offer, opens a single
/// workspace by hand, and quits. The autosave arms on that one workspace and
/// the multi-workspace file they declined is overwritten — with no other copy,
/// gone for good. Called once per run, at the moment the saver arms.
pub fn keep_previous_generation() {
    let path = session_path();
    // Fails harmlessly when there is no file yet (a first-ever launch).
    let _ = std::fs::copy(&path, path.with_extension("prev.json"));
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Autosave gate
// ---------------------------------------------------------------------------

/// Decide whether `snapshot` may be written, arming the saver as a side effect.
///
/// The hazard this exists for: amux dies, the user relaunches, and the fresh
/// window comes up with **zero** workspaces on purpose (see the setup hook in
/// `src-tauri/src/lib.rs`). An autosave running at that moment would write
/// "no workspaces" straight over the file the user is about to restore from.
///
/// So the saver stays *disarmed* until it has seen a non-empty state in this
/// run. Only after that is an empty state a real user action — they closed
/// everything — and worth recording.
pub fn arm_and_check(armed: &mut bool, snapshot: &SavedSession) -> bool {
    if !snapshot.is_empty() {
        *armed = true;
    }
    *armed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_with(workspaces: usize) -> SavedSession {
        SavedSession {
            version: FORMAT_VERSION,
            saved_at_ms: 0,
            workspaces: (0..workspaces)
                .map(|i| SavedWorkspace {
                    name: format!("ws {i}"),
                    tabs: vec![SavedTab {
                        name: "탭 1".into(),
                        layout: SavedLayout::Leaf { cwd: None, claude_session: None },
                        active_pane: 0,
                    }],
                    active_tab: 0,
                    tab_created_count: 1,
                })
                .collect(),
            active_workspace: (workspaces > 0).then_some(0),
            workspace_created_count: workspaces,
        }
    }

    /// The anti-clobber rule. A launch that starts empty must not be able to
    /// erase the saved session before the user has had a chance to restore it.
    #[test]
    fn empty_launch_never_overwrites() {
        let mut armed = false;
        // Fresh launch: engine empty, nothing has happened yet.
        assert!(!arm_and_check(&mut armed, &session_with(0)));
        assert!(!arm_and_check(&mut armed, &session_with(0)));
        // The user opens a workspace — now the state is worth saving.
        assert!(arm_and_check(&mut armed, &session_with(1)));
        // ...and once armed, closing everything IS a deliberate action, so the
        // now-empty state is written and the stale session stops being offered.
        assert!(arm_and_check(&mut armed, &session_with(0)));
    }

    #[test]
    fn summary_counts_the_whole_tree() {
        let mut session = session_with(2);
        session.workspaces[0].tabs[0].layout = SavedLayout::Split {
            axis: SplitAxis::Horizontal,
            ratio: 0.5,
            first: Box::new(SavedLayout::Leaf { cwd: Some("/a".into()), claude_session: None }),
            second: Box::new(SavedLayout::Split {
                axis: SplitAxis::Vertical,
                ratio: 0.3,
                first: Box::new(SavedLayout::Leaf { cwd: None, claude_session: None }),
                second: Box::new(SavedLayout::Leaf { cwd: None, claude_session: None }),
            }),
        };
        let summary = session.summary();
        assert_eq!(summary.workspaces, 2);
        assert_eq!(summary.tabs, 2);
        assert_eq!(summary.panes, 4); // 3 in the split tab + 1 in the plain one
    }

    /// The file has to survive a round trip byte-for-byte in meaning, since a
    /// restore reads back exactly what an autosave wrote.
    #[test]
    fn json_round_trip() {
        let session = session_with(3);
        let json = serde_json::to_string(&session).unwrap();
        let back: SavedSession = serde_json::from_str(&json).unwrap();
        assert_eq!(session, back);
    }
}
