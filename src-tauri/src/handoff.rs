//! One Claude conversation moving between a pane's terminal and its chat view.
//!
//! What the person chose: switching modes keeps the same session, and never
//! with two processes on it at once (both would append to one transcript and
//! the conversation would fork).
//!   terminal -> chat: `term_claude_link` finds the Claude Code running under
//!       the pane's shell; `term_claude_end` ends it (SIGTERM, only while it
//!       waits for input) and returns once it is gone, so the chat can
//!       `--resume` the same session.
//!   chat -> terminal: `chat_handback` in chat.rs.
//!   `session_history` gives the chat the conversation so far, read from the
//!   session's transcript, to draw above its own turns.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use amux_core::Engine;
use amux_protocol::PaneId;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::State;

use crate::subagents::{self, Link, Subagents};

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn home() -> Result<std::path::PathBuf, String> {
    subagents::claude_home().ok_or_else(|| "no home directory".to_string())
}

/// The Claude Code running under the pane's shell right now, if any.
#[tauri::command]
pub async fn term_claude_link(engine: State<'_, Arc<Engine>>, pane: PaneId) -> Result<Option<Link>, String> {
    let Some(shell) = engine.pane(pane).map_err(err)?.child_pid() else {
        return Ok(None);
    };
    let home = home()?;
    let key = pane.to_string();
    tauri::async_runtime::spawn_blocking(move || subagents::links_of(&home, &[(key.clone(), shell)]).remove(&key))
        .await
        .map_err(err)
}

/// End the Claude Code under the pane's shell so its session can move to the
/// chat. Refused unless it still runs `session` and is waiting for input.
/// Returns once the process is gone.
#[tauri::command]
pub async fn term_claude_end(engine: State<'_, Arc<Engine>>, pane: PaneId, session: String) -> Result<(), String> {
    if !subagents::valid_session(&session) {
        return Err("bad session id".into());
    }
    let shell = engine
        .pane(pane)
        .map_err(err)?
        .child_pid()
        .ok_or("이 pane 의 셸을 찾지 못했습니다")?;
    let home = home()?;
    let key = pane.to_string();
    tauri::async_runtime::spawn_blocking(move || end_claude(&home, &key, shell, &session, Duration::from_secs(5)))
        .await
        .map_err(err)?
}

