//! Read-only views of a chat's working folder, for the chat view's side panel:
//! the folder's root (the git repository's top when there is one), one
//! directory level at a time (honouring .gitignore inside a repository), git's
//! list of changed files, and one file's content now and at the last commit.
//!
//! Every git call passes `--no-optional-locks`. The panel refreshes while
//! Claude may be running git itself, and a plain `git status` takes the index
//! lock to refresh its stat cache, which could make Claude's own `git add`
//! fail with "index.lock exists".

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;

const MAX_OUTPUT: usize = 512 * 1024;
const MAX_FILE: u64 = 512 * 1024;
const MAX_ENTRIES: usize = 3000;
const GIT_TIMEOUT: Duration = Duration::from_secs(10);

/// Never worth listing outside a repository (inside one, .gitignore decides).
const NOISE: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "venv",
    "__pycache__",
];

#[derive(Debug, Serialize)]
pub struct WorkRoot {
    root: String,
    git: bool,
    branch: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Entry {
    name: String,
    dir: bool,
}

#[derive(Debug, Serialize)]
pub struct Change {
    /// Relative to the root, `/`-separated.
    path: String,
    /// `modified`, `added`, `deleted`, `renamed`, `copied`, `new` (untracked)
    /// or `conflict`.
    status: &'static str,
    /// The old path of a rename or copy.
    from: Option<String>,
    added: Option<u32>,
    removed: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct FileText {
    /// `text`, `binary` or `missing`.
    kind: &'static str,
    text: String,
    truncated: bool,
    size: u64,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Run git in `root`; returns the exit code and up to MAX_OUTPUT of stdout.
fn git(root: &Path, args: &[&str], stdin: Option<&[u8]>) -> Result<(Option<i32>, Vec<u8>), String> {
    let mut child = Command::new("git")
        .arg("--no-optional-locks")
        .args(["-c", "core.quotepath=off", "-C"])
        .arg(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("git 을 실행하지 못했습니다: {e}"))?;
    if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let input = input.to_vec();
        std::thread::spawn(move || {
            let _ = pipe.write_all(&input);
        });
    }
    // Drain stdout on its own thread: a large output must not block git while
    // we wait for it to exit.
    let mut out = child.stdout.take().ok_or("git 출력 없음")?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = (&mut out).take(MAX_OUTPUT as u64).read_to_end(&mut buf);
        let _ = std::io::copy(&mut out, &mut std::io::sink());
        buf
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(err)? {
            break status;
        }
        if start.elapsed() > GIT_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err("git 이 너무 오래 걸려 멈췄습니다".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let buf = reader.join().unwrap_or_default();
    Ok((status.code(), buf))
}

/// `root` joined with `rel`, refusing anything that could leave the root.
fn resolve(root: &str, rel: &str) -> Result<(PathBuf, PathBuf), String> {
    let root = PathBuf::from(root);
    if !root.is_absolute() || !root.is_dir() {
        return Err("작업 폴더가 없습니다".into());
    }
    let rel_path = Path::new(rel);
    let inside = !rel_path.is_absolute()
        && rel_path.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    if !inside {
        return Err("작업 폴더 밖의 경로입니다".into());
    }
    let full = root.join(rel_path);
    Ok((root, full))
}

/// A symlink inside the root may still point outside it.
fn still_inside(root: &Path, full: &Path) -> bool {
    match (root.canonicalize(), full.canonicalize()) {
        (Ok(r), Ok(f)) => f.starts_with(r),
        _ => false,
    }
}

fn root_of(cwd: &Path) -> Result<WorkRoot, String> {
    let cwd = cwd.canonicalize().map_err(|e| format!("작업 폴더를 열 수 없습니다: {e}"))?;
    if let Ok((Some(0), out)) = git(&cwd, &["rev-parse", "--show-toplevel"], None) {
        let top = String::from_utf8_lossy(&out).trim().to_string();
        if !top.is_empty() {
            let top_path = PathBuf::from(&top);
            let branch = [&["symbolic-ref", "--short", "HEAD"][..], &["rev-parse", "--short", "HEAD"][..]]
                .iter()
                .find_map(|args| match git(&top_path, args, None) {
                    Ok((Some(0), o)) => Some(String::from_utf8_lossy(&o).trim().to_string()),
                    _ => None,
                })
                .filter(|b| !b.is_empty());
            return Ok(WorkRoot { root: top, git: true, branch });
        }
    }
    Ok(WorkRoot { root: cwd.to_string_lossy().into_owned(), git: false, branch: None })
}

fn list_dir(root: &str, rel: &str, in_git: bool) -> Result<Vec<Entry>, String> {
    let (root_path, dir) = resolve(root, rel)?;
    if !still_inside(&root_path, &dir) {
        return Err("작업 폴더 밖의 경로입니다".into());
    }
    let mut entries: Vec<Entry> = Vec::new();
    for item in std::fs::read_dir(&dir).map_err(err)?.flatten().take(MAX_ENTRIES) {
        let name = item.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        // Outside a repository, hidden files and build output are noise.
        if !in_git && (name.starts_with('.') || NOISE.contains(&name.as_str())) {
            continue;
        }
        let dir = item.file_type().map(|t| t.is_dir()).unwrap_or(false)
            || (item.file_type().map(|t| t.is_symlink()).unwrap_or(false) && item.path().is_dir());
        entries.push(Entry { name, dir });
    }
    if in_git && !entries.is_empty() {
        // Ask git which of these .gitignore hides. Directories get a trailing
        // slash so `build/` style patterns match them. Tracked files are
        // never reported, even when a pattern would match them.
        let prefix = if rel.is_empty() { String::new() } else { format!("{}/", rel.trim_end_matches('/')) };
        let mut input = Vec::new();
        for e in &entries {
            input.extend_from_slice(prefix.as_bytes());
            input.extend_from_slice(e.name.as_bytes());
            if e.dir {
                input.push(b'/');
            }
            input.push(0);
        }
        if let Ok((_, out)) = git(&root_path, &["check-ignore", "--stdin", "-z"], Some(&input)) {
            let ignored: std::collections::HashSet<String> = out
                .split(|b| *b == 0)
                .filter(|s| !s.is_empty())
                .map(|s| String::from_utf8_lossy(s).trim_end_matches('/').to_string())
                .collect();
            entries.retain(|e| !ignored.contains(&format!("{prefix}{}", e.name)));
        }
    }
    entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(entries)
}

fn kind_of(xy: &str) -> &'static str {
    let (x, y) = (xy.as_bytes()[0], xy.as_bytes()[1]);
    if xy == "??" {
        "new"
    } else if x == b'U' || y == b'U' || xy == "AA" || xy == "DD" {
        "conflict"
    } else if x == b'D' || y == b'D' {
        "deleted"
    } else if x == b'R' {
        "renamed"
    } else if x == b'C' {
        "copied"
    } else if x == b'A' {
        "added"
    } else {
        "modified"
    }
}

fn status_of(root: &str) -> Result<Vec<Change>, String> {
    let root_path = PathBuf::from(root);
    let (code, out) = git(&root_path, &["status", "--porcelain=v1", "-z", "--untracked-files=all"], None)?;
    if code != Some(0) {
        return Err("git status 를 읽지 못했습니다".into());
    }
    let mut changes = Vec::new();
    let mut parts = out.split(|b| *b == 0).filter(|s| !s.is_empty());
    while let Some(entry) = parts.next() {
        if entry.len() < 4 {
            continue;
        }
        let xy = String::from_utf8_lossy(&entry[..2]).into_owned();
        let path = String::from_utf8_lossy(&entry[3..]).into_owned();
        let from = if xy.starts_with('R') || xy.starts_with('C') {
            parts.next().map(|p| String::from_utf8_lossy(p).into_owned())
        } else {
            None
        };
        changes.push(Change { path, status: kind_of(&xy), from, added: None, removed: None });
        if changes.len() >= MAX_ENTRIES {
            break;
        }
    }
    // Line counts against HEAD for tracked files (absent before the first
    // commit), and a plain line count for new files.
    let mut counts: HashMap<String, (u32, u32)> = HashMap::new();
    if let Ok((Some(0), numstat)) = git(&root_path, &["diff", "--numstat", "-z", "HEAD"], None) {
        let mut fields = numstat.split(|b| *b == 0);
        while let Some(head) = fields.next() {
            let text = String::from_utf8_lossy(head);
            let mut cols = text.splitn(3, '\t');
            let (Some(a), Some(r), Some(p)) = (cols.next(), cols.next(), cols.next()) else { continue };
            // A rename leaves the path column empty and puts both paths next.
            let path = if p.is_empty() {
                let _old = fields.next();
                fields.next().map(|n| String::from_utf8_lossy(n).into_owned()).unwrap_or_default()
            } else {
                p.to_string()
            };
            if let (Ok(a), Ok(r)) = (a.parse(), r.parse()) {
                counts.insert(path, (a, r));
            }
        }
    }
    for c in &mut changes {
        if let Some((a, r)) = counts.get(&c.path) {
            c.added = Some(*a);
            c.removed = Some(*r);
        } else if c.status == "new" {
            let full = root_path.join(&c.path);
            if let Ok(meta) = std::fs::metadata(&full) {
                if meta.len() <= MAX_FILE {
                    if let Ok(bytes) = std::fs::read(&full) {
                        if !bytes.contains(&0) {
                            let lines = bytes.iter().filter(|b| **b == b'\n').count()
                                + usize::from(!bytes.is_empty() && !bytes.ends_with(b"\n"));
                            c.added = Some(lines as u32);
                            c.removed = Some(0);
                        }
                    }
                }
            }
        }
    }
    Ok(changes)
}

fn read_of(root: &str, rel: &str) -> Result<FileText, String> {
    let (root_path, full) = resolve(root, rel)?;
    if !full.exists() {
        return Ok(FileText { kind: "missing", text: String::new(), truncated: false, size: 0 });
    }
    if !still_inside(&root_path, &full) {
        return Err("작업 폴더 밖의 경로입니다".into());
    }
    let meta = std::fs::metadata(&full).map_err(err)?;
    if meta.is_dir() {
        return Err("폴더입니다".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&full).map_err(err)?.take(MAX_FILE).read_to_end(&mut bytes).map_err(err)?;
    let binary = bytes.iter().take(8000).any(|b| *b == 0);
    Ok(FileText {
        kind: if binary { "binary" } else { "text" },
        text: if binary { String::new() } else { String::from_utf8_lossy(&bytes).into_owned() },
        truncated: meta.len() > MAX_FILE,
        size: meta.len(),
    })
}

/// The file as of the last commit (`git show HEAD:<rel>`): what a file that
/// had no uncommitted changes looked like before something changed it.
fn head_of(root: &str, rel: &str) -> Result<FileText, String> {
    let (root_path, _) = resolve(root, rel)?;
    let spec = format!("HEAD:{rel}");
    match git(&root_path, &["show", &spec], None)? {
        (Some(0), out) => {
            let binary = out.iter().take(8000).any(|b| *b == 0);
            let truncated = out.len() >= MAX_OUTPUT;
            Ok(FileText {
                kind: if binary { "binary" } else { "text" },
                text: if binary { String::new() } else { String::from_utf8_lossy(&out).into_owned() },
                truncated,
                size: out.len() as u64,
            })
        }
        _ => Ok(FileText { kind: "missing", text: String::new(), truncated: false, size: 0 }),
    }
}

// Tauri commands: git and the file system can be slow on a big repository,
// so the work runs off the main thread (same as `search_history`).

#[tauri::command]
pub async fn work_root(cwd: String) -> Result<WorkRoot, String> {
    tauri::async_runtime::spawn_blocking(move || root_of(Path::new(&cwd))).await.map_err(err)?
}

#[tauri::command]
pub async fn work_list(root: String, rel: String, git: bool) -> Result<Vec<Entry>, String> {
    tauri::async_runtime::spawn_blocking(move || list_dir(&root, &rel, git)).await.map_err(err)?
}

#[tauri::command]
pub async fn work_status(root: String) -> Result<Vec<Change>, String> {
    tauri::async_runtime::spawn_blocking(move || status_of(&root)).await.map_err(err)?
}

#[tauri::command]
pub async fn work_head(root: String, rel: String) -> Result<FileText, String> {
    tauri::async_runtime::spawn_blocking(move || head_of(&root, &rel)).await.map_err(err)?
}

#[tauri::command]
pub async fn work_read(root: String, rel: String) -> Result<FileText, String> {
    tauri::async_runtime::spawn_blocking(move || read_of(&root, &rel)).await.map_err(err)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU32, Ordering};

    static SEQ: AtomicU32 = AtomicU32::new(0);

    fn sh(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .args(["-c", "user.email=t@example.com", "-c", "user.name=t", "-c", "init.defaultBranch=main"])
            .args(["-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"])
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    /// A committed repository, then: one file changed, one new, one deleted,
    /// plus ignored output that must stay hidden.
    fn repo() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "amux-workdir-test-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        fs::write(dir.join("sub/b.txt"), "bee\n").unwrap();
        fs::write(dir.join(".gitignore"), "build/\n*.log\n").unwrap();
        sh(&dir, &["init", "-q"]);
        sh(&dir, &["add", "-A"]);
        sh(&dir, &["commit", "-q", "-m", "init"]);
        fs::write(dir.join("a.txt"), "one\nTWO\nthree\n").unwrap();
        fs::write(dir.join("c.txt"), "new\nfile\n").unwrap();
        fs::remove_file(dir.join("sub/b.txt")).unwrap();
        fs::create_dir_all(dir.join("build")).unwrap();
        fs::write(dir.join("build/out.bin"), [0u8, 1, 2]).unwrap();
        fs::write(dir.join("x.log"), "noise\n").unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn root_is_the_repository_top_with_its_branch() {
        let dir = repo();
        let r = root_of(&dir.join("sub")).unwrap();
        assert!(r.git);
        assert_eq!(PathBuf::from(&r.root), dir);
        assert_eq!(r.branch.as_deref(), Some("main"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn status_lists_changed_new_and_deleted_files_with_counts() {
        let dir = repo();
        let root = dir.to_string_lossy();
        let changes = status_of(&root).unwrap();
        let get = |p: &str| changes.iter().find(|c| c.path == p).unwrap_or_else(|| panic!("{p} missing: {changes:?}"));
        assert_eq!(get("a.txt").status, "modified");
        assert_eq!((get("a.txt").added, get("a.txt").removed), (Some(2), Some(1)));
        assert_eq!(get("c.txt").status, "new");
        assert_eq!(get("c.txt").added, Some(2));
        assert_eq!(get("sub/b.txt").status, "deleted");
        assert!(!changes.iter().any(|c| c.path.ends_with(".log") || c.path.starts_with("build")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn listing_hides_ignored_paths_and_sorts_folders_first() {
        let dir = repo();
        let root = dir.to_string_lossy();
        let names: Vec<(String, bool)> =
            list_dir(&root, "", true).unwrap().into_iter().map(|e| (e.name, e.dir)).collect();
        assert_eq!(
            names,
            vec![
                ("sub".into(), true),
                (".gitignore".into(), false),
                ("a.txt".into(), false),
                ("c.txt".into(), false),
            ]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn contents_now_and_at_the_last_commit() {
        let dir = repo();
        let root = dir.to_string_lossy();
        let f = read_of(&root, "a.txt").unwrap();
        assert_eq!((f.kind, f.text.as_str()), ("text", "one\nTWO\nthree\n"));
        assert_eq!(read_of(&root, "sub/b.txt").unwrap().kind, "missing");
        assert_eq!(read_of(&root, "build/out.bin").unwrap().kind, "binary");
        let h = head_of(&root, "a.txt").unwrap();
        assert_eq!((h.kind, h.text.as_str()), ("text", "one\ntwo\n"));
        assert_eq!(head_of(&root, "sub/b.txt").unwrap().text, "bee\n");
        assert_eq!(head_of(&root, "c.txt").unwrap().kind, "missing");
        assert!(head_of(&root, "../x").is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn paths_cannot_leave_the_root() {
        let dir = repo();
        let root = dir.to_string_lossy();
        assert!(read_of(&root, "../etc/passwd").is_err());
        assert!(read_of(&root, "/etc/passwd").is_err());
        assert!(list_dir(&root, "sub/../..", true).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn plain_folders_hide_dotfiles_and_build_output() {
        let dir = std::env::temp_dir().join(format!("amux-workdir-plain-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("node_modules")).unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join(".env"), "x").unwrap();
        fs::write(dir.join("main.py"), "print(1)\n").unwrap();
        let dir = dir.canonicalize().unwrap();
        let r = root_of(&dir).unwrap();
        assert!(!r.git);
        let names: Vec<String> = list_dir(&r.root, "", false).unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["src".to_string(), "main.py".to_string()]);
        let _ = fs::remove_dir_all(&dir);
    }
}
