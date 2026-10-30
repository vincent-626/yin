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
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1400.), px(900.)), cx)),
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
