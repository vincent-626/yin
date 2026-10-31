//! The History tab: commits on the current branch, and the selected commit's
//! message and diff.

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

/// Body lines shown before "Show more".
const COLLAPSED_BODY_LINES: usize = 3;

impl Workspace {
    /// Reloads the commit list, keeping the selected commit when it remains.
    pub(super) fn load_history(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        let path = self.history_path.clone();
        self.history_head = self.status.branch.oid.clone();
        self._history_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    match path {
                        Some(path) => repo.file_log(&path, HISTORY_LIMIT),
                        None => repo.log(HISTORY_LIMIT),
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(commits) => {
                        let previous = this
                            .pending_commit
                            .take()
                            .or_else(|| this.selected_commit().map(|commit| commit.oid.clone()));
                        let ix = previous
                            .and_then(|oid| commits.iter().position(|c| c.oid == oid))
                            .or((!commits.is_empty()).then_some(0));
                        this.commits = Arc::new(commits);
                        this.history_loaded = true;
                        this.commit_selected = None;
                        this.select_commit(ix, cx);
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
        if self.history_path != path {
            self.history_path = path;
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

    pub(super) fn select_commit(&mut self, ix: Option<usize>, cx: &mut Context<Self>) {
        if ix == self.commit_selected {
            return;
        }
        self.commit_selected = ix;
        let Some(commit) = ix.and_then(|ix| self.commits.get(ix)) else {
            self.commit_detail = None;
            self.history_diff
                .update(cx, |state, cx| state.set_files([], cx));
            return;
        };
        if let Some(ix) = ix {
            self.history_scroll
                .scroll_to_item(ix, ScrollStrategy::Nearest);
        }
        let oid = commit.oid.clone();
        let repo = self.repo.clone();
        let path = self.history_path.clone();
        self._commit_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn({
                    let oid = oid.clone();
                    async move {
                        let message = repo.commit_message(&oid)?;
                        let patch = match &path {
                            Some(path) => repo.commit_file_patch(&oid, path)?,
                            None => repo.commit_patch(&oid)?,
                        };
                        let files = DiffFile::parse(&patch)
                            .map_err(|error| anyhow::anyhow!("couldn't read diff: {error}"))?;
                        anyhow::Ok((message, files))
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                if this.selected_commit().map(|c| &c.oid) != Some(&oid) {
                    return;
                }
                match result {
                    Ok((message, files)) => {
                        let selection = TextSelectionHandle::new(message.clone(), cx);
                        this.commit_detail = Some(CommitDetail {
                            oid,
                            message,
                            selection,
                        });
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

    pub(super) fn step_commit(&mut self, direction: isize, cx: &mut Context<Self>) {
        if self.commits.is_empty() {
            return;
        }
        let ix = match self.commit_selected {
            Some(ix) => ix
                .saturating_add_signed(direction)
                .min(self.commits.len() - 1),
            None => 0,
        };
        self.select_commit(Some(ix), cx);
    }

    pub(super) fn render_history(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let commits = self.commits.clone();
        let selected = self.commit_selected;
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
                            render_commit_row(ix, &commits[ix], selected, focused, &owner, cx)
                        })
                        .collect()
                })
                .size_full()
                .track_scroll(&self.history_scroll),
            )
            .child(Scrollbar::vertical(&self.history_scroll));
        v_flex()
            .flex_1()
            .min_h_0()
            .children(filter)
            .child(list)
            .into_any_element()
    }

    pub(super) fn render_commit_detail(&self, cx: &mut Context<Self>) -> AnyElement {
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
            .child(
                div().flex_1().min_h_0().child(
                    Diff::new(&self.history_diff)
                        .line_number(true)
                        .render_header({
                            let state = self.history_diff.clone();
                            move |file, _, cx| super::style::diff_file_header(file, &state, cx)
                        }),
                ),
            )
            .into_any_element()
    }
}

fn render_commit_row(
    ix: usize,
    commit: &Commit,
    selected: Option<usize>,
    focused: bool,
    owner: &WeakEntity<Workspace>,
    cx: &App,
) -> AnyElement {
    let is_selected = selected == Some(ix);
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
        .on_click(move |_, _, cx| {
            owner
                .update(cx, |this, cx| this.select_commit(Some(ix), cx))
                .ok();
        })
        .into_any_element()
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
