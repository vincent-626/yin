//! What yin remembers between launches.

use std::path::{Path, PathBuf};

use yin_git::Repository;

fn state_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/yin"))
}

/// What a repository's window looked like, restored when it opens again.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Session {
    pub open_files: Vec<String>,
    pub active_file: Option<usize>,
    /// `changes`, `history`, `files` or `search`.
    pub panel: String,
    pub sidebar_hidden: bool,
    pub split_diff: bool,
    /// Sidebar width in points.
    pub sidebar_width: Option<f32>,
}

/// The window's last position and size, in points.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WindowFrame {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

pub fn load_window_frame() -> Option<WindowFrame> {
    let text = std::fs::read_to_string(state_dir()?.join("window.json")).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save_window_frame(frame: WindowFrame) {
    let Some(dir) = state_dir() else { return };
    let result = serde_json::to_string(&frame)
        .map_err(std::io::Error::other)
        .and_then(|json| {
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join("window.json"), json)
        });
    if let Err(error) = result {
        eprintln!("yin: couldn't save the window size: {error}");
    }
}

fn session_file(workdir: &Path) -> Option<PathBuf> {
    let key = crate::review::content_hash(Some(workdir.as_os_str().as_encoded_bytes()));
    Some(state_dir()?.join(format!("sessions/{key:016x}.json")))
}

pub fn load_session(workdir: &Path) -> Option<Session> {
    let text = std::fs::read_to_string(session_file(workdir)?).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save_session(workdir: &Path, session: &Session) {
    let Some(file) = session_file(workdir) else {
        return;
    };
    let result = serde_json::to_string_pretty(session)
        .map_err(std::io::Error::other)
        .and_then(|json| {
            std::fs::create_dir_all(file.parent().expect("sessions dir"))?;
            std::fs::write(&file, json)
        });
    if let Err(error) = result {
        eprintln!("yin: couldn't save the session: {error}");
    }
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
    fn sessions_tolerate_missing_fields() {
        let session: Session = serde_json::from_str(r#"{"open_files": ["a.rs"]}"#).unwrap();
        assert_eq!(session.open_files, ["a.rs"]);
        assert_eq!(session.panel, "");
        assert!(!session.split_diff);
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