/// The pid is looked up here, under the pane's shell, never taken from the
/// page: only a Claude Code registered for that pane can be ended.
pub(crate) fn end_claude(home: &Path, key: &str, shell: u32, session: &str, wait: Duration) -> Result<(), String> {
    let link = subagents::links_of(home, &[(key.to_string(), shell)])
        .remove(key)
        .ok_or("터미널에서 도는 Claude 를 찾지 못했습니다")?;
    if link.session != session {
        return Err("터미널의 Claude 가 다른 대화로 바뀌었습니다".into());
    }
    if link.status != "idle" {
        return Err("터미널의 Claude 가 일하는 중입니다".into());
    }
    let Some(start) = subagents::proc_start(link.pid) else {
        return Ok(()); // already gone
    };
    terminate(link.pid)?;
    let deadline = Instant::now() + wait;
    while Instant::now() < deadline {
        if subagents::proc_start(link.pid).as_deref() != Some(start.as_str()) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(format!("터미널의 Claude 가 {}초 안에 끝나지 않았습니다", wait.as_secs()))
}

#[cfg(unix)]
fn terminate(pid: u32) -> Result<(), String> {
    let ok = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .map_err(err)?
        .success();
    if ok {
        Ok(())
    } else {
        Err("터미널의 Claude 에 종료 신호를 보내지 못했습니다".into())
    }
}

#[cfg(not(unix))]
fn terminate(_pid: u32) -> Result<(), String> {
    Err("이 운영체제에서는 지원하지 않습니다".into())
}

// ---- the conversation so far -------------------------------------------------

/// At most this many entries go to the page (the latest ones).
const MAX_ENTRIES: usize = 600;
/// And at most about this many bytes of them.
const MAX_BYTES: usize = 1_500_000;

#[derive(Debug, Serialize, PartialEq)]
pub struct History {
    /// `prompt` (what the person typed), `text`, `tool`, `result`, in order.
    pub entries: Vec<Value>,
    /// Earlier entries left out to keep it small.
    pub omitted: usize,
    /// The conversation was compacted: what came before is only a summary
    /// to Claude now, and is not shown.
    pub compacted: bool,
}

#[tauri::command]
pub async fn session_history(state: State<'_, Subagents>, session: String) -> Result<History, String> {
    if !subagents::valid_session(&session) {
        return Err("bad session id".into());
    }
    let home = home()?;
    let cache = Arc::clone(&state.projects);
    tauri::async_runtime::spawn_blocking(move || {
        let project = subagents::project_of(&cache, &home, &session).ok_or("이 대화의 기록 파일을 찾지 못했습니다")?;
        history_of(&project.join(format!("{session}.jsonl")))
    })
    .await
    .map_err(err)?
}

/// The conversation Claude would resume: from the latest message back along
/// `parentUuid` (a rewind leaves its old branch in the file; a compaction
/// starts a new chain at its boundary), main thread only.
pub(crate) fn history_of(path: &Path) -> Result<History, String> {
    let bytes = std::fs::read(path).map_err(err)?;
    let text = String::from_utf8_lossy(&bytes);
    // uuid -> (parent, is a compaction boundary)
    let mut nodes: HashMap<String, (Option<String>, bool)> = HashMap::new();
    let mut shown: Vec<(String, Vec<Value>)> = Vec::new();
    let mut leaf: Option<String> = None;
    for line in text.lines() {
        // Raw API requests and other bookkeeping lines carry no place in the
        // chain (no `parentUuid`, which every chain line starts with).
        if line.len() > subagents::BIG_LINE && !line[..head_end(line)].contains("\"parentUuid\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let Some(uuid) = v["uuid"].as_str() else { continue };
        let boundary = v["type"].as_str() == Some("system") && v["subtype"].as_str() == Some("compact_boundary");
        nodes.insert(uuid.to_string(), (v["parentUuid"].as_str().map(String::from), boundary));
        if v["isSidechain"].as_bool() == Some(true) {
            continue;
        }
        if matches!(v["type"].as_str(), Some("user" | "assistant")) {
            leaf = Some(uuid.to_string());
            let entries = history_entries(&v);
            if !entries.is_empty() {
                shown.push((uuid.to_string(), entries));
            }
        }
    }
    let mut chain: HashSet<String> = HashSet::new();
    let mut compacted = false;
    let mut cur = leaf;
    while let Some(u) = cur {
        if !chain.insert(u.clone()) {
            break; // a loop: stop rather than spin
        }
        match nodes.get(&u) {
            Some((parent, boundary)) => {
                compacted |= *boundary;
                cur = parent.clone();
            }
            None => break,
        }
    }
    let mut entries: Vec<Value> = shown
        .into_iter()
        .filter(|(u, _)| chain.contains(u))
        .flat_map(|(_, e)| e)
        .collect();
    // Keep the latest ones within both limits.
    let mut keep = 0;
    let mut size = 0;
    for e in entries.iter().rev() {
        size += e.to_string().len();
        if keep >= MAX_ENTRIES || size > MAX_BYTES {
            break;
        }
        keep += 1;
    }
    let omitted = entries.len() - keep;
    entries.drain(..omitted);
    Ok(History { entries, omitted, compacted })
}

fn head_end(line: &str) -> usize {
    let mut end = line.len().min(512);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    end
}

/// What the history shows of one main-thread message: what the person
/// typed (a slash command as its name), what Claude said and called, and the
/// tools' outcomes. Injected text, notifications and summaries are left out.
fn history_entries(v: &Value) -> Vec<Value> {
    if v["isMeta"].as_bool() == Some(true) {
        return Vec::new();
    }
    let ts = v["timestamp"].as_str().unwrap_or("");
    let content = &v["message"]["content"];
    match v["type"].as_str() {
        Some("assistant") => subagents::assistant_entries(content, ts),
        Some("user") if subagents::is_user_prompt_value(v) => {
            let typed = match content {
                Value::String(s) => s.as_str(),
                Value::Array(blocks) => blocks.iter().find_map(|b| b["text"].as_str()).unwrap_or(""),
                _ => "",
            }
            .trim();
            if typed.starts_with("[Request interrupted") {
                return Vec::new();
            }
            let text = slash_command(typed).unwrap_or_else(|| subagents::clip(typed, 6000));
            vec![json!({ "k": "prompt", "text": text, "ts": ts })]
        }
        Some("user") => content
            .as_array()
            .into_iter()
            .flatten()
            .filter(|b| b["type"].as_str() == Some("tool_result"))
            .map(|b| subagents::result_entry(b, ts))
            .collect(),
        _ => Vec::new(),
    }
}

/// `<command-name>/x</command-name>...<command-args>a</command-args>` -> "/x a".
fn slash_command(text: &str) -> Option<String> {
    let inner = |tag: &str| {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");
        let from = text.find(&open)? + open.len();
        let to = text[from..].find(&close)? + from;
        Some(text[from..to].trim().to_string())
    };
    let name = inner("command-name")?;
    let args = inner("command-args").unwrap_or_default();
    Some(if args.is_empty() { name } else { format!("{name} {}", subagents::clip(&args, 300)) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU32, Ordering};

    static SEQ: AtomicU32 = AtomicU32::new(0);

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "amux-handoff-{tag}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn line(uuid: &str, parent: Option<&str>, kind: &str, extra: Value) -> String {
        let mut v = json!({ "parentUuid": parent, "isSidechain": false, "type": kind, "uuid": uuid, "timestamp": "T" });
        for (k, x) in extra.as_object().unwrap() {
            v[k] = x.clone();
        }
        v.to_string()
    }

    #[test]
    fn history_follows_the_branch_claude_resumes() {
        let dir = temp_dir("hist");
        let path = dir.join("s.jsonl");
        let user = |text: &str| json!({ "message": { "role": "user", "content": text }, "origin": { "kind": "human" } });
        let say = |text: &str| json!({ "message": { "content": [{ "type": "text", "text": text }] } });
        let lines = [
            line("a", None, "user", user("첫 질문")),
            line("b", Some("a"), "assistant", say("첫 답")),
            // A rewind: the person went back and asked again from "b".
            line("c", Some("b"), "user", user("버린 질문")),
            line("d", Some("c"), "assistant", say("버린 답")),
            line("e", Some("b"), "user", user("다시 한 질문")),
            r#"{"type":"api-request","data":"x"}"#.to_string(),
            line("f", Some("e"), "assistant", json!({ "message": { "content": [
                { "type": "thinking", "thinking": "..." },
                { "type": "tool_use", "id": "t1", "name": "Read", "input": { "file_path": "/r/a.py" } }
            ] } })),
            line("g", Some("f"), "user", json!({ "message": { "content": [
                { "type": "tool_result", "tool_use_id": "t1", "content": "1\tprint()" }
            ] } })),
            line("h", Some("g"), "user", json!({ "isMeta": true, "message": { "content": "<system-reminder>x</system-reminder>" } })),
            line("i", Some("h"), "user", json!({ "origin": { "kind": "task-notification" }, "message": { "content": "<task-notification>" } })),
            line("j", Some("i"), "user", user("<command-name>/context</command-name>\n<command-message>context</command-message>\n<command-args></command-args>")),
            line("k", Some("j"), "assistant", say("마지막 답")),
            line("z", Some("k"), "assistant", json!({ "isSidechain": true, "message": { "content": [{ "type": "text", "text": "서브에이전트" }] } })),
        ];
        fs::write(&path, lines.join("\n") + "\n").unwrap();
        let h = history_of(&path).unwrap();
        let kinds: Vec<String> = h
            .entries
            .iter()
            .map(|e| format!("{}:{}", e["k"].as_str().unwrap(), e["text"].as_str().or(e["name"].as_str()).unwrap_or("")))
            .collect();
        assert_eq!(
            kinds,
            vec!["prompt:첫 질문", "text:첫 답", "prompt:다시 한 질문", "tool:Read", "result:1\tprint()", "prompt:/context", "text:마지막 답"]
        );
        assert!(!h.compacted);
        assert_eq!(h.omitted, 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_stops_at_a_compaction() {
        let dir = temp_dir("compact");
        let path = dir.join("s.jsonl");
        let lines = [
            line("a", None, "user", json!({ "message": { "content": "옛 질문" } })),
            line("b", Some("a"), "assistant", json!({ "message": { "content": [{ "type": "text", "text": "옛 답" }] } })),
            line("c", None, "system", json!({ "subtype": "compact_boundary", "logicalParentUuid": "b" })),
            line("d", Some("c"), "user", json!({ "isCompactSummary": true, "message": { "content": "요약: ..." } })),
            line("e", Some("d"), "user", json!({ "message": { "content": "요약 뒤 질문" } })),
        ];
        fs::write(&path, lines.join("\n")).unwrap();
        let h = history_of(&path).unwrap();
        assert!(h.compacted);
        assert_eq!(h.entries.len(), 1, "{:?}", h.entries);
        assert_eq!(h.entries[0]["text"], "요약 뒤 질문");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_keeps_the_latest_entries() {
        let dir = temp_dir("limit");
        let path = dir.join("s.jsonl");
        let mut lines = Vec::new();
        let mut parent: Option<String> = None;
        for i in 0..(MAX_ENTRIES + 25) {
            let id = format!("u{i}");
            lines.push(line(&id, parent.as_deref(), "user", json!({ "message": { "content": format!("질문 {i}") } })));
            parent = Some(id);
        }
        fs::write(&path, lines.join("\n")).unwrap();
        let h = history_of(&path).unwrap();
        assert_eq!(h.entries.len(), MAX_ENTRIES);
        assert_eq!(h.omitted, 25);
        assert_eq!(h.entries.last().unwrap()["text"], format!("질문 {}", MAX_ENTRIES + 24));
        let _ = fs::remove_dir_all(&dir);
    }

    /// A registered Claude under a shell is ended only when it is idle and
    /// still runs the expected session; the shell is `sh`, the "Claude" the
    /// `sleep` it runs (SIGTERM ends it).
    #[cfg(target_os = "linux")]
    #[test]
    fn ends_only_the_idle_claude_of_the_expected_session() {
        let mut shell = std::process::Command::new("sh").args(["-c", "sleep 30; true"]).spawn().unwrap();
        let sh = shell.id();
        let mut child = None;
        for _ in 0..100 {
            let kids = fs::read_to_string(format!("/proc/{sh}/task/{sh}/children")).unwrap_or_default();
            if let Some(pid) = kids.split_whitespace().next().and_then(|p| p.parse::<u32>().ok()) {
                child = Some(pid);
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let child = child.expect("sh started sleep");
        let start = subagents::proc_start(child).unwrap();
        let home = temp_dir("end");
        fs::create_dir_all(home.join("sessions")).unwrap();
        let reg = |status: &str| {
            fs::write(
                home.join("sessions").join(format!("{child}.json")),
                format!(r#"{{"pid":{child},"sessionId":"ee9ffb79-0000-0000-0000-00000000000e","procStart":"{start}","status":"{status}","startedAt":5}}"#),
            )
            .unwrap();
        };
        let wait = Duration::from_secs(3);
        reg("busy");
        assert_eq!(
            end_claude(&home, "p", sh, "ee9ffb79-0000-0000-0000-00000000000e", wait),
            Err("터미널의 Claude 가 일하는 중입니다".into())
        );
        reg("idle");
        assert_eq!(
            end_claude(&home, "p", sh, "ee9ffb79-0000-0000-0000-0000000000ff", wait),
            Err("터미널의 Claude 가 다른 대화로 바뀌었습니다".into())
        );
        assert!(subagents::proc_start(child).is_some(), "still running after both refusals");
        assert_eq!(end_claude(&home, "p", sh, "ee9ffb79-0000-0000-0000-00000000000e", wait), Ok(()));
        assert!(subagents::proc_start(child).is_none(), "gone once it returns");
        assert_eq!(
            end_claude(&home, "p", sh, "ee9ffb79-0000-0000-0000-00000000000e", wait),
            Err("터미널에서 도는 Claude 를 찾지 못했습니다".into()),
            "nothing left under the shell"
        );
        let _ = shell.wait();
        let _ = fs::remove_dir_all(&home);
    }

    /// Real data: `AMUX_HIST=<main transcript>` prints the shape of what the
    /// chat would draw (kinds and counts only, never the text).
    #[test]
    #[ignore]
    fn real_history_shape() {
        let Some(path) = std::env::var_os("AMUX_HIST") else { return };
        let t = Instant::now();
        let h = history_of(Path::new(&path)).unwrap();
        let mut kinds: HashMap<String, usize> = HashMap::new();
        for e in &h.entries {
            *kinds.entry(e["k"].as_str().unwrap_or("?").to_string()).or_default() += 1;
        }
        let bytes: usize = h.entries.iter().map(|e| e.to_string().len()).sum();
        println!(
            "HIST entries={} omitted={} compacted={} bytes={} kinds={kinds:?} took={:?}",
            h.entries.len(),
            h.omitted,
            h.compacted,
            bytes,
            t.elapsed()
        );
    }
}
