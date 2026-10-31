//! What yin remembers between launches.

use std::path::{Path, PathBuf};

fn state_dir() -> Option<PathBuf> {
    // Tests never read or write the real state.
    if cfg!(test) {
        return None;
    }
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

/// `~/Code/yin` rather than `/Users/me/Code/yin`.
pub fn tilde(path: &Path) -> String {
    let shown = path.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => match shown.strip_prefix(&home) {
            Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("~{rest}"),
            _ => shown,
        },
        _ => shown,
    }
}

/// How many repositories the switcher remembers.
const RECENT_LIMIT: usize = 30;

/// Repositories opened before, most recent first. Ones that no longer exist
/// are left out.
pub fn recent_repos() -> Vec<PathBuf> {
    let Some(dir) = state_dir() else {
        return Vec::new();
    };
    // `last-repo` held a single path before there was a list.
    let text = std::fs::read_to_string(dir.join("recent-repos"))
        .or_else(|_| std::fs::read_to_string(dir.join("last-repo")))
        .unwrap_or_default();
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .collect()
}

pub fn last_repo() -> Option<PathBuf> {
    recent_repos().into_iter().next()
}

/// Moves `path` to the front of the recent repositories.
pub fn remember_repo(path: &Path) {
    let Some(dir) = state_dir() else { return };
    let mut recent = recent_repos();
    recent.retain(|known| known != path);
    recent.insert(0, path.to_owned());
    recent.truncate(RECENT_LIMIT);
    let text: String = recent
        .iter()
        .map(|path| format!("{}\n", path.display()))
        .collect();
    let result =
        std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(dir.join("recent-repos"), text));
    if let Err(error) = result {
        eprintln!("yin: couldn't remember {}: {error}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_tolerate_missing_fields() {
        let session: Session = serde_json::from_str(r#"{"open_files": ["a.rs"]}"#).unwrap();
        assert_eq!(session.open_files, ["a.rs"]);
        assert_eq!(session.panel, "");
        assert!(!session.split_diff);
    }
}
