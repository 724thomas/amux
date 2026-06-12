//! Pane metadata sweeper: cwd, git branch, listening ports.
//!
//! A single engine task polls every pane on a 2-second cadence and emits a
//! state change only when a value actually differs. Everything is derived
//! from /proc — no subprocesses are spawned.

mod cwd;
mod git;
mod ports;

use cmux_protocol::PaneMeta;

use crate::pane::Pane;

pub fn compute(pane: &Pane) -> PaneMeta {
    // Foreground process group leader (tcgetpgrp): reflects what is running
    // *right now* in the pane, so `cd` inside the shell and TUI apps both
    // resolve to the directory the user would expect.
    let fg_pid = pane.shell_pid();
    let cwd = fg_pid.and_then(cwd::process_cwd);
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
    }
}
