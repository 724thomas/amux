//! Structured Claude chat panes.
//!
//! A chat pane talks to Claude Code directly instead of showing its TUI: the
//! app runs `claude -p` with JSON lines on both stdin and stdout (the same
//! wire the official Agent SDK uses) as a plain child process, and relays
//! every line it prints to the webview, which draws the conversation. The
//! pane's own shell keeps running, idle, underneath; the engine only carries
//! a flag saying the chat view is on screen (`Engine::set_pane_chat`).
//!
//! This layer stays nearly protocol-agnostic. The frontend builds every line
//! it sends (user messages, answers to permission requests, interrupts) and
//! parses every line it receives. The few things this side reads off the
//! stream are the ones the engine needs for the pane's agent status:
//! - `system/init` carries the conversation id → saved with the layout.
//! - `control_request` / `can_use_tool` means Claude waits for the person →
//!   `attention`. Claude Code fires no `Notification` hook in this mode, so
//!   nothing else would ever flag the wait.
//! - A turn that ends other than in success (an interrupt) fires no `Stop`
//!   hook either → quiet `idle`.
//!
//! Everything else comes from the user's Claude Code hooks, which do fire
//! here (SessionStart, UserPromptSubmit, PostToolUse, Stop). They reach the
//! right pane because the child gets the pane's `AMUX_PANE_ID` and socket,
//! exactly like a shell in a terminal pane does. `done` is deliberately left
//! to the `Stop` hook: sending it from here as well would double the desktop
//! notification.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use amux_core::Engine;
use amux_protocol::{env_keys, NotifyKind, PaneId};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};

/// Every spawn gets a fresh number. Lines are tagged with it so output from a
/// chat that was stopped (and is still draining) can never land in the chat
/// that replaced it on the same pane.
static NEXT_GEN: AtomicU64 = AtomicU64::new(1);

/// Variables that tell a process it runs inside a particular Claude Code
/// session. amux itself may have been started from inside one (a dev build
/// launched from an agent's shell); the chat must not inherit that identity.
const SESSION_IDENTITY_ENV: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_PID",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_EXECPATH",
];

/// Claude Code's own system prompt says its replies go to a terminal, so asked
/// for a picture it draws box-drawing text art. This view renders Markdown,
/// draws ```mermaid blocks as diagrams and ```svg blocks as images
/// (ChatView.svelte, `drawDiagrams`), and plays file edits live in an editor
/// (FilesPanel.svelte), so tell it that, ask for picture-first answers, and
/// ask for Edit/Write instead of shell edits: in auto mode Claude Code itself
/// suggests sed or heredoc edits through Bash, which the editor cannot follow
/// while they happen (seen 2026-11-05: a python heredoc edited three files). Claude Code records the system
/// prompt on a conversation's first request and reuses that record on
/// `--resume`, so a change here only reaches conversations started after it.
const CHAT_SYSTEM_PROMPT: &str = r#"You are running inside the amux chat view, not a terminal. Your replies are rendered as GitHub-flavored Markdown in a web view: headings, lists, tables and links show as formatted text, fenced code blocks tagged mermaid are drawn as diagrams, and fenced code blocks tagged svg are shown as images. The view also opens every file you change in an editor next to the conversation and shows your change being typed into it as you make it.
File changes: the editor can only follow the Edit and Write tools, so in this view create and modify files only with Edit (one Edit per change) and Write (new files or full rewrites). Do this even where auto mode or other guidance says small or mechanical edits may be made with sed, heredocs, or scripts run through Bash: a change made through Bash is invisible to the user until it is done. Use Bash to run programs, builds and tests, not to change file contents.
When you answer a question (how something works, how code or a system is structured, how parts relate, what happens in what order, how options compare), lead with a picture: one or more ```mermaid diagrams (flowchart, sequenceDiagram, classDiagram, stateDiagram-v2, erDiagram, gantt, mindmap, timeline, pie, xychart-beta), or an ```svg illustration when mermaid cannot express it, then explain in text. Do the same whenever the user asks for a picture, diagram, chart, map or visual overview (for example 그림, 다이어그램, 도식, 흐름도, 구조도). Skip the picture for one-line answers, for pure code requests, and when the user asks for text only. While you are changing files you do not need pictures, because the view already shows your edits. Draw pictures instead of ASCII or box-drawing art, and split a very large picture into a few smaller ones.
Mermaid tips: in flowcharts, wrap node and edge labels in double quotes, e.g. A["EMS-backend (rfp/clean)"] and A -->|"calls"| B, so spaces, parentheses, slashes and Korean text parse. In sequence diagrams, write participant names and message text without quotes (quotes would show up literally). Use <br/> for a line break inside a label, keep labels short, and do not put Markdown inside labels.
SVG rules: one self-contained <svg> element with a viewBox, drawn for a white background with dark text; no scripts, no foreignObject, no external images or fonts."#;

