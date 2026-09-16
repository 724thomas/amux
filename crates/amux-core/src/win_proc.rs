//! The pane's process tree on Windows — what Unix answers with a foreground
//! process group.
//!
//! Three questions the engine asks about every pane reduce to the same walk:
//!
//! - *is a command running?* — the shell has a live child (`Pane::app_running`)
//! - *what directory is it in?* — read the child's cwd (`meta::cwd`)
//! - *which ports does it hold?* — the whole subtree owns them (`meta::ports`)
//!
//! On Unix `tcgetpgrp` answers the first two for free. ConPTY has no process
//! groups, so we enumerate instead: one `CreateToolhelp32Snapshot` lists every
//! process with its parent, which is enough to rebuild the tree.
//!
//! That snapshot walks the whole machine, and the metadata sweeper asks all
//! three questions of every pane once a second. So a snapshot is taken for a
//! beat and shared, rather than taken per question — see [`SNAPSHOT_TTL`].

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// How long one snapshot is reused. Shorter than the sweeper's one-second
/// tick, so a snapshot is never carried from one tick into the next: within a
/// tick the panes share it, across ticks every pane sees fresh processes.
const SNAPSHOT_TTL: Duration = Duration::from_millis(500);

/// Every live process's parent, as one Toolhelp snapshot saw them.
pub struct ProcTree {
    /// parent pid -> child pids, in the order Toolhelp reported them.
    children: HashMap<u32, Vec<u32>>,
}

impl ProcTree {
    /// Walk the live process table once.
    fn capture() -> Self {
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();

        // SAFETY: the snapshot handle is closed on every path out, and
        // `entry` is sized with `dwSize` exactly as Process32FirstW requires.
        unsafe {
            let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
                return Self { children };
            };
            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            if Process32FirstW(snapshot, &mut entry).is_ok() {
                loop {
                    children
                        .entry(entry.th32ParentProcessID)
                        .or_default()
                        .push(entry.th32ProcessID);
                    if Process32NextW(snapshot, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snapshot);
        }

        Self { children }
    }

    /// Direct children of `pid`.
    pub fn children_of(&self, pid: u32) -> &[u32] {
        self.children.get(&pid).map_or(&[], Vec::as_slice)
    }

    /// `root` and every process below it, breadth-first.
    ///
    /// Mirrors the macOS `descendants()` in `meta::ports`, which resolves the
    /// same tree by asking libproc for each pid's children.
    pub fn descendants(&self, root: u32) -> Vec<u32> {
        let mut all = vec![root];
        let mut queue = vec![root];
        // A pid cannot be its own ancestor, but a recycled pid could make the
        // recorded parent links look circular; the visited set keeps a corrupt
        // snapshot from spinning here forever.
        let mut seen = std::collections::HashSet::from([root]);
        while let Some(pid) = queue.pop() {
            for &child in self.children_of(pid) {
                if seen.insert(child) {
                    all.push(child);
                    queue.push(child);
                }
            }
        }
        all
    }

    /// The child of `shell` that started most recently, or `None` when the
    /// shell is sitting at its prompt with nothing running.
    ///
    /// This is the closest thing Windows has to the foreground process group
    /// leader: a command typed at the prompt (`claude`, `node`, `git`, ...)
    /// runs as a direct child of the shell. Several children can be alive at
    /// once — a `Start-Process` left running in the background, say — and the
    /// newest is the one the user just typed.
    pub fn newest_child(&self, shell: u32) -> Option<u32> {
        match self.children_of(shell) {
            [] => None,
            // The overwhelmingly common case: one command, no timestamps to
            // fetch. Each `started_at` costs an OpenProcess.
            [only] => Some(*only),
            many => many.iter().copied().max_by_key(|&pid| started_at(pid)),
        }
    }
}

/// When a process started, as a raw FILETIME tick count. Zero if it cannot be
/// opened — a process we may not query sorts oldest, so a readable sibling
/// wins the comparison in `newest_child`.
fn started_at(pid: u32) -> u64 {
    // PROCESS_QUERY_LIMITED_INFORMATION is the weakest right that carries
    // GetProcessTimes; it is granted for our own processes without elevation.
    // SAFETY: the handle is closed before returning, and the four FILETIME
    // out-params are owned locals.
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return 0;
        };
        let mut created = FILETIME::default();
        let (mut exit, mut kernel, mut user) = Default::default();
        let ok = GetProcessTimes(handle, &mut created, &mut exit, &mut kernel, &mut user).is_ok();
        let _ = CloseHandle(HANDLE(handle.0));
        if !ok {
            return 0;
        }
        (created.dwHighDateTime as u64) << 32 | created.dwLowDateTime as u64
    }
}

