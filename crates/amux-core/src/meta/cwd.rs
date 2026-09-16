use std::path::PathBuf;

/// Current working directory of a process.
///
/// - **Linux**: read the `/proc/<pid>/cwd` symlink.
/// - **macOS**: there is no procfs; ask the kernel via `proc_pidinfo` with the
///   `PROC_PIDVNODEPATHINFO` flavor and read `pvi_cdir.vip_path`.
/// - **Windows**: the directory lives in the process's PEB, at offsets
///   Microsoft does not document. `sysinfo` already reads it, so we ask it
///   rather than hard-coding a pointer walk here.
/// - **Other**: unsupported — returns `None`, which every caller already
///   treats as "unknown" (cwd/git meta fall back to empty).
#[cfg(target_os = "linux")]
pub fn process_cwd(pid: u32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

#[cfg(target_os = "macos")]
pub fn process_cwd(pid: u32) -> Option<PathBuf> {
    use std::ffi::{CStr, OsStr};
    use std::os::unix::ffi::OsStrExt;

    // SAFETY: `proc_vnodepathinfo` is a plain-old-data struct; zero-initialising
    // it is valid. We only read it back after the kernel reports it filled the
    // whole struct (see the `n < size` guard below).
    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;

    // SAFETY: FFI call into libproc; `info` is `size` bytes and writable.
    let n = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            (&mut info as *mut libc::proc_vnodepathinfo).cast::<libc::c_void>(),
            size,
        )
    };
    // proc_pidinfo returns the number of bytes written; a short/zero/negative
    // result means the pid is gone or we lack permission.
    if n < size {
        return None;
    }

    // `vip_path` is a NUL-terminated C string in a `char[MAXPATHLEN]` buffer.
    // libc types it as `[[c_char; 32]; 32]` (an old-rustc workaround for a flat
    // 1024-byte array), so cast to a flat `*const c_char` before reading.
    // SAFETY: the buffer is contiguous and the kernel NUL-terminates within it.
    let path_ptr = info.pvi_cdir.vip_path.as_ptr().cast::<libc::c_char>();
    let bytes = unsafe { CStr::from_ptr(path_ptr) }.to_bytes();
    if bytes.is_empty() {
        return None;
    }
    Some(PathBuf::from(OsStr::from_bytes(bytes)))
}

#[cfg(windows)]
pub fn process_cwd(pid: u32) -> Option<PathBuf> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    // Refresh exactly one pid, and only the field we are about to read: the
    // sweeper calls this once per pane every second, and a full process
    // enumeration at that cadence is real CPU spent on rows we discard.
    let mut sys = System::new();
    let pid = Pid::from_u32(pid);
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        // `false`: do not prune the (empty) process list of dead entries.
        false,
        ProcessRefreshKind::nothing().with_cwd(sysinfo::UpdateKind::Always),
    );

    // An empty path means the read failed — a process at a higher integrity
    // level, or one that exited between the snapshot and here.
    sys.process(pid)
        .and_then(|p| p.cwd())
        .filter(|p| !p.as_os_str().is_empty())
        .map(PathBuf::from)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn process_cwd(_pid: u32) -> Option<PathBuf> {
    None
}

#[cfg(all(test, target_os = "macos"))]
mod macos_smoke {
    #[test]
    fn cwd_of_self_matches_current_dir() {
        let pid = std::process::id();
        let got = super::process_cwd(pid).expect("cwd for self");
        let want = std::env::current_dir().unwrap();
        // canonicalize both: /tmp is a symlink to /private/tmp on macOS.
        assert_eq!(
            std::fs::canonicalize(&got).unwrap(),
            std::fs::canonicalize(&want).unwrap(),
            "got {got:?} want {want:?}"
        );
    }
}

#[cfg(all(test, windows))]
mod windows_smoke {
    /// The twin of the macOS smoke test above: the PEB read has to agree with
    /// what this very process knows its own directory to be.
    #[test]
    fn cwd_of_self_matches_current_dir() {
        let got = super::process_cwd(std::process::id()).expect("cwd for self");
        let want = std::env::current_dir().unwrap();
        assert_eq!(
            std::fs::canonicalize(&got).unwrap(),
            std::fs::canonicalize(&want).unwrap(),
            "got {got:?} want {want:?}"
        );
    }

    /// The case the sidebar actually depends on, and the one reading our own
    /// PEB cannot prove: another process, spawned into a directory of our
    /// choosing. This is how a pane's `claude` or `node` is read.
    #[test]
    fn cwd_of_another_process_is_where_it_was_spawned() {
        let want = std::env::temp_dir();
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/c", "pause"])
            .current_dir(&want)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("spawn cmd.exe");

        // The PEB is populated by the loader, not by CreateProcess returning,
        // so a just-spawned process can read back empty for a moment.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let got = loop {
            if let Some(dir) = super::process_cwd(child.id()) {
                break dir;
            }
            assert!(std::time::Instant::now() < deadline, "child cwd never appeared");
            std::thread::sleep(std::time::Duration::from_millis(50));
        };

        let _ = child.kill();
        let _ = child.wait();

        assert_eq!(
            std::fs::canonicalize(&got).unwrap(),
            std::fs::canonicalize(&want).unwrap(),
            "got {got:?} want {want:?}"
        );
    }
}
