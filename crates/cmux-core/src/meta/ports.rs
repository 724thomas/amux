use std::collections::HashSet;
use std::fs;

/// TCP ports in LISTEN state owned by `root_pid` or any of its descendants.
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
