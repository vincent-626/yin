//! Updating yin from its GitHub releases: a check at launch and every few
//! hours, a button in the title bar when a newer version is out, and an
//! install that verifies the download, swaps the app in place and restarts.
//! Only a bundled app updates itself; a development build never does.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use gpui_kit::component::{
    Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    notification::Notification,
};
use gpui_kit::{App, AppContext as _, Global, IntoElement, Styled as _, Window};

const REPO: &str = "vincent-626/yin";
const ASSET: &str = "yin-macos-arm64.zip";
/// How often a running yin looks for a release.
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// A release newer than this build.
#[derive(Clone, Debug)]
pub struct Release {
    /// `0.5.1`, without the `v`.
    pub version: String,
    pub notes: String,
    /// The download's size in bytes, for showing progress.
    pub size: u64,
}

#[derive(Default)]
struct Updates {
    available: Option<Release>,
    installing: bool,
    /// How much of the download has arrived, 0–100.
    progress: Option<u8>,
}

impl Global for Updates {}

/// `0.5.1` as numbers, for comparing; `v` and anything after a `-` ignored.
fn parse_version(version: &str) -> Option<Vec<u64>> {
    let version = version.trim().trim_start_matches('v');
    let version = version.split('-').next()?;
    version.split('.').map(|part| part.parse().ok()).collect()
}

fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

/// The `.app` bundle `exe` runs from, if it runs from one.
fn bundle_of(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .find(|dir| dir.extension().is_some_and(|ext| ext == "app"))
        .map(Path::to_owned)
}

fn running_bundle() -> Option<PathBuf> {
    bundle_of(&std::env::current_exe().ok()?.canonicalize().ok()?)
}

fn curl(args: &[&str]) -> Result<Vec<u8>> {
    // No overall limit, since a slow line takes minutes for the app; give up
    // only when the transfer stalls.
    let output = Command::new("/usr/bin/curl")
        .args([
            "-fsSL",
            "--connect-timeout",
            "20",
            "--speed-limit",
            "1024",
            "--speed-time",
            "60",
        ])
        .args(args)
        .output()
        .context("couldn't run curl")?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(output.stdout)
}

/// The latest release, when it's newer than this build.
fn latest() -> Result<Option<Release>> {
    #[derive(serde::Deserialize)]
    struct Latest {
        tag_name: String,
        #[serde(default)]
        body: String,
        #[serde(default)]
        assets: Vec<Asset>,
    }
    #[derive(serde::Deserialize)]
    struct Asset {
        name: String,
        size: u64,
    }
    let json = curl(&[&format!(
        "https://api.github.com/repos/{REPO}/releases/latest"
    )])?;
    let latest: Latest = serde_json::from_slice(&json)?;
    let version = latest.tag_name.trim_start_matches('v').to_owned();
    let size = latest
        .assets
        .iter()
        .find(|asset| asset.name == ASSET)
        .map_or(0, |asset| asset.size);
    Ok(
        is_newer(&version, env!("CARGO_PKG_VERSION")).then_some(Release {
            version,
            notes: plain_notes(&latest.body),
            size,
        }),
    )
}

