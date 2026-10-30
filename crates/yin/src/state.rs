//! What yin remembers between launches.

use std::path::{Path, PathBuf};

use yin_git::Repository;

fn state_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/yin"))
}

pub fn last_repo() -> Option<PathBuf> {
    let contents = std::fs::read_to_string(state_dir()?.join("last-repo")).ok()?;
    let path = contents.trim();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

pub fn save_last_repo(path: &Path) {
    let Some(dir) = state_dir() else { return };
    let result = std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(dir.join("last-repo"), path.as_os_str().as_encoded_bytes()));
    if let Err(error) = result {
        eprintln!("yin: couldn't remember {}: {error}", path.display());
    }
}

/// Which repository to open at launch: an explicit argument (an error if it
/// isn't a repository), else the working directory, else the last one opened.
/// `None` shows the welcome screen.
pub fn initial_repo(
    argument: Option<PathBuf>,
    cwd: Option<PathBuf>,
    last: Option<PathBuf>,
) -> Result<Option<Repository>, String> {
    if let Some(path) = argument {
        return Repository::open(&path)
            .map(Some)
            .map_err(|error| error.to_string());
    }
    Ok([cwd, last]
        .into_iter()
        .flatten()
        .find_map(|path| Repository::open(path).ok()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn repo_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        dir
    }

    #[test]
    fn launch_order_is_argument_cwd_then_last() {
        let a = repo_dir();
        let b = repo_dir();
        let plain = tempfile::tempdir().unwrap();
        let name = |repo: Option<Repository>| repo.map(|repo| repo.workdir().to_owned());
        let canonical = |dir: &tempfile::TempDir| dir.path().canonicalize().unwrap();

        let opened = initial_repo(None, Some(a.path().into()), Some(b.path().into())).unwrap();
        assert_eq!(name(opened), Some(canonical(&a)));

        // Launched from Finder the cwd is `/`, not a repository.
        let opened = initial_repo(None, Some(plain.path().into()), Some(b.path().into())).unwrap();
        assert_eq!(name(opened), Some(canonical(&b)));

        let opened = initial_repo(None, Some(plain.path().into()), None).unwrap();
        assert!(opened.is_none());

        assert!(initial_repo(Some(plain.path().into()), Some(a.path().into()), None).is_err());
    }
}
