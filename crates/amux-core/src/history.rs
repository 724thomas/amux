//! Past Claude Code conversations, searchable.
//!
//! Claude Code writes every conversation to
//! `~/.claude/projects/<encoded project path>/<session id>.jsonl`, one JSON
//! event per line. Only a small part of that is conversation — what the person
//! typed and what Claude answered. Most of the bytes are tool traffic (file
//! contents, command output), injected context (`<system-reminder>`, task
//! notifications) and bookkeeping lines, and all of that is skipped here.
//!
//! The index lives in memory for the life of the app. A transcript only ever
//! grows while its session is live, so a refresh stats every file and re-reads
//! just the bytes appended since the last pass; the first pass reads
//! everything once.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::de::{self, Deserializer, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};

/// Where Claude Code keeps its transcripts (`~/.claude/projects`).
pub fn projects_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".claude").join("projects"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

/// One readable turn fragment: something the person typed, or a text block of
/// Claude's answer.
#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub role: Role,
    /// ISO-8601 UTC, as Claude Code writes it.
    pub ts: Option<String>,
    pub text: String,
}

/// One conversation that matched a search (or, for an empty query, one of the
/// most recent conversations).
#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    pub session: String,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub branch: Option<String>,
    pub last_ts: Option<String>,
    /// How many messages matched every term.
    pub matches: usize,
    /// Index (into the conversation's messages) of the first match.
    pub first_match: Option<usize>,
    /// The first match with a little context, on one line.
    pub snippet: Option<String>,
}

/// A whole conversation, for the read-only viewer.
#[derive(Debug, Clone, Serialize)]
pub struct Conversation {
    pub session: String,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub branch: Option<String>,
    pub messages: Vec<Message>,
}

#[derive(Default)]
pub struct HistoryIndex {
    transcripts: HashMap<PathBuf, Transcript>,
}

struct Transcript {
    session: String,
    /// Title the person gave the session (`/rename`), which wins over Claude's.
    custom_title: Option<String>,
    /// Title Claude Code generated for the session.
    ai_title: Option<String>,
    first_prompt: Option<String>,
    cwd: Option<String>,
    branch: Option<String>,
    last_ts: Option<String>,
    messages: Vec<Message>,
    /// Lower-cased copy of each message, same order, for matching.
    folded: Vec<String>,
    /// File size and mtime at the last read, to skip unchanged files.
    len: u64,
    modified: Option<SystemTime>,
    /// Bytes consumed so far — always the end of a complete line.
    offset: u64,
}

impl Transcript {
    fn new(session: String) -> Self {
        Self {
            session,
            custom_title: None,
            ai_title: None,
            first_prompt: None,
            cwd: None,
            branch: None,
            last_ts: None,
            messages: Vec::new(),
            folded: Vec::new(),
            len: 0,
            modified: None,
            offset: 0,
        }
    }

    fn title(&self) -> Option<String> {
        self.custom_title.clone().or_else(|| self.ai_title.clone()).or_else(|| self.first_prompt.clone())
    }

    /// Read whatever was appended since `offset`. A line still being written
    /// (no newline yet) is left for the next pass.
    fn read_new(&mut self, path: &Path) -> std::io::Result<()> {
        let mut file = File::open(path)?;
        file.seek(SeekFrom::Start(self.offset))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let Some(end) = bytes.iter().rposition(|&b| b == b'\n') else { return Ok(()) };
        for line in bytes[..end].split(|&b| b == b'\n') {
            self.ingest(line);
        }
        self.offset += end as u64 + 1;
        Ok(())
    }

