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
use yin_git::{FileStatus, LinePosition, Side};

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
    /// start in. Changes that are all staged already unstage instead.
    pub(super) fn stage_selection(
        &mut self,
        whole_hunk: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.selected_key().map(|key| key.section) {
            Some(Section::Conflicts) => {
                window.push_notification("Resolve the conflict, then mark it resolved.", cx);
                return;
            }
            Some(Section::Stashes) => {
                window.push_notification("Pop the stash to stage its changes.", cx);
                return;
            }
            _ => {}
        }
        let Some(range) = self.all_diff.read(cx).selected_lines() else {
            window.push_notification("Select lines in the diff first.", cx);
            return;
        };
        let path = range.path().to_string();
        let whole_file_only = self
            .status
            .entries
            .iter()
            .any(|entry| entry.path == path && matches!(entry.status, FileStatus::Untracked));
        if whole_file_only {
            window.push_notification("Stage a new file as a whole (space in the list).", cx);
            return;
        }
        let start = position(range.side(), range.start());
        let end = position(range.end_side(), range.end());
        self.run_then(
            move |repo| {
                repo.stage_from_head(&path, start, end, whole_hunk)
                    .map(drop)
            },
            |this, _, cx| {
                this.all_diff
                    .update(cx, |diff, cx| diff.set_selected_lines(None, cx));
            },
            window,
            cx,
        );
    }

    /// Hints floating over the diff while lines are selected. They float so
    /// the lines don't move under the pointer when a selection starts.
    pub(super) fn render_selection_bar(
        &self,
        range: &DiffLineRange,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let lines = if range.side() == range.end_side() {
            let count = range.start().abs_diff(range.end()) + 1;
            format!("{count} line{} selected", if count == 1 { "" } else { "s" })
        } else {
            "Lines selected".into()
        };
        let hint = |action: &dyn gpui_kit::Action, label: &'static str| {
            h_flex()
                .gap_1()
                .children(Kbd::binding_for_action(action, Some("Diff"), window))
                .child(label)
        };
        h_flex()
            .absolute()
            .bottom_3()
            .right_4()
            .h_8()
            .px_3()
            .gap_4()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .shadow_md()
            .bg(cx.theme().popover)
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(div().text_color(cx.theme().foreground).child(lines))
            .child(hint(&StageLines, "Stage or unstage lines"))
            .child(hint(&StageHunk, "Stage or unstage hunk"))
            .into_any_element()
    }
}