/// The `claude` command line of a chat.
fn chat_args(model: Option<&str>, permission_mode: Option<&str>, resume: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        // Permission requests and AskUserQuestion come to us over stdio as
        // `control_request` / `can_use_tool`, answered with `control_response`.
        "--permission-prompt-tool",
        "stdio",
        "--append-system-prompt",
        CHAT_SYSTEM_PROMPT,
    ]
    .into_iter()
    .map(String::from)
    .collect();
    let mut flag = |name: &str, value: Option<&str>| {
        if let Some(v) = value.filter(|v| !v.is_empty()) {
            args.push(name.into());
            args.push(v.into());
        }
    };
    flag("--model", model);
    flag("--permission-mode", permission_mode);
    flag("--resume", resume);
    args
}

struct ChatProc {
    gen: u64,
    pid: u32,
    /// `None` once the input side is closed (stopping).
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    exited: Arc<AtomicBool>,
    log: Option<Arc<Mutex<std::fs::File>>>,
}

/// Running chats, one per pane at most.
#[derive(Default)]
pub struct Chats(Mutex<HashMap<PaneId, ChatProc>>);

#[derive(Clone, Serialize)]
struct ChatEvent {
    pane: PaneId,
    gen: u64,
    /// `line` (one JSON line from stdout), `stderr`, or `exit`.
    kind: &'static str,
    data: String,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Where `claude` lives. `$AMUX_CLAUDE_BIN` wins; then `$PATH`; then the
/// places the installers put it. A desktop launcher's `$PATH` often lacks
/// `~/.local/bin`, where the native installer links the binary.
fn find_claude() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("AMUX_CLAUDE_BIN") {
        if !explicit.is_empty() {
            return Some(explicit.into());
        }
    }
    let exe = if cfg!(windows) { "claude.exe" } else { "claude" };
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(exe);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let under_home = [".local/bin/claude", ".claude/local/claude", ".bun/bin/claude"];
    for rel in under_home {
        if let Some(candidate) = home.as_ref().map(|h| h.join(rel)) {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    ["/usr/local/bin/claude", "/opt/homebrew/bin/claude", "/usr/bin/claude"]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
}

/// Opt-in wire log for debugging: with `$AMUX_CHAT_LOG_DIR` set, every line
/// in each direction is appended to `<dir>/chat-<pane>-<gen>.jsonl`.
fn open_log(pane: PaneId, gen: u64) -> Option<Arc<Mutex<std::fs::File>>> {
    let dir = std::env::var_os("AMUX_CHAT_LOG_DIR")?;
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("chat-{}-{gen}.jsonl", pane.short()));
    let file = std::fs::OpenOptions::new().create(true).append(true).open(path).ok()?;
    Some(Arc::new(Mutex::new(file)))
}

fn log_line(log: &Option<Arc<Mutex<std::fs::File>>>, dir: &str, line: &str) {
    if let Some(log) = log {
        if let Ok(mut f) = log.lock() {
            let t = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let entry = serde_json::json!({ "t": t, "dir": dir, "line": line });
            let _ = writeln!(f, "{entry}");
        }
    }
}

/// Stop a chat process now. Closing stdin alone is not enough: in `-p` mode
/// Claude Code reads end-of-input as "no more messages" and still finishes the
/// turn in flight, tools included, with no screen left to show it. So SIGTERM
/// goes out right away too (Claude Code takes its running commands down on it
/// and its transcript is already on disk, so the conversation can be resumed).
fn begin_stop(proc: &ChatProc) {
    if let Ok(mut stdin) = proc.stdin.lock() {
        stdin.take();
    }
    // Already reaped: the pid may belong to someone else by now.
    if !proc.exited.load(Ordering::SeqCst) {
        signal(proc.pid, false);
    }
}

/// `begin_stop`, then SIGKILL in the background if it is still alive after a
/// grace period. For while the app keeps running (closing one chat or pane).
fn shut_down(proc: ChatProc) {
    begin_stop(&proc);
    std::thread::spawn(move || {
        for _ in 0..60 {
            if proc.exited.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        signal(proc.pid, true);
    });
}

#[cfg(unix)]
fn signal(pid: u32, hard: bool) {
    let sig = if hard { "-KILL" } else { "-TERM" };
    let _ = Command::new("kill").arg(sig).arg(pid.to_string()).status();
}

#[cfg(windows)]
fn signal(pid: u32, _hard: bool) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status();
}

impl Chats {
    fn stop(&self, pane: PaneId) -> bool {
        let proc = self.0.lock().ok().and_then(|mut map| map.remove(&pane));
        match proc {
            Some(proc) => {
                shut_down(proc);
                true
            }
            None => false,
        }
    }

