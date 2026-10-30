mod assets;
mod changes;
mod workspace;

use std::path::PathBuf;

use gpui_kit::component::{Theme, ThemeMode, TitleBar};
use gpui_kit::*;
use yin_git::Repository;

use workspace::Workspace;

fn main() {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let repo = match Repository::open(&path) {
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
            workspace::init(cx);
            // Always yin: dark only, never follow the system appearance.
            Theme::change(ThemeMode::Dark, None, cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1400.), px(900.)), cx)),
                app_id: Some("yin".into()),
                ..TitleBar::window_options()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| Workspace::new(repo, window, cx))
            })
            .expect("failed to open window");
            cx.activate(true);
        });
}
