//! Watches the working tree and git dirs so the UI follows changes made
//! elsewhere: agents, terminals, other editors.

use std::path::{Path, PathBuf};

use anyhow::Result;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use notify::{EventKind, RecursiveMode, Watcher as _};
use yin_git::Repository;

/// Keeps the OS watch alive; drop it to stop watching.
pub struct RepoWatcher {
    _watcher: notify::RecommendedWatcher,
}

/// Starts watching `repo`. The receiver gets a message for every batch of
/// relevant filesystem events; callers debounce.
pub fn watch(repo: &Repository) -> Result<(RepoWatcher, async_channel::Receiver<()>)> {
    let filter = Filter::new(repo)?;
    let (sender, receiver) = async_channel::unbounded();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else { return };
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        if event.paths.iter().any(|path| filter.is_relevant(path)) {
            sender.try_send(()).ok();
        }
    })?;
    let workdir = canonical(repo.workdir());
    watcher.watch(&workdir, RecursiveMode::Recursive)?;
    let (git_dir, common_dir) = repo.git_dirs()?;
    for dir in [git_dir, common_dir] {
        if !dir.starts_with(&workdir) {
            watcher.watch(&dir, RecursiveMode::Recursive)?;
        }
    }
    Ok((RepoWatcher { _watcher: watcher }, receiver))
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_owned())
}

struct Filter {
    workdir: PathBuf,
    git_dirs: [PathBuf; 2],
    ignore: Gitignore,
}

impl Filter {
    fn new(repo: &Repository) -> Result<Self> {
        let workdir = canonical(repo.workdir());
        let (git_dir, common_dir) = repo.git_dirs()?;
        // Root .gitignore and info/exclude cover the noisy directories (target/,
        // node_modules/). Nested .gitignore files only cost an extra refresh.
        let mut builder = GitignoreBuilder::new(&workdir);
        builder.add(workdir.join(".gitignore"));
        builder.add(common_dir.join("info/exclude"));
        Ok(Self {
            ignore: builder.build()?,
            git_dirs: [canonical(&git_dir), canonical(&common_dir)],
            workdir,
        })
    }

    fn is_relevant(&self, path: &Path) -> bool {
        for git_dir in &self.git_dirs {
            if let Ok(inner) = path.strip_prefix(git_dir) {
                return is_relevant_git_path(inner);
            }
        }
        match path.strip_prefix(&self.workdir) {
            Ok(relative) => {
                // Paths under a nested repository's .git still land here.
                if relative.components().any(|c| c.as_os_str() == ".git") {
                    return false;
                }
                !self
                    .ignore
                    .matched_path_or_any_parents(relative, path.is_dir())
                    .is_ignore()
            }
            Err(_) => false,
        }
    }
}

/// Inside a git dir, only files that change what yin shows matter.
fn is_relevant_git_path(inner: &Path) -> bool {
    if inner.extension().is_some_and(|ext| ext == "lock") {
        return false;
    }
    let Some(first) = inner.components().next() else {
        return false;
    };
    matches!(
        first.as_os_str().to_str(),
        Some(
            "index"
                | "HEAD"
                | "refs"
                | "packed-refs"
                | "MERGE_HEAD"
                | "REBASE_HEAD"
                | "CHERRY_PICK_HEAD"
                | "rebase-merge"
                | "rebase-apply"
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn ignores_git_noise_and_ignored_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = canonical(dir.path());
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .status()
            .unwrap();
        std::fs::write(root.join(".gitignore"), "target/\n*.log\n").unwrap();
        let repo = Repository::open(&root).unwrap();
        let filter = Filter::new(&repo).unwrap();

        assert!(filter.is_relevant(&root.join("src/main.rs")));
        assert!(filter.is_relevant(&root.join(".git/index")));
        assert!(filter.is_relevant(&root.join(".git/HEAD")));
        assert!(filter.is_relevant(&root.join(".git/refs/heads/main")));

        assert!(!filter.is_relevant(&root.join("target/debug/yin")));
        assert!(!filter.is_relevant(&root.join("build.log")));
        assert!(!filter.is_relevant(&root.join(".git/index.lock")));
        assert!(!filter.is_relevant(&root.join(".git/objects/ab/cdef")));
        assert!(!filter.is_relevant(&root.join(".git/logs/HEAD")));
        assert!(!filter.is_relevant(Path::new("/somewhere/else")));
    }
}
