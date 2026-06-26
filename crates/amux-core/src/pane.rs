//! One pane = one PTY + one shell + one server-side terminal state.
//!
//! Lifecycle: `Pane::spawn` opens a PTY, starts the shell, and launches a
//! dedicated reader thread. Every output chunk is fed to the terminal state
//! machine first (so `read_screen` is always current), appended to a bounded
//! raw tail buffer (best-effort replay for late subscribers), and finally
//! handed to the registered output sink (the Tauri layer's IPC channel).

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use amux_protocol::{env_keys, PaneId, PaneMeta, PaneNotification, PaneStatus, WorkspaceId};
use parking_lot::Mutex;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};

use crate::osc::{OscDetector, OscEvent};
use crate::term_state::TermState;

/// Receives raw output bytes; registered by the UI layer per pane.
pub type OutputSink = Box<dyn Fn(&[u8]) + Send + Sync>;

const READ_BUF: usize = 64 * 1024;
/// Cap for the raw replay buffer; old bytes are dropped from the front.
const TAIL_CAP: usize = 512 * 1024;
/// Output this soon after user input is echo / prompt repaint, not work —
/// it must not flip the pane to `processing` (e.g. typing into Claude's
/// input box redraws the whole prompt on every keystroke).
const ECHO_WINDOW: std::time::Duration = std::time::Duration::from_millis(1500);

pub struct Pane {
    pub id: PaneId,
    pub workspace: WorkspaceId,
    pub name: Mutex<String>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    pub term: TermState,
    sink: Arc<Mutex<Option<OutputSink>>>,
    tail: Arc<Mutex<Vec<u8>>>,
    exited: Arc<AtomicBool>,
    /// Root PID of the spawned shell (for the metadata sweeper's process walk).
    child_pid: Option<u32>,
    pub meta: Mutex<PaneMeta>,
    pub notification: Mutex<Option<PaneNotification>>,
    /// Work-output activity for status detection (read by the sweeper).
    /// Output arriving right after user input is echo/repaint, not work,
    /// and is not recorded here (see `ECHO_WINDOW`).
    pub activity: Mutex<Activity>,
    /// When the user last typed/pasted into this pane.
    pub last_input: Mutex<std::time::Instant>,
    /// Work status state machine (driven by the sweeper + focus events).
    pub status: Mutex<PaneStatus>,
    /// Set when a waiting-for-input signal (hook/bell) arrives; cleared by
    /// the sweeper once output resumes after that instant.
    pub waiting_since: Mutex<Option<std::time::Instant>>,
    /// True once lifecycle hooks (progress/done) drive this pane's status —
    /// the silence heuristic then stands down (TUIs like Claude Code repaint
    /// constantly, so silence never happens). Reset when the app exits.
    pub hook_managed: std::sync::atomic::AtomicBool,
    /// A Claude turn is in flight (between a turn-starting `progress` and the
    /// `done` that ends it). Mid-turn `attention` is a real permission/input
    /// request → waiting; `attention` after the turn ended is Claude's idle
    /// "waiting for your input" notification and must not override `processed`.
    pub turn_active: std::sync::atomic::AtomicBool,
    /// When the last `done` landed. A `progress` arriving within a short grace
    /// window after it is a same-turn straggler (hooks are fire-and-forget, so
    /// the final PostToolUse can be delivered just after Stop) and must not
    /// resurrect `processing`.
    pub last_done_at: Mutex<Option<std::time::Instant>>,
}

#[derive(Clone, Copy)]
pub struct Activity {
    pub last_output: std::time::Instant,
    /// Start of the current output burst (gaps > 2s start a new burst).
    pub burst_start: std::time::Instant,
}

impl Default for Activity {
    fn default() -> Self {
        let now = std::time::Instant::now();
        Self { last_output: now, burst_start: now }
    }
}

