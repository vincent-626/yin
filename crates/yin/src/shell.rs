//! The window's root view: a repository workspace, or the welcome screen.

use gpui_kit::component::{
    ActiveTheme as _, TitleBar, WindowExt as _,
    button::{Button, ButtonVariant, ButtonVariants as _},
    kbd::Kbd,
    notification::Notification,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    KeyBinding, Menu, MenuItem, ParentElement as _, PathPromptOptions, Render, Styled as _, Window,
    div, prelude::FluentBuilder as _,
};
use yin_git::Repository;

use crate::state;
use crate::workspace::{OpenRepository, Workspace};

mod actions {
    gpui_kit::actions!(yin, [Open, Quit]);
}
pub use actions::*;

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-o", Open, None),
        KeyBinding::new("cmd-q", Quit, None),
    ]);
    // Quit can arrive with focus outside the Shell (an overlay, the menu bar):
    // route it to the Shell so unsaved edits are still checked.
    cx.on_action(|_: &Quit, cx| {
        let Some(ShellHandle(shell, window)) = cx.try_global::<ShellHandle>().cloned() else {
            return cx.quit();
        };
        let routed = window.update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| shell.quit(window, cx)).is_ok()
        });
        if !matches!(routed, Ok(true)) {
            cx.quit();
        }
    });
    cx.set_menus([
        Menu::new("yin").items([MenuItem::action("Quit yin", Quit)]),
        Menu::new("File").items([MenuItem::action("Open Repository…", Open)]),
    ]);
}

pub struct Shell {
    workspace: Option<Entity<Workspace>>,
    focus: FocusHandle,
    _save_on_quit: gpui_kit::Subscription,
}

/// The window's Shell, for app-level actions that need it.
#[derive(Clone)]
struct ShellHandle(gpui_kit::WeakEntity<Shell>, gpui_kit::AnyWindowHandle);

impl gpui_kit::Global for ShellHandle {}

impl Shell {
    pub fn new(
        repo: Option<Repository>,
        keymap_errors: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        if !keymap_errors.is_empty() {
            let message = format!("Keymap problems:\n{}", keymap_errors.join("\n"));
            window.defer(cx, move |window, cx| {
                window.push_notification(Notification::warning(message), cx)
            });
        }
        let focus = cx.focus_handle();
        let handle = ShellHandle(cx.entity().downgrade(), window.window_handle());
        cx.set_global(handle);
        let shell = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            let unsaved = shell
                .read_with(cx, |shell, cx| shell.unsaved_files(cx))
                .unwrap_or_default();
            if unsaved.is_empty() {
                shell.update(cx, |shell, cx| shell.save_session(cx)).ok();
                return true;
            }
            confirm_discard(unsaved, window, cx, |_, cx| cx.quit());
            false
        });
        let mut this = Self {
            workspace: None,
            focus,
            // Every way out (menu, cmd-q, closing the window) ends here.
            _save_on_quit: cx.on_app_quit(|this, cx| {
                this.save_session(cx);
                async {}
            }),
        };
        match repo {
            Some(repo) => this.open_repo(repo, window, cx),
            None => {
                window.set_window_title("yin");
                this.focus.focus(window, cx);
            }
        }
        this
    }

    fn unsaved_files(&self, cx: &App) -> Vec<String> {
        self.workspace
            .as_ref()
            .map(|workspace| workspace.read(cx).unsaved_files())
            .unwrap_or_default()
    }

    fn save_session(&self, cx: &mut App) {
        if let Some(workspace) = &self.workspace {
            workspace.read(cx).save_session(cx);
        }
    }

    fn quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_session(cx);
        let unsaved = self.unsaved_files(cx);
        if unsaved.is_empty() {
            cx.quit();
        } else {
            confirm_discard(unsaved, window, cx, |_, cx| cx.quit());
        }
    }

    fn open_repo(&mut self, repo: Repository, window: &mut Window, cx: &mut Context<Self>) {
        self.save_session(cx);
        state::save_last_repo(repo.workdir());
        let workspace = cx.new(|cx| Workspace::new(repo, window, cx));
        cx.subscribe_in(
            &workspace,
            window,
            |this, _, event: &OpenRepository, window, cx| {
                this.switch_to(event.0.clone(), window, cx)
            },
        )
        .detach();
        self.workspace = Some(workspace);
        cx.notify();
    }

    /// Opens `path` in this window, asking first about unsaved edits.
    fn switch_to(&mut self, path: std::path::PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let repo = match Repository::open(&path) {
            Ok(repo) => repo,
            Err(error) => {
                return window.push_notification(Notification::error(error.to_string()), cx);
            }
        };
        let unsaved = self.unsaved_files(cx);
        if unsaved.is_empty() {
            self.open_repo(repo, window, cx);
        } else {
            let shell = cx.entity().downgrade();
            confirm_discard(unsaved, window, cx, move |window, cx| {
                shell
                    .update(cx, |this, cx| this.open_repo(repo.clone(), window, cx))
                    .ok();
            });
        }
    }

    fn prompt_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            this.update_in(cx, |this, window, cx| this.switch_to(path, window, cx))
                .ok();
        })
        .detach();
    }

    fn render_welcome(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().size_full().child(TitleBar::new()).child(
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_4()
                .child(
                    div()
                        .text_2xl()
                        .font_weight(gpui_kit::FontWeight::LIGHT)
                        .child("yin"),
                )
                .child(
                    Button::new("open")
                        .primary()
                        .label("Open Repository…")
                        .on_click(cx.listener(|this, _, window, cx| this.prompt_open(window, cx))),
                )
                .when_some(Kbd::binding_for_action(&Open, None, window), |this, kbd| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(kbd),
                    )
                }),
        )
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("shell")
            .track_focus(&self.focus)
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(|this, _: &Open, window, cx| this.prompt_open(window, cx)))
            .on_action(cx.listener(|this, _: &Quit, window, cx| this.quit(window, cx)))
            .map(|this| match &self.workspace {
                Some(workspace) => this.child(workspace.clone()),
                None => this.child(self.render_welcome(window, cx)),
            })
    }
}

/// Asks before an action that would lose unsaved edits, then runs it.
fn confirm_discard(
    unsaved: Vec<String>,
    window: &mut Window,
    cx: &mut App,
    then: impl Fn(&mut Window, &mut App) + 'static,
) {
    let then = std::rc::Rc::new(then);
    let description = match unsaved.as_slice() {
        [path] => format!("Your edits to {path} will be lost."),
        paths => format!("Your edits to {} files will be lost.", paths.len()),
    };
    window.open_alert_dialog(cx, move |alert, _, _| {
        let then = then.clone();
        alert
            .title("Discard unsaved changes?")
            .description(description.clone())
            .confirm()
            .ok_text("Discard")
            .ok_variant(ButtonVariant::Danger)
            .on_ok(move |_, window, cx| {
                then(window, cx);
                true
            })
    });
}
