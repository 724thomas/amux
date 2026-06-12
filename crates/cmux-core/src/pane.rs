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

use cmux_protocol::{env_keys, PaneId, PaneMeta, WorkspaceId};
use parking_lot::Mutex;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};

use crate::term_state::TermState;

/// Receives raw output bytes; registered by the UI layer per pane.
pub type OutputSink = Box<dyn Fn(&[u8]) + Send + Sync>;

const READ_BUF: usize = 64 * 1024;
/// Cap for the raw replay buffer; old bytes are dropped from the front.
const TAIL_CAP: usize = 512 * 1024;

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
            cmux_protocol::default_socket_path().as_os_str(),
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
        });

        // Reader thread: PTY → term state → tail buffer → sink.
        {
            let pane = Arc::clone(&pane);
            std::thread::Builder::new()
                .name(format!("pty-read-{}", id.short()))
                .spawn(move || {
                    let mut buf = vec![0u8; READ_BUF];
                    loop {
                        match reader.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                let chunk = &buf[..n];
                                pane.term.advance(chunk);
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

    pub fn write(&self, data: &[u8]) -> anyhow::Result<()> {
        let mut writer = self.writer.lock();
        writer.write_all(data)?;
        writer.flush()?;
        Ok(())
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
        )
        .expect("spawn pane");

        pane.write(b"echo cmux-$((40+2))\n").expect("write");

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let screen = pane.term.read_screen();
            if screen.contains("cmux-42") {
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
        let pane = Pane::spawn(PaneId::new(), WorkspaceId::new(), "test".into(), 80, 24, None, || {})
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
