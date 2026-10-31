//! Find in a rendered markdown preview (cmd-f): matches are highlighted in
//! the rendered text, and enter steps through them.

use std::ops::Range;

use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    text::{RangeHighlight, TextViewState},
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement as _,
    ParentElement as _, Styled as _, Subscription, Window, div,
};

use super::{ClosePreviewFind, Workspace};

pub(super) struct PreviewFind {
    /// The preview searched; another tab's preview has no find bar.
    preview: Entity<TextViewState>,
    input: Entity<InputState>,
    matches: Vec<Range<usize>>,
    current: usize,
    _input_events: Subscription,
}

/// Byte ranges of `query` in `text`; smart case, as in project search.
pub(super) fn find_all(text: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    let exact = query.chars().any(char::is_uppercase);
    // ASCII lowering keeps byte offsets, so ranges index the original text.
    let (haystack, needle) = if exact {
        (text.to_owned(), query.to_owned())
    } else {
        (text.to_ascii_lowercase(), query.to_ascii_lowercase())
    };
    haystack
        .match_indices(&needle)
        .map(|(start, found)| start..start + found.len())
        .collect()
}

impl Workspace {
    pub(super) fn open_preview_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(preview) = self.active_preview() else {
            return;
        };
        if let Some(find) = self.preview_find.as_ref().filter(|f| f.preview == preview) {
            let input = find.input.clone();
            input.update(cx, |input, cx| input.focus(window, cx));
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in preview"));
        let events =
            cx.subscribe_in(
                &input,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Change => this.search_preview(cx),
                    InputEvent::PressEnter { shift, .. } => {
                        this.step_preview_find(if *shift { -1 } else { 1 }, cx)
                    }
                    _ => {}
                },
            );
        input.update(cx, |input, cx| input.focus(window, cx));
        self.preview_find = Some(PreviewFind {
            preview,
            input,
            matches: Vec::new(),
            current: 0,
            _input_events: events,
        });
        cx.notify();
    }

    fn search_preview(&mut self, cx: &mut Context<Self>) {
        let Some(find) = &mut self.preview_find else {
            return;
        };
        let query = find.input.read(cx).value().to_string();
        let text = find.preview.read(cx).rendered_text();
        find.matches = find_all(text.as_str(), &query);
        find.current = 0;
        self.show_preview_matches(cx);
    }

    fn step_preview_find(&mut self, direction: isize, cx: &mut Context<Self>) {
        let Some(find) = &mut self.preview_find else {
            return;
        };
        let count = find.matches.len();
        if count == 0 {
            return;
        }
        find.current = (find.current as isize + direction).rem_euclid(count as isize) as usize;
        self.show_preview_matches(cx);
    }

    /// Paints every match, the current one stronger, and scrolls to it.
    fn show_preview_matches(&mut self, cx: &mut Context<Self>) {
        let Some(find) = &self.preview_find else {
            return;
        };
        let yellow = cx.theme().yellow;
        let mut highlights: Vec<RangeHighlight> = find
            .matches
            .iter()
            .enumerate()
            .filter(|(ix, _)| *ix != find.current)
            .map(|(_, range)| RangeHighlight::new(range.clone(), yellow.opacity(0.25)))
            .collect();
        let current = find.matches.get(find.current).cloned();
        highlights.extend(
            current
                .clone()
                .map(|range| RangeHighlight::new(range, yellow.opacity(0.6))),
        );
        find.preview.update(cx, |preview, cx| {
            // The text may have reloaded since it was searched; then the
            // next keystroke searches again.
            preview.set_range_highlights(highlights, cx).ok();
            if let Some(range) = current {
                preview.reveal_range(range, cx).ok();
            }
        });
        cx.notify();
    }

    pub(super) fn close_preview_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(find) = self.preview_find.take() {
            find.preview
                .update(cx, |preview, cx| preview.clear_range_highlights(cx));
            self.focus_active_file(window, cx);
            cx.notify();
        }
    }

    pub(super) fn render_preview_find(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let find = self
            .preview_find
            .as_ref()
            .filter(|find| Some(&find.preview) == self.active_preview().as_ref())?;
        let query_empty = find.input.read(cx).value().is_empty();
        let summary = match find.matches.len() {
            _ if query_empty => String::new(),
            0 => "No results".into(),
            n => format!("{} of {n}", find.current + 1),
        };
        Some(
            h_flex()
                .key_context("PreviewFind")
                .on_action(cx.listener(|this, _: &ClosePreviewFind, window, cx| {
                    this.close_preview_find(window, cx)
                }))
                .flex_none()
                .px_3()
                .py_1()
                .gap_2()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(div().w_64().child(Input::new(&find.input).small()))
                .child(
                    div()
                        .w_20()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(summary),
                )
                .child(
                    Button::new("preview-find-previous")
                        .ghost()
                        .xsmall()
                        .icon(IconName::ArrowUp)
                        .tooltip("Previous (shift-enter)")
                        .on_click(cx.listener(|this, _, _, cx| this.step_preview_find(-1, cx))),
                )
                .child(
                    Button::new("preview-find-next")
                        .ghost()
                        .xsmall()
                        .icon(IconName::ArrowDown)
                        .tooltip("Next (enter)")
                        .on_click(cx.listener(|this, _, _, cx| this.step_preview_find(1, cx))),
                )
                .child(div().flex_1())
                .child(
                    Button::new("preview-find-close")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .tooltip("Close (escape)")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.close_preview_find(window, cx)),
                        ),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::find_all;

    #[test]
    fn finds_with_smart_case() {
        assert_eq!(find_all("Rust and rust", "rust"), [0..4, 9..13]);
        assert_eq!(find_all("Rust and rust", "Rust"), vec![0..4]);
        assert_eq!(find_all("ünïcode rust", "rust"), vec![10..14]);
        assert!(find_all("abc", "").is_empty());
    }
}
