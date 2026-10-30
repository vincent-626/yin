use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, StyledExt as _,
    TitleBar, WindowExt as _,
    button::{Button, ButtonVariant, ButtonVariants as _},
    checkbox::Checkbox,
    diff::{Diff, DiffFile, DiffMode, DiffState},
    h_flex,
    input::{InputEvent, Textarea, TextareaState},
    list::ListItem,
    notification::Notification,
    resizable::{h_resizable, resizable_panel},
    scroll::Scrollbar,
    status_bar::StatusBar,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable as _, Hsla,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render, ScrollStrategy,
    SharedString, Styled as _, Task, UniformListScrollHandle, Window, div,
    prelude::FluentBuilder as _, uniform_list,
};
use yin_git::{Change, Repository, Status};

use crate::changes::{self, FileKey, Row, Section};
use crate::watcher::{self, RepoWatcher};

mod actions {
    gpui_kit::actions!(
        yin,
        [
            Refresh,
            ToggleSidebar,
            ToggleDiffMode,
            SelectNext,
            SelectPrevious,
            ToggleStaged,
            StageAll,
            UnstageAll,
            FocusDiff,
            FocusChanges,
            Commit,
            FocusCommitMessage,
            Discard,
        ]
    );
}
use actions::*;

const CHANGES_CONTEXT: &str = "Changes";

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-r", Refresh, Some("Workspace")),
        KeyBinding::new("cmd-b", ToggleSidebar, Some("Workspace")),
        KeyBinding::new("cmd-shift-d", ToggleDiffMode, Some("Workspace")),
        KeyBinding::new("cmd-alt-s", StageAll, Some("Workspace")),
        KeyBinding::new("cmd-alt-u", UnstageAll, Some("Workspace")),
        KeyBinding::new("]", SelectNext, Some("Workspace")),
        KeyBinding::new("[", SelectPrevious, Some("Workspace")),
        KeyBinding::new("escape", FocusChanges, Some("Workspace")),
        KeyBinding::new("down", SelectNext, Some(CHANGES_CONTEXT)),
        KeyBinding::new("j", SelectNext, Some(CHANGES_CONTEXT)),
        KeyBinding::new("up", SelectPrevious, Some(CHANGES_CONTEXT)),
        KeyBinding::new("k", SelectPrevious, Some(CHANGES_CONTEXT)),
        KeyBinding::new("space", ToggleStaged, Some(CHANGES_CONTEXT)),
        KeyBinding::new("enter", FocusDiff, Some(CHANGES_CONTEXT)),
        KeyBinding::new("cmd-enter", Commit, Some("Workspace")),
        KeyBinding::new("c", FocusCommitMessage, Some(CHANGES_CONTEXT)),
        KeyBinding::new("cmd-backspace", Discard, Some(CHANGES_CONTEXT)),
    ]);
}

pub struct Workspace {
    repo: Repository,
    status: Status,
    rows: Arc<Vec<Row>>,
    selected: Option<usize>,
    loaded: bool,
    /// The file the diff pane shows, and the patch it was built from.
    diff: Entity<DiffState>,
    diff_key: Option<FileKey>,
    diff_patch: String,
    diff_mode: DiffMode,
    error: Option<SharedString>,
    commit_message: Entity<TextareaState>,
    amend: bool,
    /// A git write is running.
    busy: bool,
    sidebar_visible: bool,
    changes_focus: FocusHandle,
    changes_scroll: UniformListScrollHandle,
    status_generation: usize,
    diff_generation: usize,
    _status_task: Task<()>,
    _diff_task: Task<()>,
    _operation_task: Task<()>,
    _watcher: Option<RepoWatcher>,
    _watch_task: Task<()>,
    _relayout_task: Task<()>,
}

