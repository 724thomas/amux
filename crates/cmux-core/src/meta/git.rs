use std::fs;
use std::path::Path;

/// Current branch (or detached short SHA) for the repo containing `dir`.
/// Handles both `.git` directories and `.git` files (worktrees — important,
/// since parallel agents typically run in worktrees).
pub fn branch_for(dir: &Path) -> Option<String> {
    let mut cur = Some(dir);
    while let Some(d) = cur {
        let dotgit = d.join(".git");
        if dotgit.is_dir() {
            return read_head(&dotgit);
        }
        if dotgit.is_file() {
            let content = fs::read_to_string(&dotgit).ok()?;
            let gitdir = content.strip_prefix("gitdir:")?.trim();
            let gitdir = if Path::new(gitdir).is_absolute() {
                Path::new(gitdir).to_path_buf()
            } else {
                d.join(gitdir)
            };
            return read_head(&gitdir);
        }
        cur = d.parent();
    }
    None
}

fn read_head(gitdir: &Path) -> Option<String> {
    let head = fs::read_to_string(gitdir.join("HEAD")).ok()?;
    let head = head.trim();
    if let Some(reference) = head.strip_prefix("ref: ") {
        let name = reference.strip_prefix("refs/heads/").unwrap_or(reference);
        Some(name.to_string())
    } else {
        // Detached HEAD: show a short SHA.
        Some(head.get(..8).unwrap_or(head).to_string())
    }
}
