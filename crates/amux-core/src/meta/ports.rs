//! TCP ports in LISTEN state owned by a pane's process tree.
//!
//! - **Linux**: parse `/proc/net/tcp{,6}` and match socket inodes found under
//!   `/proc/<pid>/fd`, walking descendants via `/proc/<pid>/task/*/children`.
//! - **macOS**: there is no procfs; ask the kernel via libproc — walk
//!   descendants by parent pid, list each process's socket fds, and read the
//!   TCP state / local port of each.
//! - **Windows**: the TCP table already records the owning pid of every
//!   socket, so no fd/inode matching is needed — ask `GetExtendedTcpTable`
//!   for the LISTEN rows and keep the ones our process tree owns.
//! - **Other**: unsupported — returns an empty list.

#[cfg(target_os = "linux")]
mod imp {
    use std::collections::HashSet;
    use std::fs;

    pub fn listening_ports(root_pid: u32) -> Vec<u16> {
        let inodes: HashSet<u64> = descendants(root_pid)
            .into_iter()
            .flat_map(socket_inodes)
            .collect();
        if inodes.is_empty() {
            return Vec::new();
        }
        let mut ports = listen_ports_matching("/proc/net/tcp", &inodes);
        ports.extend(listen_ports_matching("/proc/net/tcp6", &inodes));
        ports.sort_unstable();
        ports.dedup();
        ports
    }

    /// BFS over /proc/<pid>/task/*/children.
    fn descendants(root: u32) -> Vec<u32> {
        let mut all = vec![root];
        let mut queue = vec![root];
        while let Some(pid) = queue.pop() {
            let task_dir = format!("/proc/{pid}/task");
            let Ok(tasks) = fs::read_dir(&task_dir) else { continue };
            for task in tasks.flatten() {
                let children_path = task.path().join("children");
                let Ok(children) = fs::read_to_string(children_path) else { continue };
                for child in children.split_whitespace().filter_map(|c| c.parse::<u32>().ok()) {
                    all.push(child);
                    queue.push(child);
                }
            }
        }
        all
    }

    fn socket_inodes(pid: u32) -> Vec<u64> {
        let Ok(fds) = fs::read_dir(format!("/proc/{pid}/fd")) else {
            return Vec::new();
        };
        fds.flatten()
            .filter_map(|fd| fs::read_link(fd.path()).ok())
            .filter_map(|target| {
                let s = target.to_string_lossy();
                s.strip_prefix("socket:[")?
                    .strip_suffix(']')?
                    .parse::<u64>()
                    .ok()
            })
            .collect()
    }

