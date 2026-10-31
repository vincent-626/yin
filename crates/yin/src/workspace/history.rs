//! The History tab: commits on the current branch, and the selected commit's
//! message and diff. Shift-selecting a run of commits shows what they changed
//! together; comparing with a branch lists only this branch's commits and
//! shows its whole change, the way a pull request would.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::ClipboardItem;
use gpui_kit::base::{SelectableText, TextSelectionHandle};
use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    diff::{Diff, DiffFile},
    h_flex,
    list::ListItem,
    scroll::Scrollbar,
    tag::Tag,
    v_flex,
};
use gpui_kit::component::{Icon, IconName, WindowExt as _, tooltip::Tooltip};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, ScrollStrategy, SharedString, StatefulInteractiveElement as _, Styled as _,
    WeakEntity, Window, div, prelude::FluentBuilder as _, uniform_list,
};
use yin_git::Commit;

use super::{HISTORY_CONTEXT, Workspace};

/// How many commits the History tab reads.
const HISTORY_LIMIT: usize = 2000;

/// The selected commit, once its message and patch have loaded.
pub(super) struct CommitDetail {
    oid: String,
    message: String,
    /// Makes the subject and body one selectable, copyable document.
    selection: TextSelectionHandle,
}

/// Branches being compared in the History tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Compare {
    /// What the change is measured from, such as `main`.
    pub base: String,
    /// A branch, or `HEAD`.
    pub head: String,
}

/// What the History tab's diff shows.
enum Target {
    Commit(String),
    Range {
        newest: String,
        oldest: String,
        /// Set when the run is a whole comparison: measured from the merge
        /// base, so merges from the base don't show up as changes.
        compare: Option<Compare>,
    },
}

impl Target {
    fn key(&self) -> (String, String) {
        match self {
            Target::Commit(oid) => (oid.clone(), oid.clone()),
            Target::Range { newest, oldest, .. } => (newest.clone(), oldest.clone()),
        }
    }
}

/// Body lines shown before "Show more".
const COLLAPSED_BODY_LINES: usize = 3;

