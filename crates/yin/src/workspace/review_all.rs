//! The changes diff: every change against HEAD in one scrolling diff, for
//! reviewing an agent's work as a whole. Picking a file in the list scrolls
//! to it; conflicts and stashes show on their own.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::component::{
    ActiveTheme as _,
    diff::{Diff, DiffFile},
    v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, IntoElement as _, ParentElement as _, Styled as _,
    Window, div,
};
use yin_git::FileStatus;

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
        self._all_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let patch = repo.diff_all(&status, &|path| hidden.contains(path))?;
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
                            this.all_diff
                                .update(cx, |diff, cx| diff.set_files(files, cx));
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

    /// Brings the selected file into view in the combined diff.
    pub(super) fn scroll_all_diff_to_selection(&mut self, cx: &mut Context<Self>) {
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
            .child(div().flex_1().min_h_0().child(
                Diff::new(&self.all_diff).line_number(true).render_header({
                    let state = self.all_diff.clone();
                    move |file, _, cx| {
                        let staged =
                            staging
                                .get(file.path().as_ref())
                                .map(|staging| match staging {
                                    Staging::Fully => "Staged",
                                    Staging::Partly => "Partly staged",
                                });
                        style::diff_file_header(file, &state, staged, cx)
                    }
                }),
            ))
            .children(selection_bar)
            .into_any_element()
    }
}

#[derive(Clone, Copy)]
enum Staging {
    Fully,
    Partly,
}