    /// Stop the chats whose pane is gone (closed pane, tab or workspace).
    pub fn retain_panes(&self, alive: &HashSet<PaneId>) {
        let gone: Vec<ChatProc> = match self.0.lock() {
            Ok(mut map) => {
                let dead: Vec<PaneId> = map.keys().filter(|p| !alive.contains(p)).copied().collect();
                dead.into_iter().filter_map(|p| map.remove(&p)).collect()
            }
            Err(_) => Vec::new(),
        };
        for proc in gone {
            shut_down(proc);
        }
    }

    /// The window is closing: end every chat. The app exits right after this,
    /// and a delayed kill on a background thread would die with it, so wait
    /// here (bounded) and SIGKILL whatever is still around.
    pub fn stop_all(&self) {
        let all: Vec<ChatProc> = match self.0.lock() {
            Ok(mut map) => map.drain().map(|(_, proc)| proc).collect(),
            Err(_) => Vec::new(),
        };
        if all.is_empty() {
            return;
        }
        for proc in &all {
            begin_stop(proc);
        }
        let deadline = std::time::Instant::now() + Duration::from_millis(1200);
        while std::time::Instant::now() < deadline
            && !all.iter().all(|p| p.exited.load(Ordering::SeqCst))
        {
            std::thread::sleep(Duration::from_millis(50));
        }
        for proc in &all {
            if !proc.exited.load(Ordering::SeqCst) {
                signal(proc.pid, true);
            }
        }
    }
}

/// What the engine should hear about one line Claude printed.
fn status_signal(value: &Value) -> Option<(NotifyKind, Option<String>, Option<String>)> {
    match value["type"].as_str()? {
        "control_request" if value["request"]["subtype"] == "can_use_tool" => {
            let request = &value["request"];
            let tool = request["tool_name"].as_str().unwrap_or("도구");
            let body = if tool == "AskUserQuestion" {
                "Claude 가 질문했습니다".to_string()
            } else {
                let what = request["description"]
                    .as_str()
                    .or_else(|| request["display_name"].as_str())
                    .unwrap_or(tool);
                format!("{tool} 승인 요청: {what}")
            };
            Some((NotifyKind::Attention, Some("Claude 채팅".into()), Some(body)))
        }
        "result" if value["subtype"] != "success" => Some((NotifyKind::Idle, None, None)),
        _ => None,
    }
}

/// Start (or restart) a structured Claude chat in `pane`. Returns the
/// generation number every event of this chat will carry.
#[tauri::command]
pub fn chat_start(
    app: AppHandle,
    engine: State<'_, Arc<Engine>>,
    chats: State<'_, Arc<Chats>>,
    pane: PaneId,
    model: Option<String>,
    permission_mode: Option<String>,
    resume: Option<String>,
) -> Result<u64, String> {
    let engine = engine.inner();
    let chats = chats.inner();
    let info = engine.pane(pane).map_err(err)?;
    let claude = find_claude().ok_or_else(|| {
        "claude 실행 파일을 찾지 못했습니다. PATH 에 claude 가 있는지, 또는 AMUX_CLAUDE_BIN 을 확인하세요.".to_string()
    })?;
    chats.stop(pane);

    let cwd = info
        .meta
        .lock()
        .cwd
        .clone()
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from));

    let mut cmd = Command::new(&claude);
    cmd.args(chat_args(model.as_deref(), permission_mode.as_deref(), resume.as_deref()));
    if let Some(cwd) = &cwd {
        cmd.current_dir(cwd);
    }
    for key in SESSION_IDENTITY_ENV {
        cmd.env_remove(key);
    }
    cmd.env(env_keys::PANE_ID, pane.to_string());
    cmd.env(env_keys::WORKSPACE_ID, info.workspace.to_string());
    cmd.env(env_keys::SOCKET, amux_protocol::default_socket_name());
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| format!("claude 를 시작하지 못했습니다: {e}"))?;
    let gen = NEXT_GEN.fetch_add(1, Ordering::SeqCst);
    let pid = child.id();
    let stdin = Arc::new(Mutex::new(child.stdin.take()));
    let stdout = child.stdout.take().ok_or("stdout 없음")?;
    let stderr = child.stderr.take().ok_or("stderr 없음")?;
    let exited = Arc::new(AtomicBool::new(false));
    let log = open_log(pane, gen);
    log_line(
        &log,
        "start",
        &format!("{} cwd={:?} model={:?} mode={:?} resume={:?}", claude.display(), cwd, model, permission_mode, resume),
    );

    // stderr: drained in its own thread, or a chatty child would block once
    // the pipe buffer fills.
    {
        let app = app.clone();
        let log = log.clone();
        std::thread::Builder::new()
            .name(format!("chat-err-{}", pane.short()))
            .spawn(move || {
                let mut reader = BufReader::new(stderr);
                let mut buf = Vec::new();
                loop {
                    buf.clear();
                    match reader.read_until(b'\n', &mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let line = String::from_utf8_lossy(&buf).trim_end().to_string();
                            if line.is_empty() {
                                continue;
                            }
                            log_line(&log, "err", &line);
                            let _ = app.emit(
                                "chat:event",
                                ChatEvent { pane, gen, kind: "stderr", data: line },
                            );
                        }
                    }
                }
            })
            .map_err(err)?;
    }

    // stdout: one JSON value per line. Read as bytes: a stray invalid byte
    // must cost one line, not the whole reader.
    {
        let app = app.clone();
        let engine = Arc::clone(engine);
        let chats = Arc::clone(chats);
        let exited = Arc::clone(&exited);
        let log = log.clone();
        std::thread::Builder::new()
            .name(format!("chat-out-{}", pane.short()))
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                let mut buf = Vec::new();
                loop {
                    buf.clear();
                    match reader.read_until(b'\n', &mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let line = String::from_utf8_lossy(&buf).trim().to_string();
                            if line.is_empty() {
                                continue;
                            }
                            log_line(&log, "out", &line);
                            if let Ok(value) = serde_json::from_str::<Value>(&line) {
                                if value["type"] == "system" && value["subtype"] == "init" {
                                    if let Some(session) = value["session_id"].as_str() {
                                        let _ = engine.set_claude_session(pane, Some(session.into()));
                                    }
                                }
                                if let Some((kind, title, body)) = status_signal(&value) {
                                    engine.notify_pane(pane, kind, title, body);
                                }
                            }
                            let _ = app.emit(
                                "chat:event",
                                ChatEvent { pane, gen, kind: "line", data: line },
                            );
                        }
                    }
                }
                // stdout closed: the process is on its way out. Reap it.
                let code = child.wait().ok().and_then(|s| s.code());
                exited.store(true, Ordering::SeqCst);
                log_line(&log, "exit", &format!("{code:?}"));
                // Drop our entry only if it is still this generation.
                if let Ok(mut map) = chats.0.lock() {
                    if map.get(&pane).is_some_and(|p| p.gen == gen) {
                        map.remove(&pane);
                    }
                }
                engine.notify_pane(pane, NotifyKind::Idle, None, None);
                let _ = app.emit(
                    "chat:event",
                    ChatEvent {
                        pane,
                        gen,
                        kind: "exit",
                        data: code.map(|c| c.to_string()).unwrap_or_default(),
                    },
                );
            })
            .map_err(err)?;
    }

    if let Ok(mut map) = chats.0.lock() {
        map.insert(pane, ChatProc { gen, pid, stdin, exited, log });
    }
    engine.set_pane_chat(pane, true).map_err(err)?;
    Ok(gen)
}