    fn ingest(&mut self, line: &[u8]) {
        let Ok(line) = std::str::from_utf8(line) else { return };
        // Cheap substring checks first: only the few line kinds used below are
        // worth a JSON parse, and tool results — the bulk of every transcript —
        // are never conversation.
        let wanted = ["\"type\":\"user\"", "\"type\":\"assistant\"", "\"type\":\"ai-title\"", "\"type\":\"custom-title\""];
        if !wanted.iter().any(|w| line.contains(w)) || line.contains("\"tool_use_id\"") {
            return;
        }
        let Ok(event) = serde_json::from_str::<Event>(line) else { return };
        match event.kind.as_deref() {
            Some("ai-title") => {
                if let Some(title) = event.ai_title.filter(|t| !t.trim().is_empty()) {
                    self.ai_title = Some(title);
                }
            }
            Some("custom-title") => {
                if let Some(title) = event.custom_title.filter(|t| !t.trim().is_empty()) {
                    self.custom_title = Some(title);
                }
            }
            Some(kind @ ("user" | "assistant")) => {
                if event.is_meta || event.is_sidechain || event.is_compact_summary {
                    return;
                }
                let role = if kind == "user" { Role::User } else { Role::Assistant };
                if event.cwd.is_some() {
                    self.cwd = event.cwd;
                }
                if event.git_branch.is_some() {
                    self.branch = event.git_branch;
                }
                if event.timestamp.is_some() && event.timestamp > self.last_ts {
                    self.last_ts = event.timestamp.clone();
                }
                let Some(message) = event.message else { return };
                for text in message.content.0 {
                    let Some(text) = readable(role, &text) else { continue };
                    if role == Role::User && self.first_prompt.is_none() && !text.starts_with('/') {
                        self.first_prompt = Some(first_line(&text, 120));
                    }
                    self.folded.push(fold(&text));
                    self.messages.push(Message { role, ts: event.timestamp.clone(), text });
                }
            }
            _ => {}
        }
    }

    fn hit(&self, matches: usize, first_match: Option<usize>, terms: &[String]) -> Hit {
        let snippet = match (first_match, terms.first()) {
            (Some(i), Some(term)) => Some(snippet(&self.messages[i].text, &self.folded[i], term)),
            _ => None,
        };
        Hit {
            session: self.session.clone(),
            title: self.title(),
            cwd: self.cwd.clone(),
            branch: self.branch.clone(),
            last_ts: self.last_ts.clone(),
            matches,
            first_match,
            snippet,
        }
    }
}

impl HistoryIndex {
    /// Bring the index up to date with `root` (`~/.claude/projects`): read new
    /// transcripts, read what grew in the known ones, forget the deleted ones.
    /// Only the transcripts directly inside each project directory count;
    /// subagent transcripts live one level deeper and are not conversations
    /// the person had.
    pub fn refresh(&mut self, root: &Path) {
        let mut seen = HashSet::new();
        for dir in fs::read_dir(root).into_iter().flatten().flatten() {
            for file in fs::read_dir(dir.path()).into_iter().flatten().flatten() {
                let path = file.path();
                if path.extension().is_none_or(|e| e != "jsonl") {
                    continue;
                }
                let Ok(meta) = file.metadata() else { continue };
                if !meta.is_file() {
                    continue;
                }
                seen.insert(path.clone());
                let (len, modified) = (meta.len(), meta.modified().ok());
                let current = self.transcripts.get(&path);
                if current.is_some_and(|t| t.len == len && t.modified == modified) {
                    continue;
                }
                // Grown (or merely touched): continue from where the last read
                // stopped. Shrunk: it was rewritten, so start over.
                if current.is_none_or(|t| len < t.offset) {
                    let session = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    self.transcripts.insert(path.clone(), Transcript::new(session));
                }
                let transcript = self.transcripts.get_mut(&path).expect("inserted above");
                if transcript.read_new(&path).is_ok() {
                    transcript.len = len;
                    transcript.modified = modified;
                }
            }
        }
        self.transcripts.retain(|path, _| seen.contains(path));
    }