/// The shared snapshot and the moment it was taken.
static CACHED: Mutex<Option<(Instant, Arc<ProcTree>)>> = Mutex::new(None);

/// The process tree as of at most [`SNAPSHOT_TTL`] ago.
pub fn tree() -> Arc<ProcTree> {
    let mut cached = CACHED.lock();
    if let Some((taken, tree)) = cached.as_ref() {
        if taken.elapsed() < SNAPSHOT_TTL {
            return Arc::clone(tree);
        }
    }
    let tree = Arc::new(ProcTree::capture());
    *cached = Some((Instant::now(), Arc::clone(&tree)));
    tree
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keep a real process alive for the length of a test, and take it down
    /// even if an assertion unwinds first.
    struct Child(std::process::Child);

    impl Child {
        fn spawn() -> Self {
            Self(
                std::process::Command::new("cmd.exe")
                    .args(["/c", "pause"])
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .expect("spawn cmd.exe"),
            )
        }
        fn pid(&self) -> u32 {
            self.0.id()
        }
    }

    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// The tree has to find a process we just created, under us, or none of
    /// the three questions built on it can be answered.
    ///
    /// Only membership is asserted, never *which* child is newest: the test
    /// harness runs these in parallel threads of one process, so a sibling
    /// spawned by another test is a real and legitimate child of ours too.
    #[test]
    fn a_spawned_child_shows_up_under_this_process() {
        let child = Child::spawn();
        let me = std::process::id();
        // Fresh, not the cached one: the child is younger than the TTL.
        let tree = ProcTree::capture();

        assert!(
            tree.children_of(me).contains(&child.pid()),
            "child {} not among children of {me}: {:?}",
            child.pid(),
            tree.children_of(me),
        );
        assert!(tree.descendants(me).contains(&child.pid()), "descendants");
    }

    /// When a shell holds several live children — a background `Start-Process`
    /// alongside the command just typed — the newest is the one the user is
    /// looking at. Asserted against a parent that exists only in this map, so
    /// no other test's child can join the comparison.
    #[test]
    fn the_newest_of_several_children_wins() {
        let older = Child::spawn();
        // FILETIME has 100ns resolution, but process creation is stamped far
        // more coarsely; a clear gap keeps the two ordered.
        std::thread::sleep(Duration::from_millis(100));
        let newer = Child::spawn();

        let tree = ProcTree {
            children: HashMap::from([(u32::MAX, vec![older.pid(), newer.pid()])]),
        };
        assert_eq!(
            tree.newest_child(u32::MAX),
            Some(newer.pid()),
            "older {} started {}, newer {} started {}",
            older.pid(),
            started_at(older.pid()),
            newer.pid(),
            started_at(newer.pid()),
        );
    }

    /// A shell with nothing running has to read as idle — this is the signal
    /// `Pane::app_running` turns into the status chip.
    #[test]
    fn a_process_with_no_children_reports_none() {
        let tree = ProcTree::capture();
        // A pid that cannot exist: Windows pids are multiples of 4.
        assert_eq!(tree.newest_child(u32::MAX), None);
        assert_eq!(tree.descendants(u32::MAX), vec![u32::MAX]);
    }

    /// Back-to-back calls share one Toolhelp walk; a later one re-takes it.
    #[test]
    fn the_snapshot_is_shared_within_its_ttl() {
        let first = tree();
        assert!(Arc::ptr_eq(&first, &tree()), "second call re-enumerated");
        std::thread::sleep(SNAPSHOT_TTL + Duration::from_millis(50));
        assert!(!Arc::ptr_eq(&first, &tree()), "stale snapshot was reused");
    }
}
