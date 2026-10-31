//! Finding git repositories in the folders people keep code in.

use std::path::{Path, PathBuf};

/// Folders under the home directory where code usually lives.
pub const CODE_FOLDERS: &[&str] = &[
    "Code",
    "code",
    "Projects",
    "projects",
    "Developer",
    "dev",
    "src",
    "repos",
    "git",
    "GitHub",
    "work",
];

/// Folders never worth looking inside for repositories.
const SKIPPED: &[&str] = &["node_modules", "target", "vendor", "Library"];

/// Repositories under `roots`, at most `depth` folders down, sorted. A
/// repository's own folders aren't searched for more.
pub fn find_repositories(roots: &[PathBuf], depth: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for root in roots {
        visit(root, depth, &mut found);
    }
    found.sort();
    found.dedup();
    found
}

fn visit(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    // `.git` is a folder in a repository and a file in a linked worktree.
    if dir.join(".git").exists() {
        found.push(dir.to_owned());
        return;
    }
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || SKIPPED.contains(&name.as_ref()) {
            continue;
        }
        // Symlinks are skipped: they can loop, and usually point at a
        // repository found elsewhere anyway.
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            visit(&entry.path(), depth - 1, found);
        }
    }
}
