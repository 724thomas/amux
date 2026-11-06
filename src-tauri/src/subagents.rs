//! The subagents of the Claude Code in a pane, for the column beside it.
//!
//! Claude Code writes everything this needs to disk as it works, so it is all
//! read from there, read-only, for a terminal pane and a chat alike:
//!   <claude home>/sessions/<pid>.json
//!       which session a live Claude process runs (`sessionId`), and when the
//!       process started (`procStart`, the same number as field 22 of
//!       /proc/<pid>/stat, so a stale file or a reused pid is caught)
//!   <claude home>/projects/<project>/<session>.jsonl
//!       the main transcript; only new user prompts are counted from it (the
//!       card of a finished subagent closes at the next one)
//!   <claude home>/projects/<project>/<session>/subagents/agent-<id>.jsonl
//!       each subagent's transcript, one line per message block
//!   <claude home>/projects/<project>/<session>/subagents/agent-<id>.meta.json
//!       its description and type, and `finished: true` once it is done
//!
//! The transcripts also hold raw API requests and attachments (tens of KB a
//! line). Lines are compacted here to what the cards show, strings clipped,
//! so those never cross to the page. The `*.key` files beside the session
//! registry are never opened.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use amux_core::Engine;
use serde::Serialize;
use serde_json::{json, Map, Value};
use tauri::State;

/// At most this much of a transcript is read per call (the page asks again).
const MAX_READ: usize = 4 << 20;
/// A line longer than this is parsed only when its head says it is a message.
pub(crate) const BIG_LINE: usize = 64 << 10;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Claude Code's state directory: `$CLAUDE_CONFIG_DIR`, else `~/.claude`.
pub fn claude_home() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".claude"))
}

fn home() -> Result<PathBuf, String> {
    claude_home().ok_or_else(|| "no home directory".to_string())
}