/// Write one JSON line to the chat's stdin. The frontend builds the line.
#[tauri::command]
pub fn chat_write(
    engine: State<'_, Arc<Engine>>,
    chats: State<'_, Arc<Chats>>,
    pane: PaneId,
    gen: u64,
    line: String,
) -> Result<(), String> {
    let (stdin, log) = {
        let map = chats.0.lock().map_err(err)?;
        let proc = map.get(&pane).filter(|p| p.gen == gen).ok_or("채팅이 실행 중이 아닙니다")?;
        (Arc::clone(&proc.stdin), proc.log.clone())
    };
    let line = line.trim().to_string();
    {
        let mut guard = stdin.lock().map_err(err)?;
        let input = guard.as_mut().ok_or("채팅이 종료되는 중입니다")?;
        input.write_all(line.as_bytes()).map_err(err)?;
        input.write_all(b"\n").map_err(err)?;
        input.flush().map_err(err)?;
    }
    log_line(&log, "in", &line);
    // A message or an answer to a permission card means Claude works again.
    // `progress` is quiet, and sending it here (in-process, before the async
    // UserPromptSubmit hook lands) also opens the turn, so an `attention`
    // that follows is read as a real mid-turn wait.
    if let Ok(value) = serde_json::from_str::<Value>(&line) {
        if matches!(value["type"].as_str(), Some("user" | "control_response")) {
            engine.notify_pane(pane, NotifyKind::Progress, None, None);
        }
    }
    Ok(())
}

