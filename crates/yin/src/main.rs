mod assets;
mod changes;
mod hotkey;
mod keymap;
mod notes;
mod review;
mod shell;
mod state;
#[cfg(test)]
mod tests;
mod watcher;
mod windows;
mod workspace;

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::component::{Theme, ThemeMode, WindowExt as _, notification::Notification};
use gpui_kit::*;

fn main() {
    let argument = std::env::args_os().nth(1).map(PathBuf::from);
    if let Some(path) = &argument
        && let Err(error) = yin_git::Repository::open(path)
    {
        eprintln!("yin: {error}");
        std::process::exit(1);
    }

    // Folders handed to the running app: `open -a yin <folder>` (the `yin`
    // command), or dropped on the Dock icon.
    let (opened, opened_rx) = async_channel::unbounded::<PathBuf>();
    let app = gpui_kit::application().with_assets(assets::Assets);
    app.on_open_urls(move |urls| {
        for path in urls.iter().filter_map(|url| file_url_path(url)) {
            opened.try_send(path).ok();
        }
    });
    app.on_reopen(windows::open_default);
    app.run(move |cx| {
        gpui_kit::init(cx);
        shell::init(cx);
        workspace::init(cx);
        let mut problems = keymap::load(cx);
        // Always yin: dark only, never follow the system appearance.
        Theme::change(ThemeMode::Dark, None, cx);
        // The dark theme gives popovers the editor's own background, so
        // hover cards and menus had no edge; lift them one step.
        Theme::update(cx, |theme| theme.popover = hsla(0., 0., 0.09, 1.));

        let (pressed, pressed_rx) = async_channel::unbounded::<()>();
        let registered = keymap::global_hotkey().and_then(|key| match key {
            Some(key) => hotkey::register(hotkey::Hotkey::parse(&key)?, move || {
                pressed.try_send(()).ok();
            }),
            None => Ok(()),
        });
        if let Err(error) = registered {
            problems.push(format!("Global hotkey: {error}"));
        }
        cx.spawn(async move |cx| {
            while pressed_rx.recv().await.is_ok() {
                cx.update(show_switcher);
            }
        })
        .detach();

        match argument {
            Some(path) => windows::open_now(Some(path), cx),
            // `open -a yin <folder>` launches with no arguments and delivers
            // the folder just after; give it a moment before opening the
            // usual window.
            None => cx
                .spawn(async |cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(250))
                        .await;
                    cx.update(windows::open_default);
                })
                .detach(),
        }
        cx.spawn(async move |cx| {
            while let Ok(path) = opened_rx.recv().await {
                cx.update(|cx| windows::open(Some(path), cx));
            }
        })
        .detach();
        if !problems.is_empty() {
            let message = problems.join("\n");
            eprintln!("yin: {message}");
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(400))
                    .await;
                cx.update(|cx| {
                    if let Some((_, window)) = windows::frontmost(cx) {
                        window
                            .update(cx, |_, window, cx| {
                                window.push_notification(Notification::warning(message), cx)
                            })
                            .ok();
                    }
                });
            })
            .detach();
        }
        cx.activate(true);
    });
}

/// From anywhere: yin comes forward with the repository switcher open in its
/// frontmost window, or a new window when none is open.
fn show_switcher(cx: &mut App) {
    windows::ensure_window(cx);
    let Some((shell, window)) = windows::frontmost(cx) else {
        return;
    };
    cx.activate(true);
    window
        .update(cx, |_, window, cx| {
            window.activate_window();
            shell.update(cx, |shell, cx| shell.open_switcher(window, cx));
        })
        .ok();
}

/// The path of a `file://` URL, decoding `%20` and the like.
fn file_url_path(url: &str) -> Option<PathBuf> {
    let encoded = url.strip_prefix("file://")?;
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut ix = 0;
    while ix < bytes.len() {
        let hex = bytes
            .get(ix + 1..ix + 3)
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match (bytes[ix], hex) {
            (b'%', Some(byte)) => {
                decoded.push(byte);
                ix += 3;
            }
            (byte, _) => {
                decoded.push(byte);
                ix += 1;
            }
        }
    }
    let path = String::from_utf8(decoded).ok()?;
    Some(
        PathBuf::from(path.trim_end_matches('/'))
            .components()
            .collect(),
    )
}

#[cfg(test)]
mod url_tests {
    use super::file_url_path;
    use std::path::PathBuf;

    #[test]
    fn decodes_file_urls() {
        assert_eq!(
            file_url_path("file:///Users/me/Code/my%20repo/"),
            Some(PathBuf::from("/Users/me/Code/my repo"))
        );
        assert_eq!(file_url_path("https://example.com"), None);
    }
}
