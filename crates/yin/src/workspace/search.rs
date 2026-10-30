//! Project search (cmd-shift-f): ripgrep over the working tree, results by
//! file, opened at the matching line.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    list::ListItem,
    scroll::Scrollbar,
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, HighlightStyle, InteractiveElement as _,
    IntoElement as _, ParentElement as _, ScrollStrategy, Styled as _, StyledText, WeakEntity,
    Window, div, prelude::FluentBuilder as _, uniform_list,
};
use yin_fs::{SearchMatch, SearchOptions};

use super::{SEARCH_CONTEXT, Workspace};

const SEARCH_LIMIT: usize = 5000;

/// A row of the results list.
#[derive(Clone, Debug)]
pub(super) enum SearchRow {
    File { path: String, count: usize },
    Match(usize),
}

#[derive(Default)]
pub(super) struct SearchState {
    pub(super) matches: Arc<Vec<SearchMatch>>,
    pub(super) rows: Arc<Vec<SearchRow>>,
    pub(super) selected: Option<usize>,
    pub(super) truncated: bool,
    pub(super) running: bool,
    pub(super) error: Option<String>,
    pub(super) options: SearchOptions,
    /// The query the results belong to.
    pub(super) query: String,
    /// Shown instead of the result count, for results that aren't a search.
    pub(super) title: Option<String>,
    cancel: Arc<AtomicBool>,
}

fn rows(matches: &[SearchMatch]) -> Vec<SearchRow> {
    let mut rows = Vec::new();
    let mut ix = 0;
    while ix < matches.len() {
        let path = &matches[ix].path;
        let count = matches[ix..].iter().take_while(|m| &m.path == path).count();
        rows.push(SearchRow::File {
            path: path.clone(),
            count,
        });
        rows.extend((ix..ix + count).map(SearchRow::Match));
        ix += count;
    }
    rows
}

impl Workspace {
    pub(super) fn new_search_input(
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search the project"));
        cx.subscribe_in(
            &input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => this.schedule_search(cx),
                InputEvent::PressEnter { .. } => {
                    this.search_focus.focus(window, cx);
                    this.step_search(1, cx);
                }
                _ => {}
            },
        )
        .detach();
        input
    }

    pub(super) fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
    }

    /// Searches shortly after typing stops, cancelling any search under way.
    pub(super) fn schedule_search(&mut self, cx: &mut Context<Self>) {
        self.search.cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.search.cancel = cancel.clone();
        let query = self.search_input.read(cx).value().to_string();
        let options = self.search.options;
        let root = self.repo.workdir().to_owned();
        let files = self.files.clone();
        if query.is_empty() {
            self.search = SearchState {
                options,
                cancel,
                ..Default::default()
            };
            cx.notify();
            return;
        }
        self.search.running = true;
        cx.notify();
        self._search_task = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            let result = cx
                .background_spawn({
                    let query = query.clone();
                    let cancel = cancel.clone();
                    async move {
                        let scanned;
                        let files: &[String] = if files.is_empty() {
                            scanned = yin_fs::scan(&root)?;
                            &scanned
                        } else {
                            &files
                        };
                        yin_fs::search(&root, files, &query, options, SEARCH_LIMIT, &cancel)
                    }
                })
                .await;
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            this.update(cx, |this, cx| {
                let search = &mut this.search;
                search.running = false;
                search.query = query;
                match result {
                    Ok(results) => {
                        search.rows = Arc::new(rows(&results.matches));
                        search.matches = Arc::new(results.matches);
                        search.truncated = results.truncated;
                        search.error = None;
                    }
                    Err(error) => {
                        search.rows = Arc::default();
                        search.matches = Arc::default();
                        search.error = Some(error.to_string());
                    }
                }
                search.selected = None;
                cx.notify();
            })
            .ok();
        });
    }

    fn toggle_search_option(&mut self, regex: bool, cx: &mut Context<Self>) {
        let options = &mut self.search.options;
        if regex {
            options.regex = !options.regex;
        } else {
            options.case_sensitive = !options.case_sensitive;
        }
        self.schedule_search(cx);
    }

    /// Moves between matches, skipping file headers, and previews each one.
    pub(super) fn step_search(&mut self, direction: isize, cx: &mut Context<Self>) {
        let rows = self.search.rows.clone();
        let mut ix = match self.search.selected {
            Some(ix) => ix as isize + direction,
            None => 0,
        };
        while let Some(row) = usize::try_from(ix).ok().and_then(|ix| rows.get(ix)) {
            if matches!(row, SearchRow::Match(_)) {
                self.search.selected = Some(ix as usize);
                self.search_scroll
                    .scroll_to_item(ix as usize, ScrollStrategy::Nearest);
                cx.notify();
                return;
            }
            ix += if direction < 0 { -1 } else { 1 };
        }
    }

    /// Lists find-references results in the Search tab.
    pub(super) fn show_references(
        &mut self,
        symbol: String,
        matches: Vec<SearchMatch>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search.cancel.store(true, Ordering::Relaxed);
        let title = match matches.len() {
            0 => format!("No references to {symbol}"),
            1 => format!("1 reference to {symbol}"),
            n => format!("{n} references to {symbol}"),
        };
        self.search = SearchState {
            rows: Arc::new(rows(&matches)),
            matches: Arc::new(matches),
            title: Some(title),
            options: self.search.options,
            ..Default::default()
        };
        self.panel = super::Panel::Search;
        self.sidebar_visible = true;
        self.search_focus.focus(window, cx);
        self.step_search(1, cx);
        cx.notify();
    }

    pub(super) fn open_search_result(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(SearchRow::Match(ix)) = self
            .search
            .selected
            .and_then(|ix| self.search.rows.get(ix).cloned())
        else {
            return;
        };
        let Some(found) = self.search.matches.get(ix).cloned() else {
            return;
        };
        self.open_file(&found.path, Some(found.line as u32), window, cx);
    }

    pub(super) fn render_search(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let search = &self.search;
        let status = if search.running {
            "Searching…".to_owned()
        } else if let Some(error) = &search.error {
            error.clone()
        } else if let Some(title) = &search.title {
            title.clone()
        } else if search.query.is_empty() {
            String::new()
        } else {
            let files = search
                .rows
                .iter()
                .filter(|row| matches!(row, SearchRow::File { .. }))
                .count();
            format!(
                "{}{} results in {files} files",
                search.matches.len(),
                if search.truncated { "+" } else { "" },
            )
        };
        let rows = search.rows.clone();
        let matches = search.matches.clone();
        let selected = search.selected;
        let focused = self.search_focus.is_focused(window);
        let owner = cx.entity().downgrade();
        v_flex()
            .flex_1()
            .min_h_0()
            .child(
                v_flex()
                    .flex_none()
                    .p_2()
                    .gap_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .gap_1()
                            .child(Input::new(&self.search_input).small().flex_1())
                            .child(
                                Button::new("search-case")
                                    .xsmall()
                                    .ghost()
                                    .label("Aa")
                                    .selected(search.options.case_sensitive)
                                    .tooltip("Match case")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_search_option(false, cx)
                                    })),
                            )
                            .child(
                                Button::new("search-regex")
                                    .xsmall()
                                    .ghost()
                                    .label(".*")
                                    .selected(search.options.regex)
                                    .tooltip("Regular expression")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_search_option(true, cx)
                                    })),
                            ),
                    )
                    .when(!status.is_empty(), |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(if search.error.is_some() {
                                    cx.theme().red
                                } else {
                                    cx.theme().muted_foreground
                                })
                                .child(status),
                        )
                    }),
            )
            .child(
                div()
                    .id("search-results")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .track_focus(&self.search_focus)
                    .key_context(SEARCH_CONTEXT)
                    .child(
                        uniform_list("search-rows", rows.len(), move |range, _, cx| {
                            range
                                .map(|ix| {
                                    render_search_row(
                                        ix, &rows[ix], &matches, selected, focused, &owner, cx,
                                    )
                                })
                                .collect()
                        })
                        .size_full()
                        .track_scroll(&self.search_scroll),
                    )
                    .child(Scrollbar::vertical(&self.search_scroll)),
            )
            .into_any_element()
    }
}