/// End the chat (if any) and give the pane its terminal back.
#[tauri::command]
pub fn chat_stop(
    engine: State<'_, Arc<Engine>>,
    chats: State<'_, Arc<Chats>>,
    pane: PaneId,
) -> Result<(), String> {
    chats.stop(pane);
    engine.set_pane_chat(pane, false).map_err(err)
}

/// Close the chat and give its conversation back to the pane's terminal
/// (src-tauri/src/handoff.rs): once the chat's Claude is gone, so that two
/// processes never write one session, type `claude --resume <session>` at
/// the shell. Only when the shell is at its prompt (nothing else in the
/// foreground) and the session has a transcript. Returns whether it typed.
#[tauri::command]
pub async fn chat_handback(
    engine: State<'_, Arc<Engine>>,
    chats: State<'_, Arc<Chats>>,
    pane: PaneId,
    session: String,
) -> Result<bool, String> {
    let engine = Arc::clone(engine.inner());
    let chats = Arc::clone(chats.inner());
    tauri::async_runtime::spawn_blocking(move || hand_back(&engine, &chats, pane, &session))
        .await
        .map_err(err)?
}

fn hand_back(engine: &Engine, chats: &Chats, pane: PaneId, session: &str) -> Result<bool, String> {
    let proc = chats.0.lock().ok().and_then(|mut map| map.remove(&pane));
    let gone = proc.map_or(true, stop_and_wait);
    engine.set_pane_chat(pane, false).map_err(err)?;
    if !gone {
        return Err("채팅의 Claude 가 끝나지 않아 터미널에서 다시 열지 않았습니다".into());
    }
    if !amux_core::session::claude_transcript_exists(session) {
        return Ok(false);
    }
    let p = engine.pane(pane).map_err(err)?;
    let shell = p.child_pid();
    if shell.is_none() || p.shell_pid() != shell {
        return Ok(false);
    }
    let line = format!("{}\r", amux_core::session::ResumePrefs::default().resume_command(session));
    engine.write_pane(pane, line.as_bytes()).map_err(err)?;
    Ok(true)
}

