use std::sync::Arc;

use gpui_kit::component::{
    ActiveTheme as _, Icon, Selectable as _, Sizable as _, StyledExt as _, TitleBar,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    diff::{Diff, DiffFile, DiffMode, DiffState},
    h_flex,
    list::ListItem,
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

mod actions {
    gpui_kit::actions!(
        yin,
        [
            Quit,
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
        ]
    );
}
use actions::*;

const CHANGES_CONTEXT: &str = "Changes";

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
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
    sidebar_visible: bool,
    changes_focus: FocusHandle,
    changes_scroll: UniformListScrollHandle,
    status_generation: usize,
    diff_generation: usize,
    _status_task: Task<()>,
    _diff_task: Task<()>,
    _operation_task: Task<()>,
}

impl Workspace {
    pub fn new(repo: Repository, window: &mut Window, cx: &mut Context<Self>) -> Self {
        window.set_window_title(&repo.name());
        let diff = cx.new(|cx| DiffState::new([], cx));
        cx.observe(&diff, |_, _, cx| cx.notify()).detach();
        let changes_focus = cx.focus_handle().tab_stop(true);
        changes_focus.focus(window, cx);
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
            sidebar_visible: true,
            changes_focus,
            changes_scroll: UniformListScrollHandle::new(),
            status_generation: 0,
            diff_generation: 0,
            _status_task: Task::ready(()),
            _diff_task: Task::ready(()),
            _operation_task: Task::ready(()),
        };
        this.refresh(cx);
        this
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
        cx.notify();
    }

    /// Runs a git write off the main thread, then refreshes.
    fn run(
        &mut self,
        operation: impl FnOnce(&Repository) -> anyhow::Result<()> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let repo = self.repo.clone();
        self._operation_task = cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { operation(&repo) }).await;
            this.update(cx, |this, cx| {
                this.error = result.err().map(|error| error.to_string().into());
                this.refresh(cx);
                cx.notify();
            })
            .ok();
        });
    }

    fn toggle_staged_at(&mut self, ix: usize, cx: &mut Context<Self>) {
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
            cx,
        );
    }

    fn toggle_section(&mut self, section: Section, cx: &mut Context<Self>) {
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
            cx,
        );
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
                        .on_click(move |_, _, cx| {
                            owner
                                .update(cx, |this, cx| this.toggle_section(section, cx))
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
                                .on_click(move |_, _, cx| {
                                    toggle_owner
                                        .update(cx, |this, cx| this.toggle_staged_at(ix, cx))
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
            .on_action(|_: &Quit, _, cx| cx.quit())
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
            .on_action(cx.listener(|this, _: &ToggleStaged, _, cx| {
                if let Some(ix) = this.selected {
                    this.toggle_staged_at(ix, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &StageAll, _, cx| {
                this.run(|repo| repo.stage_all(), cx);
            }))
            .on_action(cx.listener(|this, _: &UnstageAll, _, cx| {
                this.run(|repo| repo.unstage_all(), cx);
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
