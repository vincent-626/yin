//! Staging and unstaging selected lines or hunks from the diff.

use gpui_kit::component::{
    ActiveTheme as _, WindowExt as _,
    diff::{DiffLineRange, DiffSide},
    h_flex,
    kbd::Kbd,
};
use gpui_kit::{
    AnyElement, Context, IntoElement as _, ParentElement as _, Styled as _, Window, div,
};
use yin_git::{FilePatch, LinePosition, Side};

use super::{Section, StageHunk, StageLines, Workspace};

fn position(side: DiffSide, line: usize) -> LinePosition {
    LinePosition {
        side: match side {
            DiffSide::Original => Side::Old,
            _ => Side::New,
        },
        line,
    }
}

impl Workspace {
    /// Stages the diff's selected lines, or with `whole_hunk` the hunk they
    /// start in. In the Staged section, unstages instead.
    pub(super) fn stage_selection(
        &mut self,
        whole_hunk: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.review_all {
            // The combined diff is against HEAD, not the index.
            window.push_notification("Switch to one file (a) to stage lines.", cx);
            return;
        }
        let Some(key) = self.diff_key.clone() else {
            return;
        };
        let Some(range) = self.diff.read(cx).selected_lines() else {
            window.push_notification("Select lines in the diff first.", cx);
            return;
        };
        let reverse = match key.section {
            Section::Staged => true,
            Section::Changes => false,
            Section::Untracked | Section::Conflicts => {
                window.push_notification("Stage this file as a whole.", cx);
                return;
            }
            Section::Stashes => {
                window.push_notification("Pop the stash to stage its changes.", cx);
                return;
            }
        };
        let start = position(range.side(), range.start());
        let end = position(range.end_side(), range.end());
        let patch = self.diff_patch.clone();
        self.run(
            move |repo| {
                let patch = FilePatch::parse(&patch)?;
                let rows = if whole_hunk {
                    patch.hunk_rows(start)
                } else {
                    patch.rows_between(start, end)
                };
                let partial = patch.select(&rows, reverse)?;
                repo.apply_to_index(&partial, reverse)
            },
            window,
            cx,
        );
    }

    /// Hints shown over the diff while lines are selected.
    pub(super) fn render_selection_bar(
        &self,
        range: &DiffLineRange,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let section = self.diff_key.as_ref()?.section;
        if !matches!(section, Section::Staged | Section::Changes) {
            return None;
        }
        let verb = if section.is_staged() {
            "Unstage"
        } else {
            "Stage"
        };
        let lines = if range.side() == range.end_side() {
            let count = range.start().abs_diff(range.end()) + 1;
            format!("{count} line{} selected", if count == 1 { "" } else { "s" })
        } else {
            "Lines selected".into()
        };
        let hint = |action: &dyn gpui_kit::Action, label: String| {
            h_flex()
                .gap_1()
                .children(Kbd::binding_for_action(action, Some("Diff"), window))
                .child(label)
        };
        Some(
            h_flex()
                .flex_none()
                .h_8()
                .px_3()
                .gap_4()
                .border_b_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().secondary)
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(div().text_color(cx.theme().foreground).child(lines))
                .child(hint(&StageLines, format!("{verb} lines")))
                .child(hint(&StageHunk, format!("{verb} hunk")))
                .into_any_element(),
        )
    }
}
