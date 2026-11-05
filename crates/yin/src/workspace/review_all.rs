//! The changes diff: every change against HEAD in one scrolling diff, for
//! reviewing an agent's work as a whole. Picking a file in the list scrolls
//! to it; conflicts and stashes show on their own.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    diff::{Diff, DiffFile},
    h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, InteractiveElement as _, IntoElement as _,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
};
use yin_git::FileStatus;

use super::annotations::DiffPane;
use super::{Section, Workspace, style};

impl Workspace {
    /// Whether the selected row is a conflict or a stash, which the main
    /// pane shows by itself instead of the changes diff.
    pub(super) fn shows_one_file(&self) -> bool {
        self.selected_key()
            .is_some_and(|key| matches!(key.section, Section::Conflicts | Section::Stashes))
    }

    /// Rebuilds the combined patch; keeps the scroll position when unchanged.
    pub(super) fn load_all_diff(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        let status = self.status.clone();
        let hidden = self.hidden_paths();
        let whole = self.whole_files.clone();
        self._all_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let patch = repo.diff_all(&status, &|path| hidden.contains(path), &|path| {
                        whole.contains(path)
                    })?;
                    let files = DiffFile::parse(&patch)
                        .map_err(|error| anyhow::anyhow!("couldn't read the diff: {error}"))?;
                    anyhow::Ok((patch, files))
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok((patch, files)) => {
                        if patch != this.all_patch {
                            this.all_patch = patch;
                            this.follow_notes();
                            this.research_diff(DiffPane::Changes, cx);
                            let reviewed = this.reviewed.clone();
                            this.all_diff.update(cx, |diff, cx| {
                                let before: std::collections::HashSet<String> = diff
                                    .files()
                                    .iter()
                                    .map(|file| file.path().to_string())
                                    .collect();
                                diff.set_files(files, cx);
                                // Viewed files arrive folded, as on GitHub.
                                for path in reviewed.iter().filter(|path| !before.contains(*path)) {
                                    diff.set_file_collapsed(path, true, cx);
                                }
                            });
                            this.schedule_relayout(cx);
                        }
                    }
                    Err(error) => this.error = Some(error.to_string().into()),
                }
                cx.notify();
            })
            .ok();
        });
    }

    /// Opens the file shown in the diff at the selected line, or the
    /// selected file at its first change.
    pub(super) fn open_in_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pane = self.active_pane();
        let selected = pane.and_then(|pane| {
            let selected = self
                .diff_in(pane)
                .read(cx)
                .selected_lines()
                .map(|range| (range.path().to_string(), range.end_side(), range.end()));
            // Else the line last clicked in this diff.
            selected.or_else(|| {
                self.clicked_line
                    .as_ref()
                    .filter(|(clicked, _)| *clicked == pane)
                    .map(|(_, at)| (at.path().to_string(), at.side(), at.line()))
            })
        });
        let target = match selected {
            Some((path, side, end)) => {
                let line = match side {
                    gpui_kit::component::diff::DiffSide::Modified => Some(end),
                    // A deleted line: where it was, in the file as it is now.
                    _ => {
                        let patch = match pane {
                            Some(DiffPane::History) => &self.history_patch,
                            _ => &self.all_patch,
                        };
                        yin_git::FilePatch::of_file(patch, &path, yin_git::Side::Old)
                            .and_then(|file| file.map_line(yin_git::Side::Old, end))
                    }
                };
                Some((path, line))
            }
            None if self.panel == super::Panel::Changes => self.selected_key().map(|key| {
                let line = self.first_change(&key.path);
                (key.path, line)
            }),
            None => None,
        };
        let Some((path, line)) = target else {
            return;
        };
        self.open_at(&path, line, window, cx);
    }

    /// Opens `path` from its header in the changes diff, at its first change.
    pub(super) fn open_path_in_file(
        &mut self,
        path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let line = self.first_change(path);
        self.open_at(path, line, window, cx);
    }

    fn first_change(&self, path: &str) -> Option<usize> {
        yin_git::FilePatch::of_file(&self.all_patch, path, yin_git::Side::New).and_then(|file| {
            file.rows()
                .find(|row| row.kind != yin_git::RowKind::Context)
                .and_then(|row| row.new_line.or(row.old_line))
        })
    }

    fn open_at(
        &mut self,
        path: &str,
        line: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.repo.workdir().join(path).is_file() {
            window.push_notification(format!("{path} isn't in the working tree."), cx);
            return;
        }
        self.show_panel(super::Panel::Files, window, cx);
        self.open_file(path, line.map(|line| line as u32), window, cx);
    }

    /// Brings the selected file into view in the combined diff.
    pub(super) fn scroll_all_diff_to_selection(&mut self, cx: &mut Context<Self>) {
        // Picking a file moves on from the line clicked before.
        self.clicked_line = None;
        if let Some(key) = self.selected_key() {
            self.all_diff
                .update(cx, |diff, cx| diff.scroll_to_file(&key.path, cx));
        }
    }

    pub(super) fn render_all_diff(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if self.all_diff.read(cx).files().is_empty() {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(if !self.loaded {
                    "Reading status…"
                } else if self.hide_reviewed && !self.reviewed.is_empty() {
                    "Everything is reviewed"
                } else {
                    "Nothing to review"
                })
                .into_any_element();
        }
        let selection_bar = self
            .all_diff
            .read(cx)
            .selected_lines()
            .map(|range| self.render_selection_bar(&range, window, cx));
        // How much of each file is staged, for its header.
        let staging: Arc<HashMap<String, Staging>> = Arc::new(
            self.status
                .entries
                .iter()
                .filter_map(|entry| match entry.status {
                    FileStatus::Tracked { index, worktree } if index.is_changed() => Some((
                        entry.path.clone(),
                        if worktree.is_changed() {
                            Staging::Partly
                        } else {
                            Staging::Fully
                        },
                    )),
                    _ => None,
                })
                .collect(),
        );
        v_flex()
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(self.render_diff_bar(DiffPane::Changes, cx))
            .child(div().flex_1().min_h_0().child(self.annotate(
                Diff::new(&self.all_diff).line_number(true).render_header({
                    let state = self.all_diff.clone();
                    let whole = self.whole_files.clone();
                    let reviewed = self.reviewed.clone();
                    let owner = cx.entity().downgrade();
                    move |file, _, cx| {
                        let staged =
                            staging
                                .get(file.path().as_ref())
                                .map(|staging| match staging {
                                    Staging::Fully => "Staged",
                                    Staging::Partly => "Partly staged",
                                });
                        let path = file.path().to_string();
                        let is_whole = whole.contains(&path);
                        let viewed = reviewed.contains(&path);
                        let (open_owner, view_owner, owner) =
                            (owner.clone(), owner.clone(), owner.clone());
                        let (open_path, view_path) = (path.clone(), path.clone());
                        let viewed_id = SharedString::from(format!("viewed-{path}"));
                        let viewed_box = SharedString::from(format!("viewed-box-{path}"));
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(style::diff_file_header(file, &state, staged, cx)),
                            )
                            .child(
                                Button::new(SharedString::from(format!("open-{path}")))
                                    .cursor_pointer()
                                    .ghost()
                                    .xsmall()
                                    .icon(Icon::empty().path("icons/file-input.svg"))
                                    .tooltip_with_action(
                                        "Open the file",
                                        &super::OpenInFile,
                                        Some("Workspace"),
                                    )
                                    .on_click(move |_, window, cx| {
                                        cx.stop_propagation();
                                        open_owner
                                            .update(cx, |this, cx| {
                                                this.open_path_in_file(&open_path, window, cx)
                                            })
                                            .ok();
                                    }),
                            )
                            .child(
                                Button::new(SharedString::from(format!("whole-{path}")))
                                    .cursor_pointer()
                                    .ghost()
                                    .xsmall()
                                    .icon(Icon::empty().path(if is_whole {
                                        "icons/fold-vertical.svg"
                                    } else {
                                        "icons/unfold-vertical.svg"
                                    }))
                                    .tooltip(if is_whole {
                                        "Show only the changes"
                                    } else {
                                        "Show the whole file"
                                    })
                                    .on_click(move |_, _, cx| {
                                        cx.stop_propagation();
                                        owner
                                            .update(cx, |this, cx| {
                                                this.toggle_whole_file(&path, cx)
                                            })
                                            .ok();
                                    }),
                            )
                            .child(
                                div()
                                    .id(viewed_id)
                                    .pl_1()
                                    .pr_2()
                                    .on_click(|_, _, cx| cx.stop_propagation())
                                    .child(
                                        Checkbox::new(viewed_box)
                                            .small()
                                            .cursor_pointer()
                                            .checked(viewed)
                                            .label("Viewed")
                                            .on_click(move |checked, _, cx| {
                                                cx.stop_propagation();
                                                view_owner
                                                    .update(cx, |this, cx| {
                                                        this.set_viewed(&view_path, *checked, cx)
                                                    })
                                                    .ok();
                                            }),
                                    ),
                            )
                    }
                }),
                DiffPane::Changes,
                cx,
            )))
            .children(selection_bar)
            .children(self.render_note_draft(DiffPane::Changes, window, cx))
            .into_any_element()
    }
}

#[derive(Clone, Copy)]
enum Staging {
    Fully,
    Partly,
}
