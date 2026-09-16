use std::path::PathBuf;

/// Current working directory of a process.
///
/// - **Linux**: read the `/proc/<pid>/cwd` symlink.
/// - **macOS**: there is no procfs; ask the kernel via `proc_pidinfo` with the
///   `PROC_PIDVNODEPATHINFO` flavor and read `pvi_cdir.vip_path`.
/// - **Other (Windows, …)**: unsupported — returns `None`, which every caller
///   already treats as "unknown" (cwd/git meta fall back to empty).
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

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
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