fn render_search_row(
    ix: usize,
    row: &SearchRow,
    matches: &[SearchMatch],
    selected: Option<usize>,
    focused: bool,
    owner: &WeakEntity<Workspace>,
    cx: &App,
) -> AnyElement {
    match row {
        SearchRow::File { path, count } => {
            let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
            h_flex()
                .id(("search-file", ix))
                .w_full()
                .h_6()
                .px_2()
                .gap_2()
                .text_xs()
                .child(
                    div()
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .child(name.to_owned()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(cx.theme().muted_foreground)
                        .child(dir.to_owned()),
                )
                .child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(count.to_string()),
                )
                .into_any_element()
        }
        SearchRow::Match(match_ix) => {
            let found = &matches[*match_ix];
            // Leading indentation wastes the narrow sidebar.
            let trimmed = found.text.trim_start();
            let offset = found.text.len() - trimmed.len();
            let highlight = HighlightStyle {
                background_color: Some(cx.theme().yellow.opacity(0.3)),
                color: Some(cx.theme().foreground),
                ..Default::default()
            };
            let highlights: Vec<_> = found
                .ranges
                .iter()
                .filter(|range| range.start >= offset)
                .map(|range| (range.start - offset..range.end - offset, highlight))
                .collect();
            let is_selected = selected == Some(ix);
            let owner = owner.clone();
            ListItem::new(("search-match", ix))
                .w_full()
                .h_6()
                .pl_4()
                .pr_2()
                .selected(is_selected)
                .secondary_selected(is_selected && focused)
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .gap_2()
                        .text_xs()
                        .child(
                            div()
                                .w_8()
                                .flex_none()
                                .text_right()
                                .text_color(cx.theme().muted_foreground)
                                .child(found.line.to_string()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family(cx.theme().mono_font_family.clone())
                                .text_color(cx.theme().muted_foreground)
                                .child(
                                    StyledText::new(trimmed.to_owned()).with_highlights(highlights),
                                ),
                        ),
                )
                .on_click(move |_, window, cx| {
                    owner
                        .update(cx, |this, cx| {
                            this.search.selected = Some(ix);
                            this.open_search_result(window, cx);
                        })
                        .ok();
                })
                .into_any_element()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(path: &str, line: u64) -> SearchMatch {
        SearchMatch {
            path: path.into(),
            line,
            text: String::new(),
            ranges: Vec::new(),
        }
    }

    #[test]
    fn groups_matches_under_their_file() {
        let matches = [found("a.rs", 1), found("a.rs", 9), found("b.rs", 3)];
        let shape: Vec<String> = rows(&matches)
            .iter()
            .map(|row| match row {
                SearchRow::File { path, count } => format!("{path} {count}"),
                SearchRow::Match(ix) => format!("  {ix}"),
            })
            .collect();
        assert_eq!(shape, ["a.rs 2", "  0", "  1", "b.rs 1", "  2"]);
    }
}
