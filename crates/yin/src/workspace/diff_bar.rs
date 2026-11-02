//! The bar above a diff: how much changed, folding every file at once, and
//! finding text in the changes (cmd-f).

use std::collections::HashSet;
use std::sync::Arc;

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    diff::{DiffLinePosition, DiffLineRange, DiffSide},
    h_flex,
    input::{Input, InputEvent, InputState},
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement as _,
    ParentElement as _, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _,
};
use yin_git::FilePatch;

use super::annotations::DiffPane;
use super::{CloseDiffFind, Workspace, style};

pub(super) struct DiffFind {
    pane: DiffPane,
    input: Entity<InputState>,
    matches: Vec<DiffLinePosition>,
    current: usize,
    _input_events: Subscription,
}

/// Lines of `patch` containing `query`, smart case: as the diff numbers
/// them, on the new side, or the old for deleted lines.
pub(super) fn find_in_patch(patch: &str, query: &str) -> Vec<DiffLinePosition> {
    if query.is_empty() {
        return Vec::new();
    }
    let exact = query.chars().any(char::is_uppercase);
    let query = if exact {
        query.to_owned()
    } else {
        query.to_lowercase()
    };
    let mut found = Vec::new();
    for file in FilePatch::split(patch) {
        let Some(path) = file.path().map(str::to_owned) else {
            continue;
        };
        for row in file.rows() {
            let hit = if exact {
                row.text().contains(&query)
            } else {
                row.text().to_lowercase().contains(&query)
            };
            if !hit {
                continue;
            }
            let position = match (row.new_line, row.old_line) {
                (Some(line), _) => DiffLinePosition::new(path.clone(), DiffSide::Modified, line),
                (None, Some(line)) => DiffLinePosition::new(path.clone(), DiffSide::Original, line),
                (None, None) => continue,
            };
            found.push(position);
        }
    }
    found
}

impl Workspace {
    fn patch_in(&self, pane: DiffPane) -> &str {
        match pane {
            DiffPane::Changes => &self.all_patch,
            DiffPane::History => &self.history_patch,
        }
    }

