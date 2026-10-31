//! A window's root view: a repository workspace, or the welcome screen.

mod switcher;

use std::path::PathBuf;

use gpui_kit::component::{
    ActiveTheme as _, TitleBar, WindowExt as _,
    button::{Button, ButtonVariant, ButtonVariants as _},
    command::CommandState,
    kbd::Kbd,
    notification::Notification,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    KeyBinding, Menu, MenuItem, ParentElement as _, PathPromptOptions, Render,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _,
};
use yin_git::Repository;

use crate::workspace::{OpenRepository, Workspace};
use crate::{state, windows};

mod actions {
    gpui_kit::actions!(yin, [Open, Quit, SwitchRepository]);
}
pub use actions::*;

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-o", SwitchRepository, None),
        KeyBinding::new("cmd-shift-o", Open, None),
        KeyBinding::new("cmd-q", Quit, None),
    ]);
    // Quit can arrive with focus anywhere, or with no window at all.
    cx.on_action(|_: &Quit, cx| windows::quit(cx));
    cx.set_menus([
        Menu::new("yin").items([MenuItem::action("Quit yin", Quit)]),
        Menu::new("File").items([
            MenuItem::action("Switch Repository…", SwitchRepository),
            MenuItem::action("Open Folder…", Open),
        ]),
    ]);
}

pub struct Shell {
    workspace: Option<Entity<Workspace>>,
    focus: FocusHandle,
    _save_on_quit: gpui_kit::Subscription,
    /// The window's frame, kept current to save on quit.
    frame: Option<state::WindowFrame>,
    switcher: Entity<CommandState>,
    /// What the repository switcher offers, recent ones first.
    switcher_entries: Vec<switcher::Entry>,
    /// Repositories that appeared on the welcome screen, for clicks.
    recent: Vec<PathBuf>,
}

impl Shell {
    pub fn new(repo: Option<Repository>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        windows::register(cx.entity().downgrade(), window.window_handle(), cx);
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                windows::touch(window.window_handle(), cx);
            } else if let Some(frame) = this.frame {
                state::save_window_frame(frame);
            }
        })
        .detach();
        let shell = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            let unsaved = shell
                .read_with(cx, |shell, cx| shell.unsaved_files(cx))
                .unwrap_or_default();
            shell.update(cx, |shell, cx| shell.save_session(cx)).ok();
            if unsaved.is_empty() {
                return true;
            }
            confirm_discard(unsaved, window, cx, |window, _| window.remove_window());
            false
        });
        let mut this = Self {
            workspace: None,
            focus,
            // Every way out (menu, cmd-q, closing the window) ends here.
            _save_on_quit: cx.on_app_quit(|this, cx| {
                this.save_session(cx);
                if let Some(frame) = this.frame {
                    state::save_window_frame(frame);
                }
                async {}
            }),
            frame: None,
            switcher: cx.new(|cx| CommandState::new(window, cx)),
            switcher_entries: Vec::new(),
            recent: state::recent_repos(),
        };
        cx.observe_window_bounds(window, |this, window, _| {
            // Fullscreen and maximized frames aren't worth restoring as windows.
            if let gpui_kit::WindowBounds::Windowed(bounds) = window.window_bounds() {
                this.frame = Some(state::WindowFrame {
                    x: bounds.origin.x.into(),
                    y: bounds.origin.y.into(),
                    width: bounds.size.width.into(),
                    height: bounds.size.height.into(),
                });
            }
        })
        .detach();
        match repo {
            Some(repo) => this.open_repo(repo, window, cx),
            None => {
                window.set_window_title("yin");
                this.focus.focus(window, cx);
            }
        }
        this
    }

    /// The repository this window shows, if any.
    pub fn workdir(&self, cx: &App) -> Option<PathBuf> {
        self.workspace
            .as_ref()
            .map(|workspace| workspace.read(cx).workdir().to_owned())
    }

    pub fn unsaved_files(&self, cx: &App) -> Vec<String> {
        self.workspace
            .as_ref()
            .map(|workspace| workspace.read(cx).unsaved_files())
            .unwrap_or_default()
    }

    pub fn save_session(&self, cx: &App) {
        if let Some(workspace) = &self.workspace {
            workspace.read(cx).save_session(cx);
        }
    }

    pub fn open_repo(&mut self, repo: Repository, window: &mut Window, cx: &mut Context<Self>) {
        self.save_session(cx);
        state::remember_repo(repo.workdir());
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

    /// Shows another worktree of this repository: in its own window if it
    /// has one, else in this window, asking first about unsaved edits.
    fn switch_to(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let repo = match Repository::open(&path) {
            Ok(repo) => repo,
            Err(error) => {
                return window.push_notification(Notification::error(error.to_string()), cx);
            }
        };
        if windows::open_repos(cx)
            .iter()
            .any(|open| open == repo.workdir())
        {
            return windows::open(Some(path), cx);
        }
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

    /// Opens a folder chosen in the system dialog.
    fn prompt_open(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open".into()),
        });
        cx.spawn(async move |_, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            cx.update(|cx| windows::open(Some(path), cx));
        })
        .detach();
    }

    fn render_welcome(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
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
                .when(!self.recent.is_empty(), |this| {
                    this.child(v_flex().w_80().gap_0p5().children(
                        self.recent.iter().take(8).enumerate().map(|(ix, path)| {
                            let open = path.clone();
                            v_flex()
                                .id(("recent", ix))
                                .px_3()
                                .py_1p5()
                                .rounded_md()
                                .cursor_pointer()
                                .hover(|this| this.bg(cx.theme().secondary))
                                .on_click(move |_, _, cx| windows::open(Some(open.clone()), cx))
                                .child(div().text_sm().child(repo_name(path)))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(muted)
                                        .truncate()
                                        .child(state::tilde(path)),
                                )
                        }),
                    ))
                })
                .child(
                    Button::new("open")
                        .primary()
                        .label("Open Repository…")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_switcher(window, cx)),
                        ),
                )
                .when_some(
                    Kbd::binding_for_action(&SwitchRepository, None, window),
                    |this, kbd| this.child(div().text_xs().text_color(muted).child(kbd)),
                ),
        )
    }
}

fn repo_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("shell")
            .track_focus(&self.focus)
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(|this, _: &Open, _, cx| this.prompt_open(cx)))
            .on_action(
                cx.listener(|this, _: &SwitchRepository, window, cx| {
                    this.open_switcher(window, cx)
                }),
            )
            .map(|this| match &self.workspace {
                Some(workspace) => this.child(workspace.clone()),
                None => this.child(self.render_welcome(window, cx)),
            })
    }
}

/// Asks before an action that would lose unsaved edits, then runs it.
pub fn confirm_discard(
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