    /// Conversations with at least one message containing every term of
    /// `query` (case-insensitive substrings), most recent first. An empty query
    /// lists the most recent conversations.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Hit> {
        let terms: Vec<String> = query.split_whitespace().map(fold).collect();
        let mut hits: Vec<Hit> = Vec::new();
        for transcript in self.transcripts.values() {
            if transcript.messages.is_empty() {
                continue;
            }
            if terms.is_empty() {
                hits.push(transcript.hit(0, None, &terms));
                continue;
            }
            let mut matches = 0;
            let mut first = None;
            for (i, folded) in transcript.folded.iter().enumerate() {
                if terms.iter().all(|term| folded.contains(term.as_str())) {
                    matches += 1;
                    first.get_or_insert(i);
                }
            }
            if matches > 0 {
                hits.push(transcript.hit(matches, first, &terms));
            }
        }
        hits.sort_by(|a, b| b.last_ts.cmp(&a.last_ts));
        hits.truncate(limit);
        hits
    }

    /// The whole conversation of one session, if it is in the index.
    pub fn conversation(&self, session: &str) -> Option<Conversation> {
        let transcript = self.transcripts.values().find(|t| t.session == session)?;
        Some(Conversation {
            session: transcript.session.clone(),
            title: transcript.title(),
            cwd: transcript.cwd.clone(),
            branch: transcript.branch.clone(),
            messages: transcript.messages.clone(),
        })
    }
}

/// What a text block reads as in the conversation, or `None` when it is not
/// something a person said or read: context Claude Code injects into the user
/// turn (reminders, task notifications, local command output) is dropped, and
/// slash commands and `!` shell lines are shown the way they were typed.
fn readable(role: Role, text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if role == Role::Assistant {
        return Some(text.to_owned());
    }
    const INJECTED: [&str; 8] = [
        "<system-reminder>",
        "<task-notification>",
        "<local-command-caveat>",
        "<local-command-stdout>",
        "<local-command-stderr>",
        "<bash-stdout>",
        "<bash-stderr>",
        "Caveat:",
    ];
    if INJECTED.iter().any(|prefix| text.starts_with(prefix)) {
        return None;
    }
    if text.starts_with("<command-name>") || text.starts_with("<command-message>") {
        let name = between(text, "<command-name>", "</command-name>")?;
        let args = between(text, "<command-args>", "</command-args>").unwrap_or("");
        return Some(format!("{name} {args}").trim().to_owned());
    }
    if let Some(command) = between(text, "<bash-input>", "</bash-input>") {
        return Some(format!("! {}", command.trim()));
    }
    Some(text.to_owned())
}

fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let end = text[start..].find(close)? + start;
    Some(text[start..end].trim())
}

/// Lower-case one character at a time so the folded string keeps exactly one
/// char per original char — which is what lets a match found in the folded
/// copy be cut out of the original for the snippet.
fn fold(text: &str) -> String {
    text.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect()
}

fn first_line(text: &str, max_chars: usize) -> String {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    if line.chars().count() > max_chars {
        format!("{}…", line.chars().take(max_chars).collect::<String>())
    } else {
        line.to_owned()
    }
}

/// The match with some context either side, whitespace collapsed to one line.
fn snippet(text: &str, folded: &str, term: &str) -> String {
    const BEFORE: usize = 40;
    const AFTER: usize = 100;
    let Some(byte_at) = folded.find(term) else { return first_line(text, BEFORE + AFTER) };
    let at = folded[..byte_at].chars().count();
    let start = at.saturating_sub(BEFORE);
    let total = text.chars().count();
    let end = (at + term.chars().count() + AFTER).min(total);
    let body: String = text.chars().skip(start).take(end - start).collect();
    let body = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let lead = if start > 0 { "…" } else { "" };
    let tail = if end < total { "…" } else { "" };
    format!("{lead}{body}{tail}")
}

// ---------------------------------------------------------------------------
// The slice of a transcript line this module reads
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Event {
    #[serde(rename = "type")]
    kind: Option<String>,
    message: Option<EventMessage>,
    timestamp: Option<String>,
    cwd: Option<String>,
    #[serde(rename = "gitBranch")]
    git_branch: Option<String>,
    #[serde(rename = "aiTitle")]
    ai_title: Option<String>,
    #[serde(rename = "customTitle")]
    custom_title: Option<String>,
    #[serde(rename = "isMeta", default)]
    is_meta: bool,
    #[serde(rename = "isSidechain", default)]
    is_sidechain: bool,
    #[serde(rename = "isCompactSummary", default)]
    is_compact_summary: bool,
}

