//! File URIs and finding server executables.

use std::path::{Path, PathBuf};
use std::str::FromStr as _;
use std::sync::OnceLock;

use anyhow::{Result, anyhow};
use lsp_types::Uri;

fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/')
}

pub fn path_to_uri(path: &Path) -> Result<Uri> {
    let path = path
        .to_str()
        .ok_or_else(|| anyhow!("{} is not UTF-8", path.display()))?;
    let mut uri = String::from("file://");
    for byte in path.bytes() {
        if is_unreserved(byte) {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    Uri::from_str(&uri).map_err(|error| anyhow!("bad uri {uri}: {error}"))
}

/// The local path of a `file://` URI.
pub fn uri_to_path(uri: &Uri) -> Option<PathBuf> {
    let rest = uri.as_str().strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut ix = 0;
    while ix < bytes.len() {
        if bytes[ix] == b'%'
            && let Some(hex) = rest.get(ix + 1..ix + 3)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            decoded.push(byte);
            ix += 3;
        } else {
            decoded.push(bytes[ix]);
            ix += 1;
        }
    }
    String::from_utf8(decoded).ok().map(PathBuf::from)
}

/// PATH from a login shell. An app opened from Finder gets a minimal PATH
/// that misses Homebrew, cargo and similar.
fn login_path() -> &'static str {
    static PATH: OnceLock<String> = OnceLock::new();
    PATH.get_or_init(|| {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        std::process::Command::new(shell)
            .args(["-l", "-c", "printf %s \"$PATH\""])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .unwrap_or_default()
    })
}

pub fn find_executable(name: &str) -> Option<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let fallbacks = [
        format!("{home}/.cargo/bin"),
        "/opt/homebrew/bin".into(),
        "/usr/local/bin".into(),
        format!("{home}/go/bin"),
    ];
    let env_path = std::env::var("PATH").unwrap_or_default();
    env_path
        .split(':')
        .chain(fallbacks.iter().map(String::as_str))
        .chain(login_path().split(':'))
        .filter(|dir| !dir.is_empty())
        .map(|dir| Path::new(dir).join(name))
        .find(|candidate| candidate.is_file())
}
