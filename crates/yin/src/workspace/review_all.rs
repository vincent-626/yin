//! The all-changes view: every change against HEAD in one scrolling diff,
//! for reviewing an agent's work as a whole.

use gpui_kit::component::{
    ActiveTheme as _,
    diff::{Diff, DiffFile},
    v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, IntoElement as _, ParentElement as _, Styled as _, div,
};

use super::Workspace;

impl Workspace {
    pub(super) fn toggle_review_all(&mut self, cx: &mut Context<Self>) {
        self.review_all = !self.review_all;
        if self.review_all {
            self.load_all_diff(cx);
        }
        cx.notify();
    }

    /// Rebuilds the combined patch; keeps the scroll position when unchanged.
    pub(super) fn load_all_diff(&mut self, cx: &mut Context<Self>) {
        if !self.review_all {
            return;
        }
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

    pub(super) fn render_all_diff(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.all_diff.read(cx).files().is_empty() {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(if self.hide_reviewed && !self.reviewed.is_empty() {
                    "Everything is reviewed"
                } else {
                    "Nothing to review"
                })
                .into_any_element();
        }
        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(Diff::new(&self.all_diff).line_number(true))
            .into_any_element()
    }
}
