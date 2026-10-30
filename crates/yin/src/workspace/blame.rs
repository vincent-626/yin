//! Blame for an open file: each line with the commit that last changed it.

use std::sync::Arc;

use gpui_kit::component::{ActiveTheme as _, WindowExt as _, notification::Notification, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, InteractiveElement as _, IntoElement as _,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div,
    prelude::FluentBuilder as _, uniform_list,
};
use yin_git::BlameLine;

use super::Workspace;
use super::history::relative_time;

impl Workspace {
    /// Shows or hides blame for the active file.
    pub(super) fn toggle_blame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self
            .active_file
            .and_then(|ix| self.open_files.get(ix))
            .map(|file| file.path.clone())
        else {
            return;
        };
        if self
            .blame
            .as_ref()
            .is_some_and(|(blamed, _)| *blamed == path)
        {
            self.blame = None;
            self.focus_active_file(window, cx);
            cx.notify();
            return;
        }
        let repo = self.repo.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn({
                    let path = path.clone();
                    async move { repo.blame(&path) }
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(lines) => {
                        this.blame = Some((path, Arc::new(lines)));
                        // The editor is hidden now; keep keys working.
                        this.focus_active_file(window, cx);
                    }
                    Err(error) => window.push_notification(
                        Notification::error(format!("Couldn't blame {path}: {error}")),
                        cx,
                    ),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The blame view when it is on for `path`.
    pub(super) fn render_blame(&self, path: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (blamed, lines) = self.blame.as_ref()?;
        if blamed != path {
            return None;
        }
        let lines = lines.clone();
        let owner = cx.entity().downgrade();
        Some(
            v_flex()
                .size_full()
                .child(
                    uniform_list("blame", lines.len(), move |range, _, cx| {
                        range
                            .map(|ix| render_blame_line(ix, &lines, &owner, cx))
                            .collect()
                    })
                    .size_full(),
                )
                .into_any_element(),
        )
    }
}

fn render_blame_line(
    ix: usize,
    lines: &[BlameLine],
    owner: &WeakEntity<Workspace>,
    cx: &App,
) -> AnyElement {
    let line = &lines[ix];
    // Commit details only where a run of lines from one commit starts.
    let starts_group = ix == 0 || lines[ix - 1].oid != line.oid;
    let muted = cx.theme().muted_foreground;
    let owner = owner.clone();
    let oid = line.oid.clone();
    let committed = line.is_committed();
    div()
        .id(("blame-line", ix))
        .flex()
        .w_full()
        .h_5()
        .text_xs()
        .font_family(cx.theme().mono_font_family.clone())
        .when(starts_group && ix > 0, |this| {
            this.border_t_1()
                .border_color(cx.theme().border.opacity(0.5))
        })
        .child(
            div()
                .id(("blame-commit", ix))
                .flex_none()
                .w_80()
                .px_2()
                .flex()
                .gap_2()
                .text_color(muted)
                .when(starts_group, |this| {
                    if committed {
                        this.cursor_pointer()
                            .hover(|this| this.text_color(cx.theme().foreground))
                            .child(super::style::hash(&line.oid, cx))
                            .child(
                                div()
                                    .w_24()
                                    .flex_none()
                                    .truncate()
                                    .text_color(cx.theme().foreground.opacity(0.8))
                                    .child(line.author.clone()),
                            )
                            .child(div().flex_1().truncate().child(line.summary.clone()))
                            .child(div().flex_none().child(relative_time(line.time)))
                            .on_click(move |_, window, cx| {
                                let oid = oid.clone();
                                owner
                                    .update(cx, |this, cx| this.show_commit(oid, window, cx))
                                    .ok();
                            })
                    } else {
                        this.child(
                            div()
                                .text_color(cx.theme().yellow)
                                .child("Not committed yet"),
                        )
                    }
                }),
        )
        .child(
            div()
                .flex_none()
                .w_10()
                .pr_2()
                .text_right()
                .text_color(muted)
                .child((ix + 1).to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .whitespace_nowrap()
                .child(line.text.clone()),
        )
        .into_any_element()
}