#[derive(Deserialize)]
struct EventMessage {
    #[serde(default)]
    content: Texts,
}

/// The readable text of a message's `content`, which is either a plain string
/// or an array of blocks of which only `text` blocks count. Deserialized with a
/// visitor rather than an untagged enum so the (often huge) tool and thinking
/// blocks are skipped in place instead of being buffered first.
#[derive(Default)]
struct Texts(Vec<String>);

impl<'de> Deserialize<'de> for Texts {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct TextsVisitor;

        impl<'de> Visitor<'de> for TextsVisitor {
            type Value = Texts;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a string or an array of content blocks")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Texts, E> {
                Ok(Texts(vec![v.to_owned()]))
            }

            fn visit_string<E: de::Error>(self, v: String) -> Result<Texts, E> {
                Ok(Texts(vec![v]))
            }

            fn visit_unit<E: de::Error>(self) -> Result<Texts, E> {
                Ok(Texts::default())
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Texts, A::Error> {
                let mut texts = Vec::new();
                while let Some(block) = seq.next_element::<Block>()? {
                    if block.kind.as_deref() == Some("text") {
                        texts.extend(block.text);
                    }
                }
                Ok(Texts(texts))
            }
        }

        deserializer.deserialize_any(TextsVisitor)
    }
}

#[derive(Deserialize)]
struct Block {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("amux-history-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("-home-me-proj")).unwrap();
        dir
    }

    fn append(path: &Path, lines: &[&str]) {
        let mut file = fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
    }

