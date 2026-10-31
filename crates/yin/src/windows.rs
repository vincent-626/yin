//! yin's windows, one per repository. Opening a repository that already has a
//! window brings that window forward instead of opening it twice.

use std::path::{Path, PathBuf};

use gpui_kit::component::{TitleBar, WindowExt as _, notification::Notification};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Entity, Global, WeakEntity, WindowBounds,
    WindowOptions, point, px, size,
};
use yin_git::Repository;

use crate::shell::{self, Shell};
use crate::state;

/// Every window's Shell, most recently active first.
#[derive(Default)]
struct Windows(Vec<(WeakEntity<Shell>, AnyWindowHandle)>);

impl Global for Windows {}

/// Adds a new window, in front.
pub fn register(shell: WeakEntity<Shell>, window: AnyWindowHandle, cx: &mut App) {
    cx.default_global::<Windows>().0.insert(0, (shell, window));
}

/// Moves `window` to the front, when it becomes active.
pub fn touch(window: AnyWindowHandle, cx: &mut App) {
    let windows = &mut cx.default_global::<Windows>().0;
    if let Some(ix) = windows.iter().position(|(_, handle)| *handle == window) {
        let entry = windows.remove(ix);
        windows.insert(0, entry);
    }
}

/// The windows still open, most recently active first.
fn live(cx: &mut App) -> Vec<(Entity<Shell>, AnyWindowHandle)> {
    let open = cx.windows();
    let windows = &mut cx.default_global::<Windows>().0;
    windows.retain(|(shell, handle)| shell.upgrade().is_some() && open.contains(handle));
    windows
        .iter()
        .filter_map(|(shell, handle)| Some((shell.upgrade()?, *handle)))
        .collect()
}

/// The repositories that have a window.
pub fn open_repos(cx: &mut App) -> Vec<PathBuf> {
    live(cx)
        .into_iter()
        .filter_map(|(shell, _)| shell.read(cx).workdir(cx))
        .collect()
}

/// The window to act in: the active one, else the last one active.
pub fn frontmost(cx: &mut App) -> Option<(Entity<Shell>, AnyWindowHandle)> {
    let windows = live(cx);
    let active = cx.active_window();
    windows
        .iter()
        .find(|(_, handle)| Some(*handle) == active)
        .or(windows.first())
        .cloned()
}

fn find(workdir: &Path, cx: &mut App) -> Option<AnyWindowHandle> {
    live(cx)
        .into_iter()
        .find(|(shell, _)| shell.read(cx).workdir(cx).as_deref() == Some(workdir))
        .map(|(_, handle)| handle)
}

fn activate(window: AnyWindowHandle, cx: &mut App) {
    window
        .update(cx, |_, window, _| window.activate_window())
        .ok();
    cx.activate(true);
}

/// Shows `path`'s repository: its window if it has one, else the welcome
/// window if one is open, else a new window. `None` opens a welcome window.
pub fn open(path: Option<PathBuf>, cx: &mut App) {
    let repo = match path.map(|path| Repository::open(&path)).transpose() {
        Ok(repo) => repo,
        Err(error) => return show_error(error.to_string(), cx),
    };
    let Some(repo) = repo else {
        open_window(None, cx);
        return;
    };
    if let Some(window) = find(repo.workdir(), cx) {
        return activate(window, cx);
    }
    let welcome = live(cx)
        .into_iter()
        .find(|(shell, _)| shell.read(cx).workdir(cx).is_none());
    if let Some((shell, window)) = welcome {
        window
            .update(cx, |_, window, cx| {
                shell.update(cx, |shell, cx| shell.open_repo(repo, window, cx))
            })
            .ok();
        return activate(window, cx);
    }
    open_window(Some(repo), cx);
}

/// At launch, or when the Dock icon is clicked with no windows open: the
/// working directory's repository, else the last one opened.
pub fn open_default(cx: &mut App) {
    if !live(cx).is_empty() {
        return;
    }
    let cwd = std::env::current_dir().ok();
    let repo = [cwd, state::last_repo()]
        .into_iter()
        .flatten()
        .find(|path| Repository::open(path).is_ok());
    open(repo, cx);
}

fn open_window(repo: Option<Repository>, cx: &mut App) {
    let options = WindowOptions {
        window_bounds: Some(next_bounds(cx)),
        app_id: Some("yin".into()),
        ..TitleBar::window_options()
    };
    if let Err(error) = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| Shell::new(repo, window, cx))
    }) {
        eprintln!("yin: couldn't open a window: {error}");
    }
    cx.activate(true);
}

/// Reports `message` in the frontmost window, opening one if there's none.
pub fn show_error(message: String, cx: &mut App) {
    if frontmost(cx).is_none() {
        open_window(None, cx);
    }
    if let Some((_, window)) = frontmost(cx) {
        window
            .update(cx, |_, window, cx| {
                window.push_notification(Notification::error(message), cx)
            })
            .ok();
        activate(window, cx);
    }
}

/// Saves every window's session, then quits, asking first when files have
/// unsaved edits.
pub fn quit(cx: &mut App) {
    let windows = live(cx);
    let mut unsaved = Vec::new();
    let mut ask_in = None;
    for (shell, handle) in &windows {
        shell.read(cx).save_session(cx);
        let files = shell.read(cx).unsaved_files(cx);
        if !files.is_empty() {
            ask_in.get_or_insert(*handle);
            unsaved.extend(files);
        }
    }
    let Some(window) = ask_in else {
        return cx.quit();
    };
    window
        .update(cx, |_, window, cx| {
            window.activate_window();
            shell::confirm_discard(unsaved, window, cx, |_, cx| cx.quit());
        })
        .ok();
}

/// A new window steps down and right from the frontmost one; the first goes
/// where the last window was, if that still fits a display.
fn next_bounds(cx: &mut App) -> WindowBounds {
    if let Some((_, window)) = frontmost(cx)
        && let Ok(bounds) = window.update(cx, |_, window, _| window.bounds())
    {
        let offset = point(px(28.), px(28.));
        return WindowBounds::Windowed(Bounds {
            origin: bounds.origin + offset,
            size: bounds.size,
        });
    }
    initial_bounds(cx)
}

fn initial_bounds(cx: &App) -> WindowBounds {
    if let Some(frame) = state::load_window_frame() {
        let bounds = Bounds {
            origin: point(px(frame.x), px(frame.y)),
            size: size(px(frame.width), px(frame.height)),
        };
        let visible = cx.displays().iter().any(|display| {
            let screen = display.bounds();
            screen.contains(&bounds.center())
        });
        if visible && frame.width >= 600. && frame.height >= 400. {
            return WindowBounds::Windowed(bounds);
        }
    }
    // Fit the usable screen: a window macOS has to shrink keeps its first
    // layout and stops redrawing.
    let Some(screen) = cx.primary_display().map(|display| display.visible_bounds()) else {
        return WindowBounds::centered(size(px(1400.), px(900.)), cx);
    };
    let size = size(
        px(1400.).min(screen.size.width * 0.92),
        px(900.).min(screen.size.height * 0.92),
    );
    WindowBounds::Windowed(Bounds::centered_at(screen.center(), size))
}