fn run(program: &str, args: &[&str], dir: &Path) -> Result<()> {
    let output = Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .with_context(|| format!("couldn't run {program}"))?;
    if !output.status.success() {
        bail!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Changelog markdown as plain text for a dialog: each bullet on one line,
/// without emphasis or code marks.
fn plain_notes(markdown: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in markdown.lines() {
        let line = line.replace("**", "").replace('`', "");
        match lines.last_mut() {
            // A wrapped continuation of the bullet above.
            Some(last) if line.starts_with("  ") && !last.is_empty() => {
                last.push(' ');
                last.push_str(line.trim());
            }
            _ => lines.push(line.trim_end().to_owned()),
        }
    }
    lines.join("\n").trim().to_owned()
}

/// Where an install downloads to; the download's size there is its progress.
fn work_dir() -> PathBuf {
    std::env::temp_dir().join(format!("yin-update-{}", std::process::id()))
}

/// A scratch folder for one install, removed when done.
struct WorkDir(PathBuf);

impl WorkDir {
    fn new() -> Result<Self> {
        let dir = work_dir();
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        std::fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// Downloads `version`, checks it against its published checksum, and puts
/// it where `bundle` is. The running app keeps its files until it quits.
fn install(version: &str, bundle: &Path) -> Result<()> {
    let work = WorkDir::new()?;
    let base = format!("https://github.com/{REPO}/releases/download/v{version}");
    for name in [format!("{ASSET}.sha256"), ASSET.to_owned()] {
        let file = work.path().join(&name);
        curl(&["-o", &file.to_string_lossy(), &format!("{base}/{name}")])?;
    }
    run(
        "/usr/bin/shasum",
        &["-a", "256", "-c", &format!("{ASSET}.sha256")],
        work.path(),
    )
    .map_err(|_| anyhow!("the download doesn't match its checksum"))?;
    run("/usr/bin/ditto", &["-x", "-k", ASSET, "."], work.path())?;
    let fresh = work.path().join("yin.app");
    if !fresh.is_dir() {
        bail!("the download has no yin.app");
    }
    // Not notarized: without this, Gatekeeper refuses to open it.
    run(
        "/usr/bin/xattr",
        &["-dr", "com.apple.quarantine", "yin.app"],
        work.path(),
    )
    .ok();
    let old = bundle.with_extension("app.old");
    if old.exists() {
        std::fs::remove_dir_all(&old)?;
    }
    std::fs::rename(bundle, &old).context("couldn't move the current app aside")?;
    if let Err(error) = std::fs::rename(&fresh, bundle) {
        std::fs::rename(&old, bundle).ok();
        return Err(error).context("couldn't put the new app in place");
    }
    std::fs::remove_dir_all(&old).ok();
    Ok(())
}

/// Starts the check at launch and repeats it every few hours.
pub fn init(cx: &mut App) {
    cx.set_global(Updates::default());
    if cfg!(test) || running_bundle().is_none() {
        return;
    }
    cx.spawn(async |cx| {
        loop {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            check(false, cx);
            cx.background_executor().timer(CHECK_EVERY).await;
        }
    })
    .detach();
}

/// Asks GitHub for a newer release. With `report`, says what it found even
/// when there's nothing new, for an explicit "Check for updates".
fn check(report: bool, cx: &mut gpui_kit::AsyncApp) {
    let found = cx.background_spawn(async { latest() });
    cx.spawn(async move |cx| {
        let found = found.await;
        cx.update(|cx| {
            match &found {
                Ok(release) => {
                    cx.global_mut::<Updates>().available = release.clone();
                    cx.refresh_windows();
                }
                Err(error) => eprintln!("yin: update check: {error}"),
            }
            if report {
                let message = match found {
                    Ok(Some(release)) => {
                        Notification::info(format!("yin {} is available.", release.version))
                    }
                    Ok(None) => Notification::info(format!(
                        "yin {} is the latest version.",
                        env!("CARGO_PKG_VERSION")
                    )),
                    Err(error) => {
                        Notification::error(format!("Couldn't check for updates: {error}"))
                    }
                };
                notify(message, cx);
            }
        });
    })
    .detach();
}

/// "Check for updates" from the palette.
pub fn check_now(cx: &mut App) {
    if running_bundle().is_none() {
        notify(
            Notification::info("This is a development build; it doesn't update itself."),
            cx,
        );
        return;
    }
    cx.spawn(async |cx| check(true, cx)).detach();
}

fn notify(message: Notification, cx: &mut App) {
    if let Some((_, window)) = crate::windows::frontmost(cx) {
        window
            .update(cx, |_, window, cx| window.push_notification(message, cx))
            .ok();
    }
}

/// The title bar's update button, when a newer release is out.
pub fn render_button(cx: &App) -> Option<impl IntoElement> {
    let updates = cx.try_global::<Updates>()?;
    let release = updates.available.clone()?;
    let installing = updates.installing;
    Some(
        Button::new("update")
            .cursor_pointer()
            .primary()
            .xsmall()
            .label(if installing {
                match updates.progress {
                    Some(percent) => format!("Updating… {percent}%"),
                    None => "Updating…".to_owned(),
                }
            } else {
                format!("Update to {}", release.version)
            })
            .loading(installing)
            .on_click(move |_, window, cx| {
                if !installing {
                    confirm(release.clone(), window, cx)
                }
            }),
    )
}

/// Shows what's new, then installs and restarts on confirmation.
fn confirm(release: Release, window: &mut Window, cx: &mut App) {
    window.open_alert_dialog(cx, move |alert, _, _| {
        let release = release.clone();
        alert
            .title(format!("Update to yin {}?", release.version))
            .description(if release.notes.is_empty() {
                "yin restarts to finish; windows reopen where they were.".to_owned()
            } else {
                format!(
                    "{}\n\nyin restarts to finish; windows reopen where they were.",
                    release.notes
                )
            })
            .confirm()
            .ok_text("Update and restart")
            .on_ok(move |_, _, cx| {
                start_install(release.version.clone(), cx);
                true
            })
    });
}

fn start_install(version: String, cx: &mut App) {
    let Some(bundle) = running_bundle() else {
        return;
    };
    let size = cx
        .global::<Updates>()
        .available
        .as_ref()
        .map_or(0, |release| release.size);
    cx.global_mut::<Updates>().installing = true;
    cx.refresh_windows();
    let task = cx.background_spawn({
        let bundle = bundle.clone();
        async move { install(&version, &bundle) }
    });
    // Watch the download grow, for the button's percentage.
    if size > 0 {
        cx.spawn(async move |cx| {
            let download = work_dir().join(ASSET);
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                let installing = cx.update(|cx| {
                    let arrived = std::fs::metadata(&download).map_or(0, |meta| meta.len());
                    let percent = (arrived * 100 / size).min(100) as u8;
                    let updates = cx.global_mut::<Updates>();
                    if updates.installing && updates.progress != Some(percent) {
                        updates.progress = Some(percent);
                        cx.refresh_windows();
                    }
                    cx.global::<Updates>().installing
                });
                if !installing {
                    break;
                }
            }
        })
        .detach();
    }
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| {
            cx.global_mut::<Updates>().installing = false;
            cx.global_mut::<Updates>().progress = None;
            match result {
                Ok(()) => {
                    cx.global_mut::<Updates>().available = None;
                    restart(&bundle, cx);
                }
                Err(error) => {
                    cx.refresh_windows();
                    notify(Notification::error(format!("Couldn't update: {error}")), cx);
                }
            }
        });
    })
    .detach();
}