    const PROMPT: &str = r#"{"type":"user","message":{"role":"user","content":"worktree 를 넣을까?"},"timestamp":"2026-09-21T08:00:00.000Z","cwd":"/home/me/proj","gitBranch":"main"}"#;
    const ANSWER: &str = r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"hmm"},{"type":"text","text":"Orca 는 작업마다 Worktree 를 만듭니다."},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]},"timestamp":"2026-09-21T08:00:05.000Z"}"#;
    const TOOL_RESULT: &str = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"worktree worktree"}]},"timestamp":"2026-09-21T08:00:06.000Z"}"#;
    const REMINDER: &str = r#"{"type":"user","message":{"content":[{"type":"text","text":"<system-reminder>worktree</system-reminder>"}]},"timestamp":"2026-09-21T08:00:07.000Z"}"#;
    const META: &str = r#"{"type":"user","isMeta":true,"message":{"content":"worktree meta"},"timestamp":"2026-09-21T08:00:08.000Z"}"#;
    const TITLE: &str = r#"{"type":"ai-title","aiTitle":"worktree 검토","sessionId":"s1"}"#;

    #[test]
    fn keeps_only_what_was_said_and_answered() {
        let root = scratch("said");
        let path = root.join("-home-me-proj").join("s1.jsonl");
        append(&path, &[PROMPT, ANSWER, TOOL_RESULT, REMINDER, META, TITLE]);
        let mut index = HistoryIndex::default();
        index.refresh(&root);

        let conversation = index.conversation("s1").unwrap();
        let texts: Vec<_> = conversation.messages.iter().map(|m| (m.role, m.text.as_str())).collect();
        assert_eq!(
            texts,
            vec![(Role::User, "worktree 를 넣을까?"), (Role::Assistant, "Orca 는 작업마다 Worktree 를 만듭니다.")]
        );
        assert_eq!(conversation.title.as_deref(), Some("worktree 검토"));
        assert_eq!(conversation.cwd.as_deref(), Some("/home/me/proj"));
        assert_eq!(conversation.branch.as_deref(), Some("main"));
    }

    #[test]
    fn search_is_case_insensitive_and_needs_every_term_in_one_message() {
        let root = scratch("search");
        let path = root.join("-home-me-proj").join("s1.jsonl");
        append(&path, &[PROMPT, ANSWER]);
        let mut index = HistoryIndex::default();
        index.refresh(&root);

        let hits = index.search("WORKTREE", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].matches, 2);
        assert_eq!(hits[0].first_match, Some(0));
        assert_eq!(hits[0].snippet.as_deref(), Some("worktree 를 넣을까?"));

        assert_eq!(index.search("orca worktree", 10)[0].first_match, Some(1));
        assert!(index.search("orca 넣을까", 10).is_empty(), "terms spread over two messages do not match");
        assert_eq!(index.search("", 10).len(), 1, "an empty query lists recent conversations");
    }

    #[test]
    fn refresh_reads_only_what_was_appended_and_waits_for_whole_lines() {
        let root = scratch("append");
        let path = root.join("-home-me-proj").join("s1.jsonl");
        append(&path, &[PROMPT]);
        let mut index = HistoryIndex::default();
        index.refresh(&root);
        assert_eq!(index.conversation("s1").unwrap().messages.len(), 1);

        // A line caught half-written is left for the next pass, not lost.
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        let (head, tail) = ANSWER.split_at(40);
        write!(file, "{head}").unwrap();
        index.refresh(&root);
        assert_eq!(index.conversation("s1").unwrap().messages.len(), 1);
        writeln!(file, "{tail}").unwrap();
        index.refresh(&root);
        assert_eq!(index.conversation("s1").unwrap().messages.len(), 2);

        fs::remove_file(&path).unwrap();
        index.refresh(&root);
        assert!(index.conversation("s1").is_none(), "a deleted transcript leaves the index");
    }

    #[test]
    fn slash_commands_and_shell_lines_read_as_typed() {
        assert_eq!(
            readable(Role::User, "<command-name>/jira</command-name>\n<command-message>jira</command-message>\n<command-args>등록 할 일</command-args>"),
            Some("/jira 등록 할 일".to_owned())
        );
        assert_eq!(readable(Role::User, "<bash-input>git status</bash-input>"), Some("! git status".to_owned()));
        assert_eq!(readable(Role::User, "<local-command-stdout>Set model</local-command-stdout>"), None);
    }

    #[test]
    fn snippet_cuts_around_the_match_in_the_original_text() {
        let text = format!("{}ÉTAPE worktree 끝", "가".repeat(60));
        let folded = fold(&text);
        let cut = snippet(&text, &folded, "worktree");
        assert!(cut.starts_with('…'));
        assert!(cut.contains("ÉTAPE worktree 끝"), "{cut}");
    }

    /// Not a correctness test: indexes the real `~/.claude/projects` and prints
    /// how long it took and how much text it kept. `cargo test --release -p
    /// amux-core history::tests::real_transcripts -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_transcripts() {
        let root = projects_dir().unwrap();
        let started = std::time::Instant::now();
        let mut index = HistoryIndex::default();
        index.refresh(&root);
        let built = started.elapsed();
        let started = std::time::Instant::now();
        index.refresh(&root);
        let again = started.elapsed();
        let (messages, bytes): (usize, usize) = index
            .transcripts
            .values()
            .map(|t| (t.messages.len(), t.messages.iter().map(|m| m.text.len()).sum::<usize>()))
            .fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
        let started = std::time::Instant::now();
        let hits = index.search("worktree", 50);
        let searched = started.elapsed();
        println!(
            "transcripts={} messages={} text={:.1}MB build={:?} refresh={:?} search={:?} hits={}",
            index.transcripts.len(),
            messages,
            bytes as f64 / 1048576.0,
            built,
            again,
            searched,
            hits.len()
        );
        for hit in hits.iter().take(3) {
            println!("  {:?} | {:?} | {:?}", hit.title, hit.last_ts, hit.snippet);
        }
    }
}
