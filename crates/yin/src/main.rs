use gpui_kit::component::{ActiveTheme as _, Theme, ThemeMode, TitleBar};
use gpui_kit::*;

struct Yin;

impl Render for Yin {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(TitleBar::new().child("yin"))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child("yin"),
            )
    }
}

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            // Always yin: dark only, never follow the system appearance.
            Theme::change(ThemeMode::Dark, None, cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1400.), px(900.)), cx)),
                app_id: Some("yin".into()),
                ..TitleBar::window_options()
            };
            gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| Yin))
                .expect("failed to open window");
            cx.activate(true);
        });
}