/// Opens the new app once this one has quit, then quits (asking first about
/// unsaved edits; the new app opens whenever this one finally exits).
fn restart(bundle: &Path, cx: &mut App) {
    let script = format!(
        "while kill -0 {pid} 2>/dev/null; do sleep 0.2; done; open \"{bundle}\"",
        pid = std::process::id(),
        bundle = bundle.display(),
    );
    if let Err(error) = Command::new("/bin/sh").args(["-c", &script]).spawn() {
        notify(
            Notification::warning(format!("Updated; restart yin to use it ({error})")),
            cx,
        );
        return;
    }
    crate::windows::quit(cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.5.1", "0.5.0"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(!is_newer("0.5.0", "0.5.0"));
        assert!(!is_newer("0.4.9", "0.5.0"));
        assert!(!is_newer("garbage", "0.5.0"));
        assert_eq!(parse_version("1.2.3-beta"), Some(vec![1, 2, 3]));
    }

    #[test]
    fn notes_read_as_plain_text() {
        let notes =
            "- **Stacks.** When the branch sits on `main`, the\n  stack shows.\n- Fixed: a bug.";
        assert_eq!(
            plain_notes(notes),
            "- Stacks. When the branch sits on main, the stack shows.\n- Fixed: a bug."
        );
    }

    #[test]
    fn finds_the_running_bundle() {
        assert_eq!(
            bundle_of(Path::new(
                "/Users/me/Applications/yin.app/Contents/MacOS/yin"
            )),
            Some(PathBuf::from("/Users/me/Applications/yin.app"))
        );
        assert_eq!(
            bundle_of(Path::new("/Users/me/Code/yin/target/debug/yin")),
            None
        );
    }
}
