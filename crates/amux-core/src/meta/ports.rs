//! TCP ports in LISTEN state owned by a pane's process tree.
//!
//! - **Linux**: parse `/proc/net/tcp{,6}` and match socket inodes found under
//!   `/proc/<pid>/fd`, walking descendants via `/proc/<pid>/task/*/children`.
//! - **macOS**: there is no procfs; ask the kernel via libproc — walk
//!   descendants by parent pid, list each process's socket fds, and read the
//!   TCP state / local port of each.
//! - **Other (Windows, …)**: unsupported — returns an empty list.

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

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
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