    pub(super) fn open_diff_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = self.active_pane() else {
            return;
        };
        if let Some(find) = self.diff_find.as_ref().filter(|find| find.pane == pane) {
            let input = find.input.clone();
            input.update(cx, |input, cx| input.focus(window, cx));
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in changes"));
        let events =
            cx.subscribe_in(
                &input,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Change => this.search_diff(cx),
                    InputEvent::PressEnter { shift, .. } => {
                        this.step_diff_find(if *shift { -1 } else { 1 }, cx)
                    }
                    _ => {}
                },
            );
        input.update(cx, |input, cx| input.focus(window, cx));
        self.diff_find = Some(DiffFind {
            pane,
            input,
            matches: Vec::new(),
            current: 0,
            _input_events: events,
        });
        cx.notify();
    }

    /// Searches `pane`'s diff again after it changed, if it's being searched.
    pub(super) fn research_diff(&mut self, pane: DiffPane, cx: &mut Context<Self>) {
        if self
            .diff_find
            .as_ref()
            .is_some_and(|find| find.pane == pane)
        {
            self.search_diff(cx);
        }
    }

    pub(super) fn search_diff(&mut self, cx: &mut Context<Self>) {
        let Some(find) = &self.diff_find else {
            return;
        };
        let query = find.input.read(cx).value().to_string();
        let matches = find_in_patch(self.patch_in(find.pane), &query);
        if let Some(find) = &mut self.diff_find {
            find.matches = matches;
            find.current = find.current.min(find.matches.len().saturating_sub(1));
        }
        self.reveal_diff_match(cx);
    }

    fn step_diff_find(&mut self, direction: isize, cx: &mut Context<Self>) {
        let Some(find) = &mut self.diff_find else {
            return;
        };
        let count = find.matches.len();
        if count == 0 {
            return;
        }
        find.current = (find.current as isize + direction).rem_euclid(count as isize) as usize;
        self.reveal_diff_match(cx);
    }

    fn reveal_diff_match(&mut self, cx: &mut Context<Self>) {
        let Some(find) = &self.diff_find else {
            return;
        };
        // 0.8.1's diff can't tint lines, so the current match is selected.
        if let Some(position) = find.matches.get(find.current).cloned() {
            let line = position.line();
            let range = DiffLineRange::new(position.path().clone(), position.side(), line, line);
            self.diff_in(find.pane).update(cx, |diff, cx| {
                diff.scroll_to_line(position, cx);
                diff.set_selected_lines(Some(range), cx);
            });
        }
        cx.notify();
    }

    pub(super) fn close_diff_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.diff_find.take().is_some() {
            self.focus_diff(window, cx);
            cx.notify();
        }
    }

    /// Folds every file to its header, or opens them all when they're folded.
    pub(super) fn toggle_collapse_all(&mut self, pane: DiffPane, cx: &mut Context<Self>) {
        self.diff_in(pane).update(cx, |diff, cx| {
            let paths: Vec<String> = diff
                .files()
                .iter()
                .map(|file| file.path().to_string())
                .collect();
            let collapse = !paths.iter().all(|path| diff.is_file_collapsed(path));
            for path in &paths {
                diff.set_file_collapsed(path, collapse, cx);
            }
        });
    }

    /// Shows all of `path` in the changes diff, or only its changes again.
    pub(super) fn toggle_whole_file(&mut self, path: &str, cx: &mut Context<Self>) {
        let mut whole: HashSet<String> = (*self.whole_files).clone();
        if !whole.remove(path) {
            whole.insert(path.to_owned());
        }
        self.whole_files = Arc::new(whole);
        self.load_all_diff(cx);
    }

    pub(super) fn render_diff_bar(&self, pane: DiffPane, cx: &mut Context<Self>) -> AnyElement {
        let diff = self.diff_in(pane).read(cx);
        let files = diff.files();
        let (additions, deletions) = files.iter().fold((0, 0), |(a, d), file| {
            (a + file.additions(), d + file.deletions())
        });
        let all_collapsed =
            !files.is_empty() && files.iter().all(|file| diff.is_file_collapsed(file.path()));
        let count = files.len();
        let find = self.diff_find.as_ref().filter(|find| find.pane == pane);
        let muted = cx.theme().muted_foreground;
        h_flex()
            .flex_none()
            .h_8()
            .px_3()
            .gap_2()
            .text_xs()
            .text_color(muted)
            .border_b_1()
            .border_color(cx.theme().border)
            .child(super::history::count(count, "file"))
            .child(style::counts(additions, deletions, cx))
            .child(div().flex_1())
            .map(|this| match find {
                Some(find) => {
                    let query_empty = find.input.read(cx).value().is_empty();
                    let summary = match find.matches.len() {
                        _ if query_empty => String::new(),
                        0 => "No results".into(),
                        n => format!("{} of {n}", find.current + 1),
                    };
                    this.child(
                        h_flex()
                            .key_context("DiffFind")
                            .on_action(cx.listener(|this, _: &CloseDiffFind, window, cx| {
                                this.close_diff_find(window, cx)
                            }))
                            .gap_1()
                            .child(div().w_56().child(Input::new(&find.input).xsmall()))
                            .child(div().w_16().child(summary))
                            .child(
                                Button::new("diff-find-previous")
                                    .cursor_pointer()
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::ArrowUp)
                                    .tooltip("Previous (shift-enter)")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.step_diff_find(-1, cx)),
                                    ),
                            )
                            .child(
                                Button::new("diff-find-next")
                                    .cursor_pointer()
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::ArrowDown)
                                    .tooltip("Next (enter)")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.step_diff_find(1, cx)),
                                    ),
                            )
                            .child(
                                Button::new("diff-find-close")
                                    .cursor_pointer()
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Close)
                                    .tooltip("Close (escape)")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.close_diff_find(window, cx)
                                    })),
                            ),
                    )
                }
                None => this.child(
                    Button::new("diff-find")
                        .cursor_pointer()
                        .ghost()
                        .xsmall()
                        .icon(IconName::Search)
                        .tooltip_with_action(
                            "Find in changes",
                            &super::FindInDiff,
                            Some("Workspace"),
                        )
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_diff_find(window, cx)),
                        ),
                ),
            })
            .child(
                Button::new("collapse-all")
                    .cursor_pointer()
                    .ghost()
                    .xsmall()
                    .icon(Icon::empty().path(if all_collapsed {
                        "icons/chevrons-up-down.svg"
                    } else {
                        "icons/chevrons-down-up.svg"
                    }))
                    .tooltip_with_action(
                        if all_collapsed {
                            "Expand all files"
                        } else {
                            "Collapse all files"
                        },
                        &super::ToggleCollapseAll,
                        Some("Workspace"),
                    )
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.toggle_collapse_all(pane, cx)),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_lines_in_a_patch() {
        let patch = "diff --git a/x.rs b/x.rs\n--- a/x.rs\n+++ b/x.rs\n@@ -1,2 +1,2 @@\n Keep\n-old Value\n+new value\n";
        let lines = |query| -> Vec<(DiffSide, usize)> {
            find_in_patch(patch, query)
                .iter()
                .map(|p| (p.side(), p.line()))
                .collect()
        };
        assert_eq!(
            lines("value"),
            [(DiffSide::Original, 2), (DiffSide::Modified, 2)]
        );
        assert_eq!(lines("Value"), [(DiffSide::Original, 2)]);
        assert_eq!(lines("keep"), [(DiffSide::Modified, 1)]);
        assert!(lines("").is_empty());
    }
}