impl Workspace {
    pub fn new(repo: Repository, window: &mut Window, cx: &mut Context<Self>) -> Self {
        window.set_window_title(&repo.name());
        let diff = cx.new(|cx| DiffState::new([], cx));
        cx.observe(&diff, |_, _, cx| cx.notify()).detach();
        cx.observe_window_bounds(window, |this, _, cx| this.schedule_relayout(cx))
            .detach();
        let changes_focus = cx.focus_handle().tab_stop(true);
        changes_focus.focus(window, cx);
        let commit_message = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Commit message")
                .auto_grow(3, 12)
        });
        cx.subscribe_in(
            &commit_message,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter {
                    secondary: true, ..
                } = event
                {
                    this.commit(window, cx);
                }
            },
        )
        .detach();
        let mut this = Self {
            repo,
            status: Status::default(),
            rows: Arc::default(),
            selected: None,
            loaded: false,
            diff,
            diff_key: None,
            diff_patch: String::new(),
            diff_mode: DiffMode::Unified,
            error: None,
            commit_message,
            amend: false,
            busy: false,
            sidebar_visible: true,
            changes_focus,
            changes_scroll: UniformListScrollHandle::new(),
            status_generation: 0,
            diff_generation: 0,
            _status_task: Task::ready(()),
            _diff_task: Task::ready(()),
            _operation_task: Task::ready(()),
            _watcher: None,
            _watch_task: Task::ready(()),
            _relayout_task: Task::ready(()),
        };
        // The test scheduler is deterministic and rejects wakeups from the
        // FSEvents thread; tests refresh after each operation anyway.
        if !cfg!(test) {
            this.watch(cx);
        }
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.refresh(cx);
            }
        })
        .detach();
        this.refresh(cx);
        this
    }

    /// Refreshes after filesystem changes, batching bursts such as a build or
    /// an agent rewriting many files.
    fn watch(&mut self, cx: &mut Context<Self>) {
        let (watcher, events) = match watcher::watch(&self.repo) {
            Ok(watch) => watch,
            Err(error) => {
                self.error = Some(format!("not watching for changes: {error}").into());
                return;
            }
        };
        self._watcher = Some(watcher);
        self._watch_task = cx.spawn(async move |this, cx| {
            while events.recv().await.is_ok() {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                while events.try_recv().is_ok() {}
                if this.update(cx, |this, cx| this.refresh(cx)).is_err() {
                    break;
                }
            }
        });
    }

    /// The Diff learns its viewport while painting, and a notify during paint
    /// doesn't schedule another frame, so rows can keep a stale width: the
    /// window opens at a default size and then grows, or the user resizes it.
    /// Redraw twice, once to measure and once to lay out at the new width.
    fn schedule_relayout(&mut self, cx: &mut Context<Self>) {
        self._relayout_task = cx.spawn(async move |this, cx| {
            for _ in 0..2 {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let Ok(()) = this.update(cx, |this, cx| {
                    this.diff.update(cx, |_, cx| cx.notify());
                }) else {
                    return;
                };
            }
        });
    }

    #[cfg(test)]
    pub fn selected_path(&self) -> Option<String> {
        self.selected_key().map(|key| key.path)
    }

    #[cfg(test)]
    pub fn commit_message_text(&self, cx: &App) -> String {
        self.commit_message.read(cx).value().to_string()
    }

    fn selected_key(&self) -> Option<FileKey> {
        self.rows.get(self.selected?)?.key()
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.status_generation += 1;
        let generation = self.status_generation;
        let repo = self.repo.clone();
        self._status_task = cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { repo.status() }).await;
            this.update(cx, |this, cx| {
                if this.status_generation != generation {
                    return;
                }
                match result {
                    Ok(status) => this.install_status(status, cx),
                    Err(error) => this.error = Some(error.to_string().into()),
                }
                cx.notify();
            })
            .ok();
        });
    }

    fn install_status(&mut self, status: Status, cx: &mut Context<Self>) {
        let previous = self.selected_key();
        let rows = changes::rows(&status);
        self.selected = changes::reselect(&rows, previous.as_ref(), self.selected);
        self.rows = Arc::new(rows);
        self.status = status;
        self.loaded = true;
        // Reload even when the selection is unchanged: the file may have changed.
        self.load_diff(cx);
    }

    fn select(&mut self, ix: Option<usize>, cx: &mut Context<Self>) {
        if ix == self.selected {
            return;
        }
        self.selected = ix;
        if let Some(ix) = ix {
            self.changes_scroll
                .scroll_to_item(ix, ScrollStrategy::Nearest);
        }
        self.load_diff(cx);
        cx.notify();
    }

    fn load_diff(&mut self, cx: &mut Context<Self>) {
        self.diff_generation += 1;
        let generation = self.diff_generation;
        let Some(key) = self.selected_key() else {
            self.diff_key = None;
            self.diff_patch.clear();
            self.diff.update(cx, |state, cx| state.set_files([], cx));
            return;
        };
        let repo = self.repo.clone();
        self._diff_task = cx.spawn(async move |this, cx| {
            let target = key.section.diff_target();
            let path = key.path.clone();
            let result = cx
                .background_spawn(async move { repo.diff(&path, target) })
                .await;
            this.update(cx, |this, cx| {
                if this.diff_generation != generation {
                    return;
                }
                this.install_diff(key, result, cx)
            })
            .ok();
        });
    }

    fn install_diff(
        &mut self,
        key: FileKey,
        result: anyhow::Result<String>,
        cx: &mut Context<Self>,
    ) {
        let patch = match result {
            Ok(patch) => patch,
            Err(error) => {
                self.error = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        // Same file, same patch: keep the scroll position and selection.
        if self.diff_key.as_ref() == Some(&key) && self.diff_patch == patch {
            return;
        }
        let files = match DiffFile::parse(&patch) {
            Ok(files) => files,
            Err(error) => {
                self.error = Some(format!("couldn't read diff of {}: {error}", key.path).into());
                Vec::new()
            }
        };
        self.diff_key = Some(key);
        self.diff_patch = patch;
        self.diff.update(cx, |state, cx| state.set_files(files, cx));
        self.schedule_relayout(cx);
        cx.notify();
    }

    /// Runs a git write off the main thread, then refreshes.
    fn run(
        &mut self,
        operation: impl FnOnce(&Repository) -> anyhow::Result<()> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_then(operation, |_, _, _| {}, window, cx);
    }

    /// Like [`Self::run`], calling `on_success` when the operation succeeds.
    /// Failures, such as hook output, become an error notification.
    fn run_then(
        &mut self,
        operation: impl FnOnce(&Repository) -> anyhow::Result<()> + Send + 'static,
        on_success: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo = self.repo.clone();
        self.busy = true;
        cx.notify();
        self._operation_task = cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_spawn(async move { operation(&repo) }).await;
            this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(()) => on_success(this, window, cx),
                    Err(error) => {
                        window.push_notification(Notification::error(error.to_string()), cx)
                    }
                }
                this.refresh(cx);
                cx.notify();
            })
            .ok();
        });
    }

    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let message = self.commit_message.read(cx).value().trim().to_string();
        let amend = self.amend;
        if self.busy || message.is_empty() || (!amend && self.status.staged().next().is_none()) {
            return;
        }
        self.run_then(
            move |repo| repo.commit(&message, amend),
            |this, window, cx| {
                this.amend = false;
                this.commit_message
                    .update(cx, |state, cx| state.set_value("", window, cx));
                this.changes_focus.focus(window, cx);
            },
            window,
            cx,
        );
    }

    fn toggle_amend(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.amend = !self.amend;
        cx.notify();
        if !self.amend || !self.commit_message.read(cx).value().trim().is_empty() {
            return;
        }
        // Start from HEAD's message, as `git commit --amend` would.
        let repo = self.repo.clone();
        cx.spawn_in(window, async move |this, cx| {
            let message = cx
                .background_spawn(async move { repo.head_message() })
                .await;
            this.update_in(cx, |this, window, cx| {
                if let Ok(message) = message
                    && this.amend
                    && this.commit_message.read(cx).value().is_empty()
                {
                    this.commit_message
                        .update(cx, |state, cx| state.set_value(message, window, cx));
                }
            })
            .ok();
        })
        .detach();
    }

    fn toggle_staged_at(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.rows.get(ix).and_then(Row::key) else {
            return;
        };
        self.run(
            move |repo| {
                if key.section.is_staged() {
                    repo.unstage(&[&key.path])
                } else {
                    repo.stage(&[&key.path])
                }
            },
            window,
            cx,
        );
    }

    fn toggle_section(&mut self, section: Section, window: &mut Window, cx: &mut Context<Self>) {
        let paths: Vec<String> = self
            .rows
            .iter()
            .filter_map(|row| match row {
                Row::File { section: s, entry } if *s == section => Some(entry.path.clone()),
                _ => None,
            })
            .collect();
        self.run(
            move |repo| {
                let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
                if section.is_staged() {
                    repo.unstage(&paths)
                } else {
                    repo.stage(&paths)
                }
            },
            window,
            cx,
        );
    }

    /// Asks, then throws away the selected file's unstaged changes. Untracked
    /// files go to the Trash so a mistake is recoverable.
    fn discard_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.selected_key() else {
            return;
        };
        let untracked = match key.section {
            Section::Changes => false,
            Section::Untracked => true,
            Section::Staged | Section::Conflicts => {
                window.push_notification("Unstage the file before discarding it.", cx);
                return;
            }
        };
        let name = key.path.rsplit('/').next().unwrap_or(&key.path).to_owned();
        let (title, description, ok) = if untracked {
            (
                format!("Move {name} to the Trash?"),
                format!(
                    "{} is untracked. You can restore it from the Trash.",
                    key.path
                ),
                "Move to Trash",
            )
        } else {
            (
                format!("Discard changes to {name}?"),
                format!(
                    "Unstaged changes to {} will be lost. Staged changes are kept.",
                    key.path
                ),
                "Discard",
            )
        };
        let this = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let this = this.clone();
            let key = key.clone();
            alert
                .title(title.clone())
                .description(description.clone())
                .confirm()
                .ok_text(ok)
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    let key = key.clone();
                    this.update(cx, |this, cx| {
                        this.run(
                            move |repo| {
                                if untracked {
                                    move_to_trash(&repo.workdir().join(&key.path))
                                } else {
                                    repo.discard(&[&key.path])
                                }
                            },
                            window,
                            cx,
                        )
                    })
                    .ok();
                    true
                })
        });
    }

    fn focus_diff(&self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.diff.read(cx).files().is_empty() {
            self.diff.read(cx).focus_handle(cx).focus(window, cx);
        }
    }

    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let branch = &self.status.branch;
        let head = match (&branch.head, &branch.oid) {
            (Some(head), _) => head.clone(),
            (None, Some(oid)) => format!("detached at {}", &oid[..oid.len().min(7)]),
            (None, None) => "no commits".into(),
        };
        let muted = cx.theme().muted_foreground;
        TitleBar::new().child(
            h_flex()
                .size_full()
                .gap_3()
                .text_sm()
                .child(div().font_medium().child(self.repo.name()))
                .child(
                    h_flex()
                        .gap_1()
                        .text_color(muted)
                        .child(Icon::empty().path("icons/git-branch.svg").xsmall())
                        .child(head),
                )
                .when(branch.ahead > 0, |this| {
                    this.child(div().text_color(muted).child(format!("↑{}", branch.ahead)))
                })
                .when(branch.behind > 0, |this| {
                    this.child(div().text_color(muted).child(format!("↓{}", branch.behind)))
                }),
        )
    }

    fn render_changes(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows.clone();
        let selected = self.selected;
        let focused = self.changes_focus.is_focused(window);
        let owner = cx.entity().downgrade();
        v_flex()
            .size_full()
            .min_h_0()
            .bg(cx.theme().sidebar)
            .child(
                h_flex()
                    .h_8()
                    .flex_none()
                    .px_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .text_xs()
                    .font_medium()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{} CHANGED FILES", self.status.entries.len())),
            )
            .child(
                div()
                    .id("changes")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .track_focus(&self.changes_focus)
                    .key_context(CHANGES_CONTEXT)
                    .on_mouse_down(
                        gpui_kit::MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.changes_focus.focus(window, cx)),
                    )
                    .when(self.loaded && rows.is_empty(), |this| {
                        this.child(
                            div()
                                .p_4()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("Working tree clean"),
                        )
                    })
                    .child(
                        uniform_list("change-rows", rows.len(), move |range, _, cx| {
                            range
                                .map(|ix| render_row(ix, &rows[ix], selected, focused, &owner, cx))
                                .collect()
                        })
                        .size_full()
                        .track_scroll(&self.changes_scroll),
                    )
                    .child(Scrollbar::vertical(&self.changes_scroll)),
            )
            .child(self.render_commit_box(cx))
    }

    fn render_commit_box(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_message = !self.commit_message.read(cx).value().trim().is_empty();
        let can_commit =
            !self.busy && has_message && (self.amend || self.status.staged().next().is_some());
        v_flex()
            .flex_none()
            .p_2()
            .gap_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(Textarea::new(&self.commit_message).small())
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Checkbox::new("amend")
                            .small()
                            .label("Amend")
                            .checked(self.amend)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.toggle_amend(window, cx)),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("commit")
                            .primary()
                            .small()
                            .label(if self.amend { "Amend" } else { "Commit" })
                            .loading(self.busy)
                            .disabled(!can_commit)
                            .tooltip_with_action(
                                "Commit staged changes",
                                &Commit,
                                Some("Workspace"),
                            )
                            .on_click(cx.listener(|this, _, window, cx| this.commit(window, cx))),
                    ),
            )
    }

    fn render_diff(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let empty = self.diff.read(cx).files().is_empty();
        div().size_full().min_w_0().min_h_0().map(|this| {
            if empty {
                this.child(
                    v_flex()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(if !self.loaded {
                            "Reading status…"
                        } else if self.rows.is_empty() {
                            "Nothing to review"
                        } else if self.selected.is_none() {
                            "Select a file"
                        } else {
                            "No textual changes"
                        }),
                )
            } else {
                this.child(Diff::new(&self.diff).line_number(true))
            }
        })
    }
}