    /// Parse /proc/net/tcp{,6}: rows with state 0A (LISTEN) whose inode is ours.
    fn listen_ports_matching(path: &str, inodes: &HashSet<u64>) -> Vec<u16> {
        let Ok(content) = fs::read_to_string(path) else {
            return Vec::new();
        };
        content
            .lines()
            .skip(1)
            .filter_map(|line| {
                let fields: Vec<&str> = line.split_whitespace().collect();
                let (local, state, inode) = (fields.get(1)?, fields.get(3)?, fields.get(9)?);
                if *state != "0A" || !inodes.contains(&inode.parse::<u64>().ok()?) {
                    return None;
                }
                let port_hex = local.rsplit(':').next()?;
                u16::from_str_radix(port_hex, 16).ok()
            })
            .collect()
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use libproc::libproc::bsd_info::BSDInfo;
    use libproc::libproc::file_info::{pidfdinfo, ListFDs, ProcFDType};
    use libproc::libproc::net_info::{SocketFDInfo, SocketInfoKind, TcpSIState};
    use libproc::libproc::proc_pid::{listpidinfo, pidinfo};
    use libproc::processes::{pids_by_type, ProcFilter};

    pub fn listening_ports(root_pid: u32) -> Vec<u16> {
        let mut ports: Vec<u16> = descendants(root_pid)
            .into_iter()
            .flat_map(pid_listen_ports)
            .collect();
        ports.sort_unstable();
        ports.dedup();
        ports
    }

    /// BFS over child processes, resolved by parent pid (there is no
    /// `/proc/<pid>/children`; libproc filters the live pid table by ppid).
    fn descendants(root: u32) -> Vec<u32> {
        let mut all = vec![root];
        let mut queue = vec![root];
        while let Some(pid) = queue.pop() {
            let Ok(children) = pids_by_type(ProcFilter::ByParentProcess { ppid: pid }) else {
                continue;
            };
            for child in children {
                all.push(child);
                queue.push(child);
            }
        }
        all
    }

    /// Local ports of this process's TCP sockets that are in LISTEN state.
    fn pid_listen_ports(pid: u32) -> Vec<u16> {
        let pid = pid as i32;
        // pbi_nfiles bounds how many fds to ask for.
        let Ok(info) = pidinfo::<BSDInfo>(pid, 0) else {
            return Vec::new();
        };
        let Ok(fds) = listpidinfo::<ListFDs>(pid, info.pbi_nfiles as usize) else {
            return Vec::new();
        };

        let mut ports = Vec::new();
        for fd in fds {
            if !matches!(fd.proc_fdtype.into(), ProcFDType::Socket) {
                continue;
            }
            let Ok(socket) = pidfdinfo::<SocketFDInfo>(pid, fd.proc_fd) else {
                continue;
            };
            let info = socket.psi;
            if !matches!(info.soi_kind.into(), SocketInfoKind::Tcp) {
                continue;
            }
            // SAFETY: soi_kind == Tcp means the `soi_proto` union holds `pri_tcp`.
            let tcp = unsafe { info.soi_proto.pri_tcp };
            if tcp.tcpsi_state != TcpSIState::Listen as i32 {
                continue;
            }
            // insi_lport is the local port in network byte order, sign-extended
            // into a c_int; take the low 16 bits and convert to host order.
            let port = u16::from_be(tcp.tcpsi_ini.insi_lport as u16);
            if port != 0 {
                ports.push(port);
            }
        }
        ports
    }
}

#[cfg(windows)]
mod imp {
    use std::collections::HashSet;

