//! Shared visual conventions, so the same kind of information looks the same
//! everywhere: additions green, deletions red, people at full strength,
//! commit hashes, times and paths muted.

use gpui_kit::component::{
    ActiveTheme as _, StyledExt as _,
    diff::{DiffFile, DiffFileStatus, DiffState},
    h_flex,
};
use gpui_kit::{
    AnyElement, App, Div, Entity, Hsla, InteractiveElement as _, IntoElement as _,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, div,
    prelude::FluentBuilder as _,
};

/// A short commit hash: monospace, quiet.
pub(super) fn hash(oid: &str, cx: &App) -> Div {
    div()
        .flex_none()
        .font_family(cx.theme().mono_font_family.clone())
        .text_color(cx.theme().muted_foreground)
        .child(oid[..oid.len().min(7)].to_owned())
}

/// `+12 −3`, each coloured only when it isn't zero.
pub(super) fn counts(additions: usize, deletions: usize, cx: &App) -> Div {
    let muted = cx.theme().muted_foreground.opacity(0.6);
    let tint = |n: usize, color: Hsla| if n > 0 { color } else { muted };
    h_flex()
        .flex_none()
        .gap_1p5()
        .text_xs()
        .font_family(cx.theme().mono_font_family.clone())
        .child(
            div()
                .text_color(tint(additions, cx.theme().green))
                .child(format!("+{additions}")),
        )
        .child(
            div()
                .text_color(tint(deletions, cx.theme().red))
                .child(format!("−{deletions}")),
        )
}

/// A small coloured label, such as a file's status.
fn tag(label: &'static str, color: Hsla) -> Div {
    div()
        .flex_none()
        .px_1p5()
        .rounded_sm()
        .text_xs()
        .text_color(color)
        .bg(color.opacity(0.14))
        .child(label)
}

/// The header of a file in a diff: its name first, its folder muted, the
/// change counts, and what happened to it when that's more than an edit.
/// `note`, such as how much of it is staged, shows quietly before the
/// counts. Clicking anywhere on it collapses or expands the file.
pub(super) fn diff_file_header(
    file: &DiffFile,
    state: &Entity<DiffState>,
    note: Option<&'static str>,
    cx: &App,
) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let renamed_from = match (file.original_path(), file.modified_path()) {
        (Some(old), Some(new)) if old != new => Some(old.clone()),
        _ => None,
    };
    let path = file.path();
    let (dir, name) = path.rsplit_once('/').unwrap_or(("", path.as_ref()));
    let status = match file.status() {
        DiffFileStatus::Added => Some(tag("New", cx.theme().green)),
        DiffFileStatus::Deleted => Some(tag("Deleted", cx.theme().red)),
        DiffFileStatus::Renamed => Some(tag("Renamed", cx.theme().blue)),
        DiffFileStatus::Copied => Some(tag("Copied", cx.theme().blue)),
        DiffFileStatus::Conflicted => Some(tag("Conflict", cx.theme().yellow)),
        _ => None,
    };
    let toggle_path = path.to_string();
    let state = state.clone();
    h_flex()
        .id(SharedString::from(format!("file-header-{path}")))
        .w_full()
        .min_w_0()
        .gap_2()
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            state.update(cx, |state, cx| {
                let collapsed = state.is_file_collapsed(&toggle_path);
                state.set_file_collapsed(&toggle_path, !collapsed, cx);
            });
        })
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap_2()
                .child(div().flex_none().font_medium().child(name.to_owned()))
                .when(!dir.is_empty(), |this| {
                    this.child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(muted)
                            .child(dir.to_owned()),
                    )
                })
                .when_some(renamed_from, |this, old| {
                    this.child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(muted)
                            .child(format!("from {old}")),
                    )
                }),
        )
        .when_some(note, |this, note| {
            this.child(tag(note, cx.theme().muted_foreground))
        })
        .children(status)
        .when(!file.is_binary(), |this| {
            this.child(counts(file.additions(), file.deletions(), cx))
        })
        .into_any_element()
}