impl Pane {
    pub fn spawn(
        id: PaneId,
        workspace: WorkspaceId,
        name: String,
        cols: u16,
        rows: u16,
        cwd: Option<std::path::PathBuf>,
        on_exit: impl FnOnce() + Send + 'static,
        on_osc: impl Fn(OscEvent) + Send + 'static,
    ) -> anyhow::Result<Arc<Self>> {
        let pty = native_pty_system().openpty(PtySize {
            rows: rows.max(2),
            cols: cols.max(2),
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
        let mut cmd = CommandBuilder::new(&shell);
        cmd.arg("-l");
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env(env_keys::PANE_ID, id.to_string());
        cmd.env(env_keys::WORKSPACE_ID, workspace.to_string());
        cmd.env(
            env_keys::SOCKET,
            amux_protocol::default_socket_path().as_os_str(),
        );
        let cwd = cwd
            .or_else(|| std::env::var_os("HOME").map(Into::into))
            .unwrap_or_else(|| "/".into());
        cmd.cwd(cwd);

        let mut child = pty.slave.spawn_command(cmd)?;
        drop(pty.slave);

        let killer = child.clone_killer();
        let child_pid = child.process_id();
        let mut reader = pty.master.try_clone_reader()?;
        let writer = pty.master.take_writer()?;

        let pane = Arc::new(Self {
            id,
            workspace,
            name: Mutex::new(name),
            master: Mutex::new(pty.master),
            writer: Mutex::new(writer),
            killer: Mutex::new(killer),
            term: TermState::new(cols, rows),
            sink: Arc::new(Mutex::new(None)),
            tail: Arc::new(Mutex::new(Vec::new())),
            exited: Arc::new(AtomicBool::new(false)),
            child_pid,
            meta: Mutex::new(PaneMeta::default()),
            notification: Mutex::new(None),
            activity: Mutex::new(Activity::default()),
            last_input: Mutex::new(std::time::Instant::now()),
            status: Mutex::new(PaneStatus::default()),
            waiting_since: Mutex::new(None),
            hook_managed: std::sync::atomic::AtomicBool::new(false),
            turn_active: std::sync::atomic::AtomicBool::new(false),
            last_done_at: Mutex::new(None),
        });

        // Reader thread: PTY → term state → tail buffer → sink.
        {
            let pane = Arc::clone(&pane);
            std::thread::Builder::new()
                .name(format!("pty-read-{}", id.short()))
                .spawn(move || {
                    let mut buf = vec![0u8; READ_BUF];
                    let mut osc = OscDetector::default();
                    loop {
                        match reader.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                let chunk = &buf[..n];
                                {
                                    let now = std::time::Instant::now();
                                    let echo =
                                        now.duration_since(*pane.last_input.lock()) < ECHO_WINDOW;
                                    if !echo {
                                        let mut activity = pane.activity.lock();
                                        if now.duration_since(activity.last_output)
                                            > std::time::Duration::from_secs(2)
                                        {
                                            activity.burst_start = now;
                                        }
                                        activity.last_output = now;
                                    }
                                }
                                // Terminal replies (kitty keyboard mode reports)
                                // go straight back to the application.
                                for reply in pane.term.advance(chunk) {
                                    let _ = pane.write(reply.as_bytes());
                                }
                                for event in osc.advance(chunk) {
                                    on_osc(event);
                                }
                                // Lock order sink → tail (same as set_sink) so a
                                // concurrent subscribe can't replay a chunk that
                                // is also about to be sent live.
                                let sink = pane.sink.lock();
                                {
                                    let mut tail = pane.tail.lock();
                                    tail.extend_from_slice(chunk);
                                    let len = tail.len();
                                    if len > TAIL_CAP {
                                        tail.drain(..len - TAIL_CAP);
                                    }
                                }
                                if let Some(sink) = &*sink {
                                    sink(chunk);
                                }
                            }
                        }
                    }
                })?;
        }

        // Waiter thread: reap the child so it never zombies, then notify.
        {
            let exited = Arc::clone(&pane.exited);
            std::thread::Builder::new()
                .name(format!("pty-wait-{}", id.short()))
                .spawn(move || {
                    let _ = child.wait();
                    exited.store(true, Ordering::SeqCst);
                    on_exit();
                })?;
        }

        Ok(pane)
    }

    /// Raw write (terminal protocol replies). User input goes through
    /// `write_input` so the echo window can tell typing apart from work.
    pub fn write(&self, data: &[u8]) -> anyhow::Result<()> {
        let mut writer = self.writer.lock();
        writer.write_all(data)?;
        writer.flush()?;
        Ok(())
    }

    /// User/CLI input: stamps `last_input` so the output that immediately
    /// follows (echo, prompt repaint) is not mistaken for work output.
    pub fn write_input(&self, data: &[u8]) -> anyhow::Result<()> {
        *self.last_input.lock() = std::time::Instant::now();
        self.write(data)
    }

    pub fn resize(&self, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.master.lock().resize(PtySize {
            rows: rows.max(2),
            cols: cols.max(2),
            pixel_width: 0,
            pixel_height: 0,
        })?;
        self.term.resize(cols, rows);
        Ok(())
    }

    /// Register the live output sink, first replaying the raw tail so a
    /// late subscriber (e.g. app restart) sees the recent screen.
    pub fn set_sink(&self, sink: OutputSink) {
        // Lock order sink → tail, mirroring the reader thread.
        let mut sink_slot = self.sink.lock();
        {
            let tail = self.tail.lock();
            if !tail.is_empty() {
                sink(&tail);
            }
        }
        *sink_slot = Some(sink);
    }

    pub fn has_exited(&self) -> bool {
        self.exited.load(Ordering::SeqCst)
    }

    /// PID of the foreground process group leader (what runs in the pane now).
    pub fn shell_pid(&self) -> Option<u32> {
        self.master.lock().process_group_leader().map(|p| p as u32)
    }

    /// Root PID of the shell process spawned at pane creation.
    pub fn child_pid(&self) -> Option<u32> {
        self.child_pid
    }

    pub fn kill(&self) {
        let _ = self.killer.lock().kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Headless end-to-end: spawn a shell, run a command, read the screen.
    #[test]
    fn echo_round_trip() {
        let pane = Pane::spawn(
            PaneId::new(),
            WorkspaceId::new(),
            "test".into(),
            80,
            24,
            None,
            || {},
            |_| {},
        )
        .expect("spawn pane");

        pane.write(b"echo amux-$((40+2))\n").expect("write");

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let screen = pane.term.read_screen();
            if screen.contains("amux-42") {
                break;
            }
            assert!(Instant::now() < deadline, "screen never showed output:\n{screen}");
            std::thread::sleep(Duration::from_millis(100));
        }

        pane.kill();
    }

    /// Sinks receive live output, and late subscribers get the tail replay.
    #[test]
    fn sink_receives_output_with_replay() {
        let pane = Pane::spawn(
            PaneId::new(),
            WorkspaceId::new(),
            "test".into(),
            80,
            24,
            None,
            || {},
            |_| {},
        )
        .expect("spawn pane");
        pane.write(b"echo replay-me\n").expect("write");
        std::thread::sleep(Duration::from_millis(1500));

        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        pane.set_sink(Box::new(move |chunk| {
            let _ = tx.send(chunk.to_vec());
        }));

        let mut collected = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(chunk) = rx.recv_timeout(Duration::from_millis(200)) {
                collected.extend_from_slice(&chunk);
                if String::from_utf8_lossy(&collected).contains("replay-me") {
                    break;
                }
            }
        }
        assert!(
            String::from_utf8_lossy(&collected).contains("replay-me"),
            "sink never saw replayed output"
        );

        pane.kill();
    }
}
