mod assets;
mod changes;
mod keymap;
mod review;
mod shell;
mod state;
#[cfg(test)]
mod tests;
mod watcher;
mod workspace;

use std::path::PathBuf;

use gpui_kit::component::{Theme, ThemeMode, TitleBar};
use gpui_kit::*;
use shell::Shell;

fn main() {
    let argument = std::env::args_os().nth(1).map(PathBuf::from);
    let repo = match state::initial_repo(argument, std::env::current_dir().ok(), state::last_repo())
    {
        Ok(repo) => repo,
        Err(error) => {
            eprintln!("yin: {error}");
            std::process::exit(1);
        }
    };

    gpui_kit::application()
        .with_assets(assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            shell::init(cx);
            workspace::init(cx);
            let keymap_errors = keymap::load(cx);
            // Always yin: dark only, never follow the system appearance.
            Theme::change(ThemeMode::Dark, None, cx);
            // The dark theme gives popovers the editor's own background, so
            // hover cards and menus had no edge; lift them one step.
            Theme::update(cx, |theme| theme.popover = hsla(0., 0., 0.09, 1.));
            let options = WindowOptions {
                window_bounds: Some(initial_bounds(cx)),
                app_id: Some("yin".into()),
                ..TitleBar::window_options()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| Shell::new(repo, keymap_errors, window, cx))
            })
            .expect("failed to open window");
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.activate(true);
        });
}

/// The last window frame when it still fits on a connected display;
/// otherwise a centred default.
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
    WindowBounds::centered(size(px(1400.), px(900.)), cx)
}