/// A session id names a directory and a file: a uuid, nothing else.
pub fn valid_session(s: &str) -> bool {
    (8..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

pub fn valid_agent(s: &str) -> bool {
    (1..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

pub(crate) fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

// ---- which Claude runs in which pane ---------------------------------------

/// A live Claude process found under a pane's shell, and the session it runs.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Link {
    pub session: String,
    pub pid: u32,
    /// The registry's status: Claude is working (not waiting for input).
    pub busy: bool,
    /// The registry's status as written ("idle" while it waits for input).
    pub status: String,
    /// When that process started (ms since the epoch): a subagent whose
    /// transcript was last written before this belonged to an earlier run.
    pub started_ms: u64,
}

#[derive(Debug, Clone)]
struct Registered {
    pid: u32,
    session: String,
    status: String,
    started_at: u64,
}

/// (parent pid, start time) of a live process.
#[cfg(target_os = "linux")]
fn proc_stat(pid: u32) -> Option<(u32, String)> {
    let raw = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name (field 2) may hold spaces and parentheses; the fields
    // after it start past the last ')': state, ppid, ... starttime (22nd).
    let rest = raw.get(raw.rfind(')')? + 2..)?;
    let f: Vec<&str> = rest.split_whitespace().collect();
    Some((f.get(1)?.parse().ok()?, f.get(19)?.to_string()))
}

/// Process trees are read from /proc; elsewhere terminal panes go unlinked
/// (a chat knows its session from Claude's own output).
#[cfg(not(target_os = "linux"))]
fn proc_stat(_pid: u32) -> Option<(u32, String)> {
    None
}

/// When a live process started (field 22 of /proc/<pid>/stat), to tell it
/// apart from a later process given the same pid.
pub(crate) fn proc_start(pid: u32) -> Option<String> {
    proc_stat(pid).map(|(_, start)| start)
}

/// `pid` is `ancestor` itself or runs somewhere under it.
fn is_descendant(pid: u32, ancestor: u32) -> bool {
    let mut cur = pid;
    for _ in 0..64 {
        if cur == ancestor {
            return true;
        }
        match proc_stat(cur) {
            Some((ppid, _)) if ppid > 1 && ppid != cur => cur = ppid,
            _ => return false,
        }
    }
    false
}

/// Live Claude processes in the registry; stale files and reused pids are
/// dropped by comparing the recorded start time with the live process's.
fn registry(home: &Path) -> Vec<Registered> {
    let Ok(dir) = fs::read_dir(home.join("sessions")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in dir.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        // Only `<pid>.json`; the `*.key` files beside them are never opened.
        let Some(stem) = name.strip_suffix(".json") else { continue };
        let Ok(pid) = stem.parse::<u32>() else { continue };
        let Some((_, start)) = proc_stat(pid) else { continue };
        let Ok(text) = fs::read_to_string(entry.path()) else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
        let Some(session) = v["sessionId"].as_str().filter(|s| valid_session(s)) else {
            continue;
        };
        let recorded = match &v["procStart"] {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        };
        if recorded.is_some_and(|r| r != start) {
            continue;
        }
        out.push(Registered {
            pid,
            session: session.to_string(),
            status: v["status"].as_str().unwrap_or("").to_string(),
            started_at: v["startedAt"].as_u64().unwrap_or(0),
        });
    }
    out
}

/// For each pane (given as its shell's pid), the newest Claude under it.
pub fn links_of(home: &Path, shells: &[(String, u32)]) -> HashMap<String, Link> {
    let mut best: HashMap<String, (u64, Link)> = HashMap::new();
    for r in registry(home) {
        for (pane, shell) in shells {
            if !is_descendant(r.pid, *shell) {
                continue;
            }
            if best.get(pane).is_some_and(|(t, _)| *t > r.started_at) {
                continue;
            }
            let link = Link {
                session: r.session.clone(),
                pid: r.pid,
                busy: r.status == "busy",
                status: r.status.clone(),
                started_ms: r.started_at,
            };
            best.insert(pane.clone(), (r.started_at, link));
        }
    }
    best.into_iter().map(|(pane, (_, link))| (pane, link)).collect()
}

// ---- a session's files -------------------------------------------------------

/// Session id -> its project directory, found once (sessions do not move).
#[derive(Default)]
pub struct Subagents {
    pub(crate) projects: Arc<Mutex<HashMap<String, PathBuf>>>,
}

fn find_project(home: &Path, session: &str) -> Option<PathBuf> {
    for entry in fs::read_dir(home.join("projects")).ok()?.flatten() {
        let p = entry.path();
        if p.join(format!("{session}.jsonl")).is_file() || p.join(session).is_dir() {
            return Some(p);
        }
    }
    None
}

pub(crate) fn project_of(cache: &Mutex<HashMap<String, PathBuf>>, home: &Path, session: &str) -> Option<PathBuf> {
    if let Some(p) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(session) {
        return Some(p.clone());
    }
    let found = find_project(home, session)?;
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(session.to_string(), found.clone());
    Some(found)
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AgentFile {
    pub id: String,
    pub agent_type: Option<String>,
    pub description: Option<String>,
    /// The main thread's Agent call that started it.
    pub tool_use_id: Option<String>,
    /// `finished: true` in its meta file.
    pub finished: bool,
    /// `stoppedByUser: true` in its meta file: the person stopped it (such a
    /// meta has no `finished`).
    pub stopped: bool,
    pub has_meta: bool,
    pub size: u64,
    pub created_ms: u64,
    pub modified_ms: u64,
}

fn ms(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

pub fn list_agents(project: &Path, session: &str) -> Vec<AgentFile> {
    let dir = project.join(session).join("subagents");
    let Ok(rd) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(id) = name.strip_prefix("agent-").and_then(|r| r.strip_suffix(".jsonl")) else {
            continue;
        };
        if !valid_agent(id) {
            continue;
        }
        let Ok(md) = e.metadata() else { continue };
        let modified = md.modified().map(ms).unwrap_or(0);
        let meta = fs::read_to_string(dir.join(format!("agent-{id}.meta.json")))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok());
        let text = |k: &str| meta.as_ref().and_then(|m| m[k].as_str()).map(|v| clip(v, 300));
        let flag = |k: &str| meta.as_ref().and_then(|m| m[k].as_bool()).unwrap_or(false);
        out.push(AgentFile {
            id: id.to_string(),
            agent_type: text("agentType"),
            description: text("description"),
            tool_use_id: text("toolUseId"),
            finished: flag("finished"),
            stopped: flag("stoppedByUser"),
            has_meta: meta.is_some(),
            size: md.len(),
            created_ms: md.created().map(ms).unwrap_or(modified),
            modified_ms: modified,
        });
    }
    out.sort_by(|a, b| (a.created_ms, &a.id).cmp(&(b.created_ms, &b.id)));
    out
}

struct Lines {
    lines: Vec<String>,
    next: u64,
    size: u64,
    reset: bool,
}

/// Whole lines from byte `from` on (a line still being written waits for its
/// end). A file shorter than `from` was rewritten: start over from 0.
fn read_lines(path: &Path, from: u64, max: usize) -> std::io::Result<Lines> {
    let mut f = fs::File::open(path)?;
    let size = f.metadata()?.len();
    let (from, reset) = if from > size { (0, true) } else { (from, false) };
    if from == size {
        return Ok(Lines { lines: Vec::new(), next: from, size, reset });
    }
    f.seek(SeekFrom::Start(from))?;
    let want = ((size - from) as usize).min(max);
    let mut buf = Vec::with_capacity(want);
    f.take(want as u64).read_to_end(&mut buf)?;
    let end = match buf.iter().rposition(|&b| b == b'\n') {
        Some(i) => i + 1,
        // One line longer than a whole read: step over it.
        None if buf.len() >= max => buf.len(),
        None => 0,
    };
    let lines = buf[..end]
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .map(|l| String::from_utf8_lossy(l).into_owned())
        .collect();
    Ok(Lines { lines, next: from + end as u64, size, reset })
}

/// Whether a long line's head names a message (the top-level `type` comes
/// within its first few hundred bytes).
pub(crate) fn head_is_message(line: &str) -> bool {
    let mut end = line.len().min(4096);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    let head = &line[..end];
    head.contains("\"type\":\"user\"") || head.contains("\"type\":\"assistant\"")
}

fn input_limit(key: &str) -> usize {
    match key {
        "old_string" | "new_string" | "content" | "message" => 8000,
        "prompt" | "description" => 2000,
        _ => 1000,
    }
}

pub(crate) fn compact_input(input: &Value) -> Value {
    let Some(obj) = input.as_object() else {
        return Value::Null;
    };
    let mut out = Map::new();
    for (k, v) in obj {
        let v = match v {
            Value::String(s) => Value::String(clip(s, input_limit(k))),
            Value::Bool(_) | Value::Number(_) | Value::Null => v.clone(),
            other => {
                let s = other.to_string();
                if s.len() <= 1000 {
                    other.clone()
                } else {
                    Value::String(clip(&s, 1000))
                }
            }
        };
        out.insert(k.clone(), v);
    }
    Value::Object(out)
}

fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// One transcript line -> what a subagent card shows of it (often nothing):
/// `prompt` (a message to it, the first one being its task), `text` (what it
/// says), `tool` (a call, input clipped), `result` (a call's outcome).
pub fn compact_line(line: &str) -> Vec<Value> {
    if line.len() > BIG_LINE && !head_is_message(line) {
        return Vec::new();
    }
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    if v["isMeta"].as_bool() == Some(true) {
        return Vec::new();
    }
    let ts = v["timestamp"].as_str().unwrap_or("");
    let content = &v["message"]["content"];
    let mut out = Vec::new();
    match v["type"].as_str() {
        Some("assistant") => out = assistant_entries(content, ts),
        Some("user") => match content {
            Value::String(s) => {
                let t = s.trim();
                if !t.is_empty() {
                    out.push(json!({ "k": "prompt", "text": clip(t, 6000), "ts": ts }));
                }
            }
            Value::Array(blocks) => {
                for b in blocks {
                    match b["type"].as_str() {
                        Some("tool_result") => out.push(result_entry(b, ts)),
                        Some("text") => {
                            let t = b["text"].as_str().unwrap_or("").trim();
                            if !t.is_empty() {
                                out.push(json!({ "k": "prompt", "text": clip(t, 6000), "ts": ts }));
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        },
        _ => {}
    }
    out
}

/// What Claude says and calls in one assistant message (thinking is left out).
pub(crate) fn assistant_entries(content: &Value, ts: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for b in content.as_array().into_iter().flatten() {
        match b["type"].as_str() {
            Some("text") => {
                let t = b["text"].as_str().unwrap_or("").trim();
                if !t.is_empty() {
                    out.push(json!({ "k": "text", "text": clip(t, 6000), "ts": ts }));
                }
            }
            Some("tool_use") => out.push(json!({
                "k": "tool",
                "id": b["id"].as_str().unwrap_or(""),
                "name": b["name"].as_str().unwrap_or(""),
                "input": compact_input(&b["input"]),
                "ts": ts,
            })),
            _ => {} // thinking
        }
    }
    out
}

/// A tool's outcome, its text clipped.
pub(crate) fn result_entry(b: &Value, ts: &str) -> Value {
    json!({
        "k": "result",
        "id": b["tool_use_id"].as_str().unwrap_or(""),
        "err": b["is_error"].as_bool().unwrap_or(false),
        "text": clip(result_text(&b["content"]).trim(), 600),
        "ts": ts,
    })
}

/// A line of the main transcript that the person typed (or a slash command):
/// not a tool result, not text Claude Code injected (`isMeta`, the echo of a
/// local command's output), not a task notification or a subagent's report.
/// The terminal Claude marks typed lines `origin: {kind: "human"}` and every
/// injected one with another kind; `claude -p` (the chat) writes no origin.
pub fn is_user_prompt(line: &str) -> bool {
    if !line.contains("\"type\":\"user\"") {
        return false;
    }
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    is_user_prompt_value(&v)
}

/// `is_user_prompt` on a line already parsed.
pub(crate) fn is_user_prompt_value(v: &Value) -> bool {
    if v["type"].as_str() != Some("user")
        || v["isMeta"].as_bool() == Some(true)
        || v["isSidechain"].as_bool() == Some(true)
        || v["isCompactSummary"].as_bool() == Some(true)
        || v["isVisibleInTranscriptOnly"].as_bool() == Some(true)
        || !(v["origin"].is_null() || v["origin"]["kind"].as_str() == Some("human"))
    {
        return false;
    }
    let typed = |t: &str| {
        let t = t.trim_start();
        !t.is_empty() && !t.starts_with("<local-command-std")
    };
    match &v["message"]["content"] {
        Value::String(s) => typed(s),
        Value::Array(blocks) => {
            !blocks.iter().any(|b| b["type"].as_str() == Some("tool_result"))
                && blocks.iter().filter_map(|b| b["text"].as_str()).next().is_some_and(typed)
        }
        _ => false,
    }
}

// ---- commands ---------------------------------------------------------------

/// Pane id -> the Claude Code running in it (terminal panes; a chat knows its
/// own session).
#[tauri::command]
pub async fn claude_links(engine: State<'_, Arc<Engine>>) -> Result<HashMap<String, Link>, String> {
    let shells: Vec<(String, u32)> = engine
        .snapshot()
        .panes
        .iter()
        .filter_map(|p| Some((p.id.to_string(), engine.pane(p.id).ok()?.child_pid()?)))
        .collect();
    let home = home()?;
    tauri::async_runtime::spawn_blocking(move || links_of(&home, &shells)).await.map_err(err)
}

#[tauri::command]
pub async fn subagents_list(state: State<'_, Subagents>, session: String) -> Result<Vec<AgentFile>, String> {
    if !valid_session(&session) {
        return Err("bad session id".into());
    }
    let home = home()?;
    let cache = Arc::clone(&state.projects);
    tauri::async_runtime::spawn_blocking(move || match project_of(&cache, &home, &session) {
        Some(project) => list_agents(&project, &session),
        None => Vec::new(),
    })
    .await
    .map_err(err)
}

#[derive(Debug, Serialize)]
pub struct Chunk {
    pub items: Vec<Value>,
    /// Where the next read starts.
    pub next: u64,
    pub size: u64,
    /// The file was rewritten: everything read before is void.
    pub reset: bool,
}

#[tauri::command]
pub async fn subagent_read(
    state: State<'_, Subagents>,
    session: String,
    agent: String,
    from: u64,
) -> Result<Chunk, String> {
    if !valid_session(&session) || !valid_agent(&agent) {
        return Err("bad id".into());
    }
    let home = home()?;
    let cache = Arc::clone(&state.projects);
    tauri::async_runtime::spawn_blocking(move || {
        let project = project_of(&cache, &home, &session).ok_or("no such session")?;
        let path = project.join(&session).join("subagents").join(format!("agent-{agent}.jsonl"));
        let l = read_lines(&path, from, MAX_READ).map_err(err)?;
        let items = l.lines.iter().flat_map(|line| compact_line(line)).collect();
        Ok(Chunk { items, next: l.next, size: l.size, reset: l.reset })
    })
    .await
    .map_err(err)?
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Prompts {
    /// User prompts in the main transcript between `from` and `next`.
    pub count: u32,
    pub next: u64,
}

/// Count the person's prompts added to the main transcript since `from`;
/// without `from`, just say where its end is (nothing before counts).
#[tauri::command]
pub async fn session_prompts(
    state: State<'_, Subagents>,
    session: String,
    from: Option<u64>,
) -> Result<Prompts, String> {
    if !valid_session(&session) {
        return Err("bad session id".into());
    }
    let home = home()?;
    let cache = Arc::clone(&state.projects);
    tauri::async_runtime::spawn_blocking(move || {
        let Some(project) = project_of(&cache, &home, &session) else {
            return Ok(Prompts { count: 0, next: 0 });
        };
        prompts_since(&project.join(format!("{session}.jsonl")), from)
    })
    .await
    .map_err(err)?
}

fn prompts_since(path: &Path, from: Option<u64>) -> Result<Prompts, String> {
    let size = match fs::metadata(path) {
        Ok(m) => m.len(),
        Err(_) => return Ok(Prompts { count: 0, next: 0 }),
    };
    let Some(from) = from.filter(|f| *f <= size) else {
        return Ok(Prompts { count: 0, next: size });
    };
    let l = read_lines(path, from, MAX_READ).map_err(err)?;
    let count = l.lines.iter().filter(|line| is_user_prompt(line)).count() as u32;
    Ok(Prompts { count, next: l.next })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicU32, Ordering};

    static SEQ: AtomicU32 = AtomicU32::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "amux-subagents-{tag}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ids_name_nothing_but_themselves() {
        assert!(valid_session("ee9ffb79-9ac0-4d68-8db2-114a3914d5a2"));
        assert!(!valid_session("../../etc"));
        assert!(!valid_session("ee9ffb79/9ac0"));
        assert!(valid_agent("a0b23259b6e6adaa0"));
        assert!(!valid_agent("a0b2/../x"));
        assert!(!valid_agent(""));
    }

    #[test]
    fn lines_become_what_the_card_shows() {
        let task = r#"{"parentUuid":null,"isSidechain":true,"agentId":"a1","type":"user","message":{"role":"user","content":"README 끝에 줄을 추가해 줘"},"timestamp":"T0"}"#;
        assert_eq!(compact_line(task), vec![json!({ "k": "prompt", "text": "README 끝에 줄을 추가해 줘", "ts": "T0" })]);

        let meta = r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"<system-reminder>x</system-reminder>"}}"#;
        assert!(compact_line(meta).is_empty(), "text Claude Code injects is not shown");
        let attachment = r#"{"type":"attachment","attachment":{"type":"x"}}"#;
        assert!(compact_line(attachment).is_empty());
        let thinking = r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"hmm"}]}}"#;
        assert!(compact_line(thinking).is_empty());

        let edit = r#"{"type":"assistant","timestamp":"T1","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"Edit","input":{"file_path":"/r/README.md","old_string":"a","new_string":"a\nb","replace_all":false}}]}}"#;
        assert_eq!(
            compact_line(edit),
            vec![json!({ "k": "tool", "id": "toolu_1", "name": "Edit", "ts": "T1",
                "input": { "file_path": "/r/README.md", "old_string": "a", "new_string": "a\nb", "replace_all": false } })]
        );

        let result = r#"{"type":"user","timestamp":"T2","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":[{"type":"text","text":"ok"}],"is_error":false}]}}"#;
        assert_eq!(compact_line(result), vec![json!({ "k": "result", "id": "toolu_1", "err": false, "text": "ok", "ts": "T2" })]);
    }

    #[test]
    fn long_strings_and_lines_are_cut() {
        let long = "가".repeat(5000); // 15000 bytes
        let line = json!({ "type": "assistant", "message": { "content": [
            { "type": "tool_use", "id": "t", "name": "Write", "input": { "file_path": "f", "content": long } }
        ] } })
        .to_string();
        let out = compact_line(&line);
        let content = out[0]["input"]["content"].as_str().unwrap();
        assert!(content.len() <= 8000 + "…".len() && content.ends_with('…'));

        // A huge line that is not a message is skipped without parsing it.
        let blob = format!(r#"{{"type":"api-request-blob","data":"{}"}}"#, "x".repeat(BIG_LINE + 10));
        assert!(compact_line(&blob).is_empty());
    }

    #[test]
    fn only_what_the_person_typed_counts_as_a_prompt() {
        let typed = r#"{"type":"user","isSidechain":false,"message":{"role":"user","content":"다음 일 해 줘"}}"#;
        assert!(is_user_prompt(typed), "claude -p (the chat) writes no origin");
        let tui = r#"{"type":"user","origin":{"kind":"human"},"message":{"role":"user","content":"다음 일 해 줘"}}"#;
        assert!(is_user_prompt(tui), "the terminal Claude marks what the person typed");
        let slash = r#"{"type":"user","message":{"content":"<command-name>/context</command-name>"}}"#;
        assert!(is_user_prompt(slash), "a slash command is typed too");
        let echo = r#"{"type":"user","message":{"content":"<local-command-stdout>out</local-command-stdout>"}}"#;
        assert!(!is_user_prompt(echo), "the echo of its output is not");
        let blocks = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"이미지와 함께"}]}}"#;
        assert!(is_user_prompt(blocks));
        let tool = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t","content":"x"}]}}"#;
        assert!(!is_user_prompt(tool));
        let meta = r#"{"type":"user","isMeta":true,"message":{"content":"<system-reminder>"}}"#;
        assert!(!is_user_prompt(meta));
        let note = r#"{"type":"user","origin":{"kind":"task-notification"},"message":{"content":"<task-notification>"}}"#;
        assert!(!is_user_prompt(note));
        let report = r#"{"type":"user","isMeta":true,"origin":{"kind":"peer"},"message":{"content":"Another Claude session sent a message"}}"#;
        assert!(!is_user_prompt(report));
        assert!(!is_user_prompt(r#"{"type":"assistant","message":{"content":"x"}}"#));
    }

    #[test]
    fn reads_whole_lines_from_an_offset_and_starts_over_on_a_rewrite() {
        let dir = temp_dir("read");
        let path = dir.join("t.jsonl");
        fs::write(&path, "one\ntwo\nthr").unwrap();
        let l = read_lines(&path, 0, MAX_READ).unwrap();
        assert_eq!(l.lines, vec!["one", "two"], "the line being written waits");
        assert_eq!(l.next, 8);
        let l2 = read_lines(&path, l.next, MAX_READ).unwrap();
        assert!(l2.lines.is_empty());
        assert_eq!(l2.next, 8);
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"ee\n").unwrap();
        let l3 = read_lines(&path, l2.next, MAX_READ).unwrap();
        assert_eq!(l3.lines, vec!["three"]);
        fs::write(&path, "new\n").unwrap();
        let l4 = read_lines(&path, l3.next, MAX_READ).unwrap();
        assert!(l4.reset);
        assert_eq!(l4.lines, vec!["new"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prompts_count_from_the_end_on() {
        let dir = temp_dir("prompts");
        let path = dir.join("s.jsonl");
        let typed = r#"{"type":"user","message":{"role":"user","content":"하나"}}"#;
        let tool = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t","content":"x"}]}}"#;
        fs::write(&path, format!("{typed}\n")).unwrap();
        let start = prompts_since(&path, None).unwrap();
        assert_eq!(start.count, 0, "what was there before does not count");
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(f, "{tool}\n{typed}").unwrap();
        let after = prompts_since(&path, Some(start.next)).unwrap();
        assert_eq!(after.count, 1);
        assert_eq!(prompts_since(&path, Some(after.next)).unwrap().count, 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn lists_agents_with_their_meta() {
        let dir = temp_dir("list");
        let sub = dir.join("proj").join("s-1234567").join("subagents");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("agent-a1.jsonl"), "{}\n").unwrap();
        fs::write(
            sub.join("agent-a1.meta.json"),
            r#"{"agentType":"general-purpose","description":"README 고치기","toolUseId":"toolu_9","finished":true}"#,
        )
        .unwrap();
        fs::write(sub.join("agent-b2.jsonl"), "").unwrap();
        fs::write(sub.join("agent-c3.jsonl"), "{}\n").unwrap();
        // What Claude Code leaves of one the person stopped: no `finished`.
        fs::write(
            sub.join("agent-c3.meta.json"),
            r#"{"agentType":"Explore","description":"찾기","toolUseId":"toolu_8","stoppedByUser":true}"#,
        )
        .unwrap();
        fs::write(sub.join("agent-..x.jsonl"), "").unwrap();
        fs::write(sub.join("notes.txt"), "").unwrap();
        let mut agents = list_agents(&dir.join("proj"), "s-1234567");
        agents.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(agents.len(), 3, "{agents:?}");
        assert_eq!(agents[0].id, "a1");
        assert!(agents[0].finished && !agents[0].stopped && agents[0].has_meta);
        assert_eq!(agents[0].description.as_deref(), Some("README 고치기"));
        assert_eq!(agents[0].tool_use_id.as_deref(), Some("toolu_9"));
        assert!(!agents[1].finished && !agents[1].stopped && !agents[1].has_meta, "still running, no meta yet");
        assert!(!agents[2].finished && agents[2].stopped, "stopped by the person");
        let cache = Mutex::new(HashMap::new());
        fs::create_dir_all(dir.join("projects")).unwrap();
        fs::rename(dir.join("proj"), dir.join("projects").join("p")).unwrap();
        assert_eq!(project_of(&cache, &dir, "s-1234567"), Some(dir.join("projects").join("p")));
        assert_eq!(project_of(&cache, &dir, "s-7654321"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    /// A Claude found under a pane's shell, by a real process tree: the shell
    /// is `sh`, the Claude is the `sleep` it runs.
    #[cfg(target_os = "linux")]
    #[test]
    fn links_a_process_registered_under_a_shell() {
        let mut shell = std::process::Command::new("sh").args(["-c", "sleep 30; true"]).spawn().unwrap();
        let sh = shell.id();
        let mut child = None;
        for _ in 0..100 {
            let kids = fs::read_to_string(format!("/proc/{sh}/task/{sh}/children")).unwrap_or_default();
            if let Some(pid) = kids.split_whitespace().next().and_then(|p| p.parse::<u32>().ok()) {
                child = Some(pid);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let child = child.expect("sh started sleep");
        let (_, start) = proc_stat(child).unwrap();
        let home = temp_dir("links");
        fs::create_dir_all(home.join("sessions")).unwrap();
        let reg = |pid: u32, start: &str, sid: &str| {
            fs::write(
                home.join("sessions").join(format!("{pid}.json")),
                format!(r#"{{"pid":{pid},"sessionId":"{sid}","procStart":"{start}","status":"busy","startedAt":5}}"#),
            )
            .unwrap();
        };
        reg(child, &start, "ee9ffb79-0000-0000-0000-000000000001");
        // A stale file: the pid is alive (this test) but started at another time.
        reg(std::process::id(), "1", "ee9ffb79-0000-0000-0000-000000000002");
        fs::write(home.join("sessions").join(format!("{child}.secret.key")), "never read").unwrap();

        let links = links_of(&home, &[("pane-a".into(), sh), ("pane-b".into(), 1_999_999)]);
        assert_eq!(links.len(), 1, "{links:?}");
        let link = &links["pane-a"];
        assert_eq!(link.session, "ee9ffb79-0000-0000-0000-000000000001");
        assert_eq!(link.pid, child);
        assert!(link.busy);
        assert_eq!(link.started_ms, 5);
        assert!(is_descendant(child, sh) && !is_descendant(sh, child));

        let _ = shell.kill();
        let _ = shell.wait();
        let _ = fs::remove_dir_all(&home);
    }

    /// Real data: `AMUX_SUB_IN=<agent jsonl> AMUX_SUB_OUT=<json>` writes the
    /// compacted items, for the page's tests to read exactly what this sends.
    #[test]
    #[ignore]
    fn dump_compacted_transcript() {
        let (Some(input), Some(output)) = (std::env::var_os("AMUX_SUB_IN"), std::env::var_os("AMUX_SUB_OUT")) else {
            return;
        };
        let l = read_lines(Path::new(&input), 0, usize::MAX >> 1).unwrap();
        let items: Vec<Value> = l.lines.iter().flat_map(|line| compact_line(line)).collect();
        fs::write(output, serde_json::to_string_pretty(&items).unwrap()).unwrap();
    }

    /// Real data: `AMUX_SUB_MAIN=<main transcript>` prints how many of its
    /// lines count as the person's prompts, and only the leading tag names
    /// of counted ones that start with `<` (never their text).
    #[test]
    #[ignore]
    fn count_real_prompts() {
        let Some(path) = std::env::var_os("AMUX_SUB_MAIN") else {
            return;
        };
        let l = read_lines(Path::new(&path), 0, usize::MAX >> 1).unwrap();
        let mut users = 0;
        let mut prompts = 0;
        let mut tags: HashMap<String, u32> = HashMap::new();
        let mut origins: HashMap<String, u32> = HashMap::new();
        for line in &l.lines {
            let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
            if v["type"].as_str() != Some("user") {
                continue;
            }
            users += 1;
            let origin = v["origin"]["kind"].as_str().unwrap_or("-").to_string();
            let meta = v["isMeta"].as_bool() == Some(true);
            *origins.entry(format!("origin={origin} meta={meta}")).or_default() += 1;
            if !is_user_prompt(line) {
                continue;
            }
            prompts += 1;
            let text = match &v["message"]["content"] {
                Value::String(s) => s.trim_start().to_string(),
                Value::Array(b) => b.iter().find_map(|x| x["text"].as_str()).unwrap_or("").trim_start().to_string(),
                _ => String::new(),
            };
            if let Some(rest) = text.strip_prefix('<') {
                let tag: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
                *tags.entry(tag).or_default() += 1;
            }
        }
        println!("LINES {} USER {users} PROMPTS {prompts}", l.lines.len());
        println!("ORIGINS {origins:?}");
        println!("PROMPT_TAGS {tags:?}");
    }

    /// Real data: `AMUX_SUB_SHELL=<a pane shell's pid>` prints the Claude
    /// session found under it in the real registry.
    #[test]
    #[ignore]
    fn link_a_real_shell() {
        let Some(shell) = std::env::var("AMUX_SUB_SHELL").ok().and_then(|s| s.parse::<u32>().ok()) else {
            return;
        };
        let links = links_of(&claude_home().unwrap(), &[("pane".into(), shell)]);
        println!("LINK {:?}", links.get("pane"));
    }
}