impl Workspace {
    /// Reloads the commit list, keeping the selected commit when it remains.
    pub(super) fn load_history(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        let path = self.history_path.clone();
        let compare = self.history_compare.clone();
        self.history_head = self.status.branch.oid.clone();
        self._history_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let commits = match (&compare, &path) {
                        (Some(compare), _) => {
                            repo.log_range(&compare.base, &compare.head, HISTORY_LIMIT)
                        }
                        (None, Some(path)) => repo.file_log(path, HISTORY_LIMIT),
                        (None, None) => repo.log(HISTORY_LIMIT),
                    }?;
                    // A stack is worth showing only on the branch's own history.
                    let stack = match path {
                        Some(_) => None,
                        None => repo.stack().unwrap_or(None),
                    };
                    anyhow::Ok((commits, stack))
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok((commits, stack)) => {
                        this.stack = stack;
                        let previous = this
                            .pending_commit
                            .take()
                            .or_else(|| this.selected_commit().map(|commit| commit.oid.clone()));
                        let ix = previous
                            .and_then(|oid| commits.iter().position(|c| c.oid == oid))
                            .or((!commits.is_empty()).then_some(0));
                        let len = commits.len();
                        this.commits = Arc::new(commits);
                        this.history_loaded = true;
                        this.commit_selected = None;
                        this.commit_anchor = None;
                        if this.history_compare.is_some() && len > 1 {
                            // A comparison opens on the whole change.
                            this.extend_commit_to(Some(0), Some(len - 1), cx);
                        } else {
                            this.select_commit(ix, cx);
                        }
                    }
                    Err(error) => this.error = Some(error.to_string().into()),
                }
                cx.notify();
            })
            .ok();
        });
    }

    /// Shows only commits that changed `path`, or every commit with `None`.
    pub(super) fn set_history_path(
        &mut self,
        path: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.history_path != path || self.history_compare.is_some() {
            self.history_path = path;
            self.history_compare = None;
            self.history_loaded = false;
            self.commits = Arc::default();
            self.commit_selected = None;
        }
        self.show_panel(super::Panel::History, window, cx);
    }

    /// Opens a commit in the History tab, such as one picked from blame.
    pub(super) fn show_commit(&mut self, oid: String, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_commit = Some(oid);
        self.history_path = None;
        self.history_compare = None;
        self.history_loaded = false;
        self.commit_selected = None;
        self.show_panel(super::Panel::History, window, cx);
    }

    pub(super) fn copy_commit_hash(&self, oid: &str, window: &mut Window, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(oid.to_owned()));
        window.push_notification(format!("Copied {}", &oid[..oid.len().min(7)]), cx);
    }

    /// Copies the selected commit's full hash.
    pub(super) fn copy_selected_commit_hash(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(oid) = self.selected_commit().map(|commit| commit.oid.clone()) {
            self.copy_commit_hash(&oid, window, cx);
        }
    }

    fn selected_commit(&self) -> Option<&Commit> {
        self.commits.get(self.commit_selected?)
    }

    /// The selected run of commits as (newest, oldest) indices, when more
    /// than one is selected.
    pub(super) fn selected_range(&self) -> Option<(usize, usize)> {
        let (cursor, anchor) = (self.commit_selected?, self.commit_anchor?);
        (cursor != anchor).then(|| (cursor.min(anchor), cursor.max(anchor)))
    }

    /// Selects one commit.
    pub(super) fn select_commit(&mut self, ix: Option<usize>, cx: &mut Context<Self>) {
        if ix == self.commit_selected && self.commit_anchor.is_none() {
            return;
        }
        self.commit_anchor = None;
        self.commit_selected = ix;
        self.load_commit_selection(cx);
    }

    /// Selects from `anchor` (or the current selection) to `ix`.
    pub(super) fn extend_commit_to(
        &mut self,
        ix: Option<usize>,
        anchor: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        let anchor = anchor.or(self.commit_anchor).or(self.commit_selected);
        if ix == self.commit_selected && anchor == self.commit_anchor {
            return;
        }
        self.commit_anchor = anchor;
        self.commit_selected = ix;
        self.load_commit_selection(cx);
    }

    fn load_commit_selection(&mut self, cx: &mut Context<Self>) {
        let Some(ix) = self.commit_selected else {
            self.commit_detail = None;
            self.history_patch.clear();
            self.history_diff
                .update(cx, |state, cx| state.set_files([], cx));
            cx.notify();
            return;
        };
        self.history_scroll
            .scroll_to_item(ix, ScrollStrategy::Nearest);
        let repo = self.repo.clone();
        let path = self.history_path.clone();
        let target = match self.selected_range() {
            Some((newest, oldest)) => {
                let whole = newest == 0 && oldest + 1 == self.commits.len();
                let compare = self.history_compare.clone().filter(|_| whole);
                Target::Range {
                    newest: self.commits[newest].oid.clone(),
                    oldest: self.commits[oldest].oid.clone(),
                    compare,
                }
            }
            None => Target::Commit(self.commits[ix].oid.clone()),
        };
        let key = target.key();
        self._commit_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let path = path.as_deref();
                    let (message, patch) = match target {
                        Target::Commit(oid) => {
                            let patch = match path {
                                Some(path) => repo.commit_file_patch(&oid, path)?,
                                None => repo.commit_patch(&oid)?,
                            };
                            (Some(repo.commit_message(&oid)?), patch)
                        }
                        Target::Range {
                            compare: Some(compare),
                            ..
                        } => (
                            None,
                            repo.compare_patch(&compare.base, &compare.head, path)?,
                        ),
                        Target::Range { newest, oldest, .. } => {
                            (None, repo.commits_patch(&oldest, &newest, path)?)
                        }
                    };
                    let files = DiffFile::parse(&patch)
                        .map_err(|error| anyhow::anyhow!("couldn't read diff: {error}"))?;
                    anyhow::Ok((message, patch, files))
                })
                .await;
            this.update(cx, |this, cx| {
                if this.selection_key() != key {
                    return;
                }
                match result {
                    Ok((message, patch, files)) => {
                        this.commit_detail = message.map(|message| CommitDetail {
                            oid: key.0.clone(),
                            selection: TextSelectionHandle::new(message.clone(), cx),
                            message,
                        });
                        this.history_patch = patch;
                        this.history_diff
                            .update(cx, |state, cx| state.set_files(files, cx));
                        this.schedule_relayout(cx);
                    }
                    Err(error) => this.error = Some(error.to_string().into()),
                }
                cx.notify();
            })
            .ok();
        });
        cx.notify();
    }

    /// The newest and oldest selected commits, to tell whether a load is stale.
    fn selection_key(&self) -> (String, String) {
        let oid = |ix: usize| {
            self.commits
                .get(ix)
                .map(|c| c.oid.clone())
                .unwrap_or_default()
        };
        match (self.selected_range(), self.commit_selected) {
            (Some((newest, oldest)), _) => (oid(newest), oid(oldest)),
            (None, Some(ix)) => (oid(ix), oid(ix)),
            (None, None) => Default::default(),
        }
    }

    /// Moves the selection, or with `extend` grows or shrinks the run.
    pub(super) fn step_commit(&mut self, direction: isize, extend: bool, cx: &mut Context<Self>) {
        if self.commits.is_empty() {
            return;
        }
        let ix = match self.commit_selected {
            Some(ix) => ix
                .saturating_add_signed(direction)
                .min(self.commits.len() - 1),
            None => 0,
        };
        if extend {
            self.extend_commit_to(Some(ix), None, cx);
        } else {
            self.select_commit(Some(ix), cx);
        }
    }

    /// Shows only `head`'s commits since `base`, and their whole change.
    pub(super) fn compare(
        &mut self,
        base: String,
        head: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.history_compare = Some(Compare { base, head });
        self.history_path = None;
        self.history_loaded = false;
        self.commits = Arc::default();
        self.commit_selected = None;
        self.commit_anchor = None;
        self.show_panel(super::Panel::History, window, cx);
    }

    pub(super) fn stop_comparing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.history_compare.is_some() {
            self.set_history_path(None, window, cx);
        }
    }

    pub(super) fn render_history(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let commits = self.commits.clone();
        let selected = match self.selected_range() {
            Some((newest, oldest)) => Some(newest..=oldest),
            None => self.commit_selected.map(|ix| ix..=ix),
        };
        let focused = self.history_focus.is_focused(window);
        let owner = cx.entity().downgrade();
        let filter = self.history_path.clone().map(|path| {
            h_flex()
                .flex_none()
                .px_3()
                .py_1()
                .gap_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .border_b_1()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(format!("History of {path}")),
                )
                .child(
                    Button::new("all-commits")
                        .ghost()
                        .xsmall()
                        .label("All commits")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.set_history_path(None, window, cx)
                        })),
                )
        });
        let list = div()
            .id("history")
            .relative()
            .flex_1()
            .min_h_0()
            .track_focus(&self.history_focus)
            .key_context(HISTORY_CONTEXT)
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|this, _, window, cx| this.history_focus.focus(window, cx)),
            )
            .when(self.history_loaded && commits.is_empty(), |this| {
                this.child(
                    div()
                        .p_4()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("No commits yet"),
                )
            })
            .child(
                uniform_list("commit-rows", commits.len(), move |range, _, cx| {
                    range
                        .map(|ix| {
                            render_commit_row(ix, &commits[ix], &selected, focused, &owner, cx)
                        })
                        .collect()
                })
                .size_full()
                .track_scroll(&self.history_scroll),
            )
            .child(Scrollbar::vertical(&self.history_scroll));
        let compare = self.history_compare.as_ref().map(|compare| {
            h_flex()
                .flex_none()
                .px_3()
                .py_1()
                .gap_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .border_b_1()
                .border_color(cx.theme().border)
                .child(div().flex_1().min_w_0().truncate().child(format!(
                    "{} since {} · {}",
                    self.ref_label(&compare.head),
                    compare.base,
                    count(self.commits.len(), "commit")
                )))
                .child(
                    Button::new("stop-comparing")
                        .ghost()
                        .xsmall()
                        .label("All commits")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.stop_comparing(window, cx)),
                        ),
                )
        });
        v_flex()
            .flex_1()
            .min_h_0()
            .children(self.render_stack(cx))
            .children(filter)
            .children(compare)
            .child(list)
            .children(self.render_notes_strip(cx))
            .into_any_element()
    }

    /// `HEAD` as the branch name it stands for.
    pub(super) fn ref_label(&self, name: &str) -> String {
        if name == "HEAD" {
            self.head_label()
        } else {
            name.to_owned()
        }
    }

    /// The branches stacked under and over this one; each opens that
    /// layer's own change.
    fn render_stack(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let stack = self
            .stack
            .as_ref()
            .filter(|_| self.history_path.is_none())?;
        let comparing = self.history_compare.as_ref();
        let muted = cx.theme().muted_foreground;
        let separator = || div().flex_none().text_color(muted).child("›");
        let trunk = stack.trunk.clone();
        let mut row = h_flex()
            .id("stack")
            .flex_none()
            .px_2()
            .py_1()
            .gap_0p5()
            .overflow_x_scroll()
            .text_xs()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("stack-trunk")
                    .ghost()
                    .xsmall()
                    .label(trunk)
                    .text_color(muted)
                    .tooltip("Show all commits")
                    .on_click(cx.listener(|this, _, window, cx| this.stop_comparing(window, cx))),
            );
        for (ix, layer) in stack.layers.iter().enumerate() {
            let head = if layer.is_head { "HEAD" } else { &layer.branch };
            let active = comparing.is_some_and(|c| c.head == head && c.base == layer.base);
            let (base, head) = (layer.base.clone(), head.to_owned());
            row = row.child(separator()).child(
                Button::new(("stack-layer", ix))
                    .ghost()
                    .xsmall()
                    .selected(active)
                    .label(format!("{} {}", layer.branch, layer.commits))
                    .when(layer.is_head, |this| this.font_semibold())
                    .when(!layer.is_head, |this| this.text_color(muted))
                    .tooltip(format!(
                        "Review {}: {} on {}",
                        layer.branch,
                        count(layer.commits, "commit"),
                        layer.base
                    ))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.compare(base.clone(), head.clone(), window, cx)
                    })),
            );
        }
        Some(row.into_any_element())
    }

    pub(super) fn render_commit_detail(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(range) = self.selected_range() {
            return self.render_range_detail(range, window, cx);
        }
        let Some(commit) = self.selected_commit() else {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(if self.history_loaded {
                    "No commits yet"
                } else {
                    "Reading history…"
                })
                .into_any_element();
        };
        let detail = self
            .commit_detail
            .as_ref()
            .filter(|detail| detail.oid == commit.oid);
        let (subject, body) = match detail {
            Some(detail) => match detail.message.split_once("\n\n") {
                Some((subject, body)) => (subject.to_owned(), body.trim().to_owned()),
                None => (detail.message.clone(), String::new()),
            },
            None => (commit.subject.clone(), String::new()),
        };
        let body_lines = body.lines().count();
        let long = body_lines > COLLAPSED_BODY_LINES;
        let shown_body = if long && !self.commit_body_expanded {
            let mut shown: Vec<&str> = body.lines().take(COLLAPSED_BODY_LINES).collect();
            if let Some(last) = shown.last_mut() {
                *last = last.trim_end();
            }
            format!("{}…", shown.join("\n"))
        } else {
            body.clone()
        };
        let selectable = |id: &'static str, order: u64, text: String| match detail {
            Some(detail) => SelectableText::with_handle(id, detail.selection.clone(), text)
                .document_order(order)
                .into_any_element(),
            None => SelectableText::new(id, text).into_any_element(),
        };
        let oid = commit.oid.clone();
        let short_oid = commit.short_oid.clone();
        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(
                v_flex()
                    .flex_none()
                    .px_4()
                    .py_3()
                    .gap_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().text_lg().font_semibold().child(selectable(
                        "commit-subject",
                        0,
                        subject,
                    )))
                    .when(!body.is_empty(), |this| {
                        this.child(
                            div()
                                .id("commit-body")
                                .when(self.commit_body_expanded, |this| {
                                    this.max_h_80().overflow_y_scroll()
                                })
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(selectable("commit-body-text", 1, shown_body)),
                        )
                    })
                    .child(
                        h_flex()
                            .pt_1()
                            .gap_2()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                h_flex()
                                    .id("copy-hash")
                                    .gap_1()
                                    .cursor_pointer()
                                    .hover(|this| this.text_color(cx.theme().foreground))
                                    .child(super::style::hash(&short_oid, cx))
                                    .child(Icon::new(IconName::Copy).xsmall())
                                    .tooltip(|window, cx| {
                                        Tooltip::new("Copy commit hash").build(window, cx)
                                    })
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.copy_commit_hash(&oid, window, cx)
                                    })),
                            )
                            .child(
                                div()
                                    .text_color(cx.theme().foreground)
                                    .child(commit.author.clone()),
                            )
                            .child(relative_time(commit.time))
                            .when(long, |this| {
                                let expanded = self.commit_body_expanded;
                                this.child(div().flex_1()).child(
                                    Button::new("toggle-body")
                                        .ghost()
                                        .xsmall()
                                        .label(if expanded { "Show less" } else { "Show more" })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.commit_body_expanded = !this.commit_body_expanded;
                                            cx.notify();
                                        })),
                                )
                            }),
                    ),
            )
            .child(self.render_history_diff(window, cx))
            .into_any_element()
    }

    fn render_history_diff(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let selection_bar = self
            .history_diff
            .read(cx)
            .selected_lines()
            .map(|_| self.render_note_hint(window, cx));
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .child(
                self.annotate(
                    Diff::new(&self.history_diff)
                        .line_number(true)
                        .render_header({
                            let state = self.history_diff.clone();
                            move |file, _, cx| {
                                super::style::diff_file_header(file, &state, None, cx)
                            }
                        }),
                    super::annotations::DiffPane::History,
                    cx,
                ),
            )
            .children(selection_bar)
            .children(self.render_note_draft(super::annotations::DiffPane::History, window, cx))
            .into_any_element()
    }

    /// A run of commits: what they are, then their combined diff.
    fn render_range_detail(
        &self,
        (newest, oldest): (usize, usize),
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let commits = &self.commits[newest..=oldest];
        let whole = newest == 0 && oldest + 1 == self.commits.len();
        let title = match self.history_compare.as_ref().filter(|_| whole) {
            Some(compare) => format!("{} since {}", self.ref_label(&compare.head), compare.base),
            None => count(commits.len(), "commit"),
        };
        const LISTED: usize = 8;
        let muted = cx.theme().muted_foreground;
        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(
                v_flex()
                    .flex_none()
                    .px_4()
                    .py_3()
                    .gap_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().text_lg().font_semibold().child(title))
                    .children(commits.iter().take(LISTED).map(|commit| {
                        h_flex()
                            .gap_2()
                            .text_sm()
                            .min_w_0()
                            .child(super::style::hash(&commit.short_oid, cx))
                            .child(div().min_w_0().truncate().child(commit.subject.clone()))
                    }))
                    .when(commits.len() > LISTED, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child(format!("and {} more", commits.len() - LISTED)),
                        )
                    })
                    .child(
                        div()
                            .pt_1()
                            .text_xs()
                            .text_color(muted)
                            .child("Shift-click or shift-↑↓ to change the range"),
                    ),
            )
            .child(self.render_history_diff(window, cx))
            .into_any_element()
    }
}

