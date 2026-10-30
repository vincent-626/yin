//! Merge conflicts: resolve each in the diff, then mark the file resolved.

use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariant, ButtonVariants as _},
    h_flex,
};
use gpui_kit::{
    AnyElement, Context, IntoElement as _, ParentElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};

use yin_git::InProgress;

use super::{FileKey, Section, Workspace};

impl Workspace {
    /// Stages a conflicted file: with the resolutions chosen in the diff, or
    /// as it is when it no longer contains conflict markers.
    pub(super) fn mark_resolved(
        &mut self,
        key: FileKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shown = self.diff_key.as_ref() == Some(&key);
        let resolved = shown
            .then(|| self.diff.read(cx).resolved_text(&key.path))
            .flatten();
        let has_markers = shown && !self.unresolved_conflicts(cx).is_some_and(|n| n == 0);
        if resolved.is_none() && has_markers {
            window.push_notification("Choose a side for each conflict first.", cx);
            return;
        }
        let path = key.path;
        self.run(
            move |repo| match resolved {
                Some(text) => repo.resolve_conflict(&path, &text),
                None => {
                    if repo.read_file(&path)?.contains("\n<<<<<<< ") {
                        anyhow::bail!("{path} still contains conflict markers");
                    }
                    repo.stage(&[&path])
                }
            },
            window,
            cx,
        );
    }

    /// Conflicts in the shown file not yet given a resolution.
    fn unresolved_conflicts(&self, cx: &Context<Self>) -> Option<usize> {
        let key = self.diff_key.as_ref()?;
        let diff = self.diff.read(cx);
        let file = diff
            .files()
            .iter()
            .find(|file| file.path().as_ref() == key.path)?;
        Some(
            (0..file.conflicts().len())
                .filter(|ix| diff.conflict_resolution(&key.path, *ix).is_none())
                .count(),
        )
    }

    pub(super) fn render_conflict_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let key = self.diff_key.clone()?;
        if key.section != Section::Conflicts {
            return None;
        }
        let total = self
            .diff
            .read(cx)
            .files()
            .first()
            .map_or(0, |file| file.conflicts().len());
        let unresolved = self.unresolved_conflicts(cx).unwrap_or(0);
        let summary = match (total, unresolved) {
            (0, _) => "No conflict markers left".to_owned(),
            (_, 0) => format!("All {total} conflicts resolved"),
            (_, n) => format!("{n} of {total} conflicts unresolved"),
        };
        Some(
            h_flex()
                .flex_none()
                .h_9()
                .px_3()
                .gap_3()
                .border_b_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().secondary)
                .text_xs()
                .child(div().flex_1().child(summary))
                .child(
                    Button::new("mark-resolved")
                        .xsmall()
                        .primary()
                        .label("Mark resolved")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.mark_resolved(key.clone(), window, cx)
                        })),
                )
                .into_any_element(),
        )
    }

    pub(super) fn render_merge_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let in_progress = self.in_progress?;
        let rebasing = in_progress == InProgress::Rebase;
        Some(
            h_flex()
                .flex_none()
                .px_3()
                .py_1p5()
                .gap_2()
                .border_b_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().warning.opacity(0.15))
                .text_xs()
                .child(div().flex_1().child(if rebasing {
                    "Rebase in progress"
                } else {
                    "Merge in progress"
                }))
                .when(rebasing, |this| {
                    this.child(
                        Button::new("continue-rebase")
                            .xsmall()
                            .primary()
                            .label("Continue")
                            .tooltip("Resolve and stage conflicts first")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.run(|repo| repo.continue_rebase(), window, cx)
                            })),
                    )
                })
                .child(
                    Button::new("abort-operation")
                        .xsmall()
                        .ghost()
                        .label("Abort")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.confirm_abort(in_progress, window, cx)
                        })),
                )
                .into_any_element(),
        )
    }

    fn confirm_abort(
        &mut self,
        in_progress: InProgress,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let this = cx.entity().downgrade();
        let name = match in_progress {
            InProgress::Merge => "merge",
            InProgress::Rebase => "rebase",
        };
        window.open_alert_dialog(cx, move |alert, _, _| {
            let this = this.clone();
            alert
                .title(format!("Abort the {name}?"))
                .description(format!(
                    "The branch returns to how it was before the {name}."
                ))
                .confirm()
                .ok_text(format!("Abort {name}"))
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    this.update(cx, |this, cx| {
                        this.run(
                            move |repo| match in_progress {
                                InProgress::Merge => repo.abort_merge(),
                                InProgress::Rebase => repo.abort_rebase(),
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
}