/// Uses NSFileManager rather than the crate's default of scripting Finder,
/// which needs Automation permission.
fn move_to_trash(path: &std::path::Path) -> anyhow::Result<()> {
    use trash::macos::{DeleteMethod, TrashContextExtMacos as _};
    let mut context = trash::TrashContext::default();
    context.set_delete_method(DeleteMethod::NsFileManager);
    context.delete(path)?;
    Ok(())
}

fn change_color(change: Change, cx: &App) -> Hsla {
    match change {
        Change::Added | Change::Copied => cx.theme().green,
        Change::Deleted => cx.theme().red,
        Change::Renamed => cx.theme().blue,
        _ => cx.theme().yellow,
    }
}

fn render_row(
    ix: usize,
    row: &Row,
    selected: Option<usize>,
    focused: bool,
    owner: &gpui_kit::WeakEntity<Workspace>,
    cx: &App,
) -> gpui_kit::AnyElement {
    match row {
        Row::Header { section, count } => {
            let section = *section;
            let owner = owner.clone();
            h_flex()
                .id(("header", ix))
                .w_full()
                .h_7()
                .px_3()
                .gap_2()
                .text_xs()
                .font_medium()
                .text_color(cx.theme().muted_foreground)
                .child(
                    div()
                        .flex_1()
                        .child(format!("{} · {count}", section.title())),
                )
                .child(
                    Button::new(("toggle-section", ix))
                        .ghost()
                        .xsmall()
                        .label(if section.is_staged() {
                            "Unstage all"
                        } else {
                            "Stage all"
                        })
                        .on_click(move |_, window, cx| {
                            owner
                                .update(cx, |this, cx| this.toggle_section(section, window, cx))
                                .ok();
                        }),
                )
                .into_any_element()
        }
        Row::File { section, entry } => {
            let change = changes::change_for(*section, entry);
            let (directory, filename) = entry
                .path
                .rsplit_once('/')
                .unwrap_or(("", entry.path.as_str()));
            let is_selected = selected == Some(ix);
            let select_owner = owner.clone();
            let toggle_owner = owner.clone();
            ListItem::new(("file", ix))
                .w_full()
                .h_7()
                .px_3()
                .selected(is_selected)
                .secondary_selected(is_selected && focused)
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .gap_2()
                        .text_sm()
                        .child(
                            Checkbox::new(("stage", ix))
                                .small()
                                .checked(section.is_staged())
                                .on_click(move |_, window, cx| {
                                    toggle_owner
                                        .update(cx, |this, cx| {
                                            this.toggle_staged_at(ix, window, cx)
                                        })
                                        .ok();
                                }),
                        )
                        .child(div().flex_none().child(filename.to_owned()))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(directory.to_owned()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .w_3()
                                .text_xs()
                                .font_medium()
                                .text_color(change_color(change, cx))
                                .child(match section {
                                    Section::Untracked => "U",
                                    Section::Conflicts => "!",
                                    _ => change.letter(),
                                }),
                        ),
                )
                .on_click(move |_, _, cx| {
                    select_owner
                        .update(cx, |this, cx| this.select(Some(ix), cx))
                        .ok();
                })
                .into_any_element()
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rem = window.rem_size();
        let muted = cx.theme().muted_foreground;
        v_flex()
            .id("workspace")
            .key_context("Workspace")
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.refresh(cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, window, cx| {
                this.sidebar_visible = !this.sidebar_visible;
                if !this.sidebar_visible {
                    this.focus_diff(window, cx);
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleDiffMode, _, cx| {
                this.diff_mode = match this.diff_mode {
                    DiffMode::Unified => DiffMode::Split,
                    _ => DiffMode::Unified,
                };
                let mode = this.diff_mode;
                this.diff.update(cx, |state, cx| state.set_mode(mode, cx));
            }))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| {
                let ix = changes::step(&this.rows, this.selected, 1);
                this.select(ix, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| {
                let ix = changes::step(&this.rows, this.selected, -1);
                this.select(ix, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleStaged, window, cx| {
                if let Some(ix) = this.selected {
                    this.toggle_staged_at(ix, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &StageAll, window, cx| {
                this.run(|repo| repo.stage_all(), window, cx);
            }))
            .on_action(cx.listener(|this, _: &UnstageAll, window, cx| {
                this.run(|repo| repo.unstage_all(), window, cx);
            }))
            .on_action(cx.listener(|this, _: &Commit, window, cx| this.commit(window, cx)))
            .on_action(
                cx.listener(|this, _: &Discard, window, cx| this.discard_selected(window, cx)),
            )
            .on_action(cx.listener(|this, _: &FocusCommitMessage, window, cx| {
                this.sidebar_visible = true;
                this.commit_message
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FocusDiff, window, cx| this.focus_diff(window, cx)))
            .on_action(cx.listener(|this, _: &FocusChanges, window, cx| {
                this.sidebar_visible = true;
                this.changes_focus.focus(window, cx);
                cx.notify();
            }))
            .child(self.render_title_bar(cx))
            .child(
                div().flex_1().min_h_0().child(
                    h_resizable("workspace-panes")
                        .when(self.sidebar_visible, |this| {
                            this.child(
                                resizable_panel()
                                    .size(rem * 20.)
                                    .size_range(rem * 14. ..rem * 40.)
                                    .child(self.render_changes(window, cx)),
                            )
                        })
                        .child(resizable_panel().child(self.render_diff(cx))),
                ),
            )
            .child(
                StatusBar::new()
                    .left(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(self.repo.workdir().display().to_string()),
                    )
                    .when_some(self.error.clone(), |this, error| {
                        this.right(
                            div()
                                .text_xs()
                                .text_color(cx.theme().red)
                                .truncate()
                                .child(error),
                        )
                    }),
            )
    }
}