    use windows::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID, MIB_TCPROW_OWNER_PID,
        MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
    };
    use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6};

    use crate::win_proc;

    pub fn listening_ports(root_pid: u32) -> Vec<u16> {
        // The tree is the same snapshot `app_running` and the cwd lookup just
        // used, so this adds no extra walk of the process table.
        let owned: HashSet<u32> = win_proc::tree().descendants(root_pid).into_iter().collect();

        let mut ports = v4_listeners(&owned);
        ports.extend(v6_listeners(&owned));
        ports.sort_unstable();
        ports.dedup();
        ports
    }

    /// One family's LISTEN table, as raw bytes.
    ///
    /// The table is sized by the call itself, and the machine can open a
    /// socket between the sizing call and the fetch — hence the retries
    /// rather than a single "ask, then read" pair.
    fn table_bytes(family: u32) -> Option<Vec<u32>> {
        let mut size = 0u32;
        for _ in 0..4 {
            // Ask for the size (buffer omitted), then fetch into it. `false`
            // leaves the rows unsorted; we filter rather than scan in order.
            // SAFETY: `size` is a live out-param, and on the sizing pass no
            // buffer is handed over at all.
            let need = unsafe {
                GetExtendedTcpTable(
                    None,
                    &mut size,
                    false,
                    family,
                    TCP_TABLE_OWNER_PID_LISTENER,
                    0,
                )
            };
            // ERROR_INSUFFICIENT_BUFFER (122) is the expected answer and has
            // filled in `size`; NO_ERROR means the table is empty.
            if need == 0 || size == 0 {
                return None;
            }
            // A Vec<u32> is 4-byte aligned, which the MIB structs require;
            // a Vec<u8> carries no such guarantee.
            let mut buf = vec![0u32; size.div_ceil(4) as usize];
            // SAFETY: the buffer holds at least `size` bytes, which is what
            // the sizing call above asked for.
            let rc = unsafe {
                GetExtendedTcpTable(
                    Some(buf.as_mut_ptr().cast()),
                    &mut size,
                    false,
                    family,
                    TCP_TABLE_OWNER_PID_LISTENER,
                    0,
                )
            };
            if rc == 0 {
                return Some(buf);
            }
            // Anything else (the table grew, typically) — size is updated, so
            // go round again.
        }
        None
    }

    fn v4_listeners(owned: &HashSet<u32>) -> Vec<u16> {
        let Some(buf) = table_bytes(AF_INET.0 as u32) else {
            return Vec::new();
        };
        // SAFETY: on success the buffer holds a MIB_TCPTABLE_OWNER_PID whose
        // `dwNumEntries` counts the rows laid out contiguously after it.
        unsafe {
            let table = &*buf.as_ptr().cast::<MIB_TCPTABLE_OWNER_PID>();
            let rows = std::slice::from_raw_parts(
                table.table.as_ptr().cast::<MIB_TCPROW_OWNER_PID>(),
                table.dwNumEntries as usize,
            );
            rows.iter()
                .filter(|r| owned.contains(&r.dwOwningPid))
                .filter_map(|r| port_of(r.dwLocalPort))
                .collect()
        }
    }

    fn v6_listeners(owned: &HashSet<u32>) -> Vec<u16> {
        let Some(buf) = table_bytes(AF_INET6.0 as u32) else {
            return Vec::new();
        };
        // SAFETY: as above, for the IPv6 table.
        unsafe {
            let table = &*buf.as_ptr().cast::<MIB_TCP6TABLE_OWNER_PID>();
            let rows = std::slice::from_raw_parts(
                table.table.as_ptr().cast::<MIB_TCP6ROW_OWNER_PID>(),
                table.dwNumEntries as usize,
            );
            rows.iter()
                .filter(|r| owned.contains(&r.dwOwningPid))
                .filter_map(|r| port_of(r.dwLocalPort))
                .collect()
        }
    }

    /// `dwLocalPort` carries the port in network byte order in its low half.
    fn port_of(raw: u32) -> Option<u16> {
        Some(u16::from_be(raw as u16)).filter(|p| *p != 0)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod imp {
    pub fn listening_ports(_root_pid: u32) -> Vec<u16> {
        Vec::new()
    }
}

/// TCP ports in LISTEN state owned by `root_pid` or any of its descendants.
pub fn listening_ports(root_pid: u32) -> Vec<u16> {
    imp::listening_ports(root_pid)
}

#[cfg(all(test, target_os = "macos"))]
mod macos_smoke {
    #[test]
    fn detects_own_listening_port() {
        // Bind a real TCP listener in this process, then confirm libproc
        // surfaces its port for our own pid.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let want = listener.local_addr().unwrap().port();
        let ports = super::listening_ports(std::process::id());
        assert!(ports.contains(&want), "want {want} in {ports:?}");
    }
}

#[cfg(all(test, windows))]
mod windows_smoke {
    /// The twin of the macOS smoke test above: bind a real listener in this
    /// process and confirm the TCP table surfaces its port under our pid.
    #[test]
    fn detects_own_listening_port() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let want = listener.local_addr().unwrap().port();
        let ports = super::listening_ports(std::process::id());
        assert!(ports.contains(&want), "want {want} in {ports:?}");
    }

    /// The sweeper hands us the pane's *shell*, and the server that matters
    /// runs as a child of it. A port found only on a descendant is the whole
    /// reason this walks the tree instead of reading one pid.
    #[test]
    fn a_port_held_by_a_child_counts_as_the_parents() {
        // A child that binds a port and holds it until killed. PowerShell is
        // the pane shell on Windows, so this is the shape the sweeper meets.
        let mut child = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-Command",
                "$l=[System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback,0);                  $l.Start(); $l.LocalEndpoint.Port; Start-Sleep -Seconds 30",
            ])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("spawn powershell");

        // The child prints the port it got before it starts waiting.
        let mut line = String::new();
        {
            use std::io::BufRead;
            let stdout = child.stdout.take().expect("stdout");
            std::io::BufReader::new(stdout).read_line(&mut line).expect("read port");
        }
        let want: u16 = line.trim().parse().unwrap_or_else(|e| {
            let _ = child.kill();
            panic!("child printed {line:?}, not a port: {e}");
        });

        // The process tree is a snapshot held for half a second, so a child
        // this young may not be in the one on hand yet. The sweeper simply
        // catches it on a later tick; wait the same way rather than reaching
        // past the cache, so what passes here is the path that runs live.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let found = loop {
            let found = super::listening_ports(std::process::id());
            if found.contains(&want) || std::time::Instant::now() >= deadline {
                break found;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        };
        let _ = child.kill();
        let _ = child.wait();

        assert!(found.contains(&want), "child's port {want} missing from {found:?}");
    }
}