/// `begin_stop`, then wait for the process to end (SIGKILL after the same
/// grace period as `shut_down`). True once it has.
fn stop_and_wait(proc: ChatProc) -> bool {
    begin_stop(&proc);
    for i in 0..80 {
        if proc.exited.load(Ordering::SeqCst) {
            return true;
        }
        if i == 60 {
            signal(proc.pid, true);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    proc.exited.load(Ordering::SeqCst)
}

/// Read side for stray callers: is a chat process alive in this pane?
#[tauri::command]
pub fn chat_running(chats: State<'_, Arc<Chats>>, pane: PaneId) -> Option<u64> {
    chats.0.lock().ok()?.get(&pane).map(|p| p.gen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// Spawns the real CLI with the app's exact flags and does the startup
    /// handshake only (`initialize` makes no model call). Opt-in because it
    /// needs `claude` installed and signed in:
    /// `cargo test -p amux-app chat -- --ignored`.
    #[test]
    #[ignore]
    fn real_cli_answers_initialize() {
        let claude = find_claude().expect("claude on this machine");
        let mut cmd = Command::new(claude);
        cmd.args(chat_args(None, None, None)).arg("--no-session-persistence");
        for key in SESSION_IDENTITY_ENV {
            cmd.env_remove(key);
        }
        // The user's hooks load here too: keep them off any live amux. Without
        // a pane id and with a dead socket, `amux notify` just fails quietly.
        cmd.env_remove("AMUX_PANE_ID")
            .env_remove("AMUX_WORKSPACE_ID")
            .env("AMUX_SOCKET", "/nonexistent/amux.sock");
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn");
        let mut stdin = child.stdin.take().unwrap();
        writeln!(
            stdin,
            r#"{{"type":"control_request","request_id":"t1","request":{{"subtype":"initialize"}}}}"#
        )
        .unwrap();
        stdin.flush().unwrap();
        let mut reader = BufReader::new(child.stdout.take().unwrap());
        let mut found = false;
        let mut line = String::new();
        for _ in 0..50 {
            line.clear();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            let v: Value = serde_json::from_str(line.trim()).unwrap_or(Value::Null);
            if v["type"] == "control_response" && v["response"]["request_id"] == "t1" {
                assert!(v["response"]["response"]["models"].is_array());
                found = true;
                break;
            }
        }
        drop(stdin);
        let _ = child.kill();
        let _ = child.wait();
        let mut rest = String::new();
        let _ = reader.read_to_string(&mut rest);
        assert!(found, "no initialize response");
    }

    #[test]
    fn argv_tells_claude_about_diagrams_and_passes_options() {
        let args = chat_args(Some("opus"), Some("manual"), Some(""));
        let at = args.iter().position(|a| a == "--append-system-prompt").unwrap();
        assert!(args[at + 1].contains("```mermaid"));
        assert!(args.windows(2).any(|w| w[0] == "--model" && w[1] == "opus"));
        assert!(args.windows(2).any(|w| w[0] == "--permission-mode" && w[1] == "manual"));
        assert!(!args.iter().any(|a| a == "--resume"), "empty resume adds no flag");
    }

    #[test]
    fn permission_requests_flag_attention_and_interrupts_go_idle() {
        let ask: Value = serde_json::json!({
            "type": "control_request",
            "request_id": "r",
            "request": { "subtype": "can_use_tool", "tool_name": "Bash", "description": "ls" }
        });
        let (kind, _, body) = status_signal(&ask).unwrap();
        assert_eq!(kind, NotifyKind::Attention);
        assert!(body.unwrap().contains("Bash"));
        let interrupted: Value = serde_json::json!({ "type": "result", "subtype": "error_during_execution" });
        assert_eq!(status_signal(&interrupted).unwrap().0, NotifyKind::Idle);
        let ok: Value = serde_json::json!({ "type": "result", "subtype": "success" });
        assert!(status_signal(&ok).is_none());
    }
}