fn render_commit_row(
    ix: usize,
    commit: &Commit,
    selected: &Option<std::ops::RangeInclusive<usize>>,
    focused: bool,
    owner: &WeakEntity<Workspace>,
    cx: &App,
) -> AnyElement {
    let is_selected = selected.as_ref().is_some_and(|range| range.contains(&ix));
    let owner = owner.clone();
    let refs: Vec<SharedString> = commit
        .refs
        .iter()
        .map(|r| r.strip_prefix("HEAD -> ").unwrap_or(r))
        .filter(|r| *r != "HEAD")
        .map(|r| SharedString::from(r.to_owned()))
        .collect();
    ListItem::new(("commit", ix))
        .w_full()
        .h_12()
        .px_3()
        .selected(is_selected)
        .secondary_selected(is_selected && focused)
        .child(
            v_flex()
                .w_full()
                .min_w_0()
                .gap_0p5()
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .gap_1()
                        .children(refs.into_iter().map(|r| Tag::secondary().xsmall().child(r)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .child(commit.subject.clone()),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(super::style::hash(&commit.short_oid, cx))
                        .child(
                            div()
                                .truncate()
                                .text_color(cx.theme().foreground.opacity(0.8))
                                .child(commit.author.clone()),
                        )
                        .child(div().flex_none().child(relative_time(commit.time))),
                ),
        )
        .on_click(move |event, _, cx| {
            let extend = event.modifiers().shift;
            owner
                .update(cx, |this, cx| {
                    if extend {
                        this.extend_commit_to(Some(ix), None, cx)
                    } else {
                        this.select_commit(Some(ix), cx)
                    }
                })
                .ok();
        })
        .into_any_element()
}

/// "1 commit", "3 commits".
pub(super) fn count(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

/// "3 days ago"-style ages for commit lists.
pub(super) fn relative_time(time: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    age(now - time)
}

fn age(seconds: i64) -> String {
    let (count, unit) = match seconds.max(0) {
        s if s < 60 => return "just now".into(),
        s if s < 3600 => (s / 60, "minute"),
        s if s < 86_400 => (s / 3600, "hour"),
        s if s < 7 * 86_400 => (s / 86_400, "day"),
        s if s < 30 * 86_400 => (s / (7 * 86_400), "week"),
        s if s < 365 * 86_400 => (s / (30 * 86_400), "month"),
        s => (s / (365 * 86_400), "year"),
    };
    format!("{count} {unit}{} ago", if count == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::age;

    #[test]
    fn ages_read_naturally() {
        assert_eq!(age(5), "just now");
        assert_eq!(age(60), "1 minute ago");
        assert_eq!(age(3 * 3600), "3 hours ago");
        assert_eq!(age(86_400), "1 day ago");
        assert_eq!(age(15 * 86_400), "2 weeks ago");
        assert_eq!(age(400 * 86_400), "1 year ago");
        assert_eq!(age(-10), "just now");
    }
}
