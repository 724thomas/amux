//! Pane metadata sweeper: cwd, git branch, listening ports.
//!
//! A single engine task polls every pane on a 2-second cadence and emits a
//! state change only when a value actually differs. Values are derived from
//! the OS directly (procfs on Linux, libproc on macOS, Win32 on Windows) — no
//! subprocesses are spawned.

mod cwd;
mod git;
mod ports;

use std::path::PathBuf;

use amux_protocol::PaneMeta;

use crate::pane::Pane;

pub fn compute(pane: &Pane) -> PaneMeta {
    // What is running in the pane *right now*, so `cd` inside the shell and
    // TUI apps both resolve to the directory the user would expect.
    let fg_pid = foreground_pid(pane);

    // Stale kitty-mode guard: if the shell itself is foreground again but a
    // killed app left the kitty keyboard mode pushed, pop it — otherwise the
    // frontend keeps sending CSI-u encodings the shell can't read.
    if pane.term.kitty_keyboard_active() && fg_pid == pane.child_pid() {
        pane.term.reset_kitty_keyboard();
    }

    let cwd = resolve_cwd(pane, fg_pid);
    let git_branch = cwd.as_deref().and_then(git::branch_for);
    let listening_ports = pane
        .child_pid()
        .map(ports::listening_ports)
        .unwrap_or_default();
    PaneMeta {
        cwd: cwd.map(|p| p.to_string_lossy().into_owned()),
        git_branch,
        listening_ports,
        title: None, // OSC 0/2 titles land in M5
        kitty_keyboard: pane.term.kitty_keyboard_active(),
    }
}

/// The process whose view of the world the sidebar should show: the command
/// running in the pane, or the shell itself when the prompt is idle.
///
/// Unix reads it from the PTY — the foreground process group leader
/// (`tcgetpgrp`) is exactly this, maintained by the kernel.
#[cfg(not(windows))]
fn foreground_pid(pane: &Pane) -> Option<u32> {
    pane.shell_pid()
}

/// ConPTY has no foreground process group, so the shell's newest live child
/// stands in for one: a command typed at the prompt runs as a child of the
/// shell, and when none is running the shell *is* the foreground. That makes
/// the `fg_pid == child_pid()` comparison above mean the same thing on both
/// platforms — including for the kitty-mode guard.
#[cfg(windows)]
fn foreground_pid(pane: &Pane) -> Option<u32> {
    let shell = pane.child_pid()?;
    Some(crate::win_proc::tree().newest_child(shell).unwrap_or(shell))
}

#[cfg(not(windows))]
fn resolve_cwd(_pane: &Pane, fg_pid: Option<u32>) -> Option<PathBuf> {
    fg_pid.and_then(cwd::process_cwd)
}

/// Windows needs one extra step, because a PowerShell prompt sitting idle
/// cannot be asked where it is.
///
/// `Set-Location` moves PowerShell's *provider* location and deliberately
/// leaves the process's own working directory alone (it is shared by every
/// runspace in the process, so moving it would not be thread-safe). The
/// shell's directory is therefore frozen at the one amux spawned it in,
/// forever.
///
/// What PowerShell does do is hand each command it launches the provider
/// location as that command's working directory. So a running command knows
/// the true answer, and is the one to ask. Between commands the last answer
/// still stands — the user has to type `cd` to invalidate it, and typing `cd`
/// is not something we can see — so it beats falling back to a spawn
/// directory already known to be stale.
#[cfg(windows)]
fn resolve_cwd(pane: &Pane, fg_pid: Option<u32>) -> Option<PathBuf> {
    if fg_pid.is_some() && fg_pid != pane.child_pid() {
        if let Some(dir) = fg_pid.and_then(cwd::process_cwd) {
            return Some(dir);
        }
    }
    // Idle at the prompt: keep what the last command reported. `compute` is
    // called before the sweeper takes this lock to store the result, so
    // reading it here cannot deadlock.
    if let Some(last) = pane.meta.lock().cwd.as_deref() {
        return Some(PathBuf::from(last));
    }
    // Nothing has run yet: the shell has not moved from where it was spawned,
    // so its own directory is the right answer.
    fg_pid.and_then(cwd::process_cwd)
}
