//! Highlights in open files: the matches of the project search that opened a
//! file, and the other occurrences of whatever text is selected.

use std::ops::Range;

use gpui_kit::component::{
    ActiveTheme as _,
    input::{EditorState, RangeDecoration, RangeDecorationCollection, RangeDecorationStyle},
};
use gpui_kit::{Context, Entity, Hsla};

use super::Workspace;

/// Selections longer than this aren't looked for elsewhere.
const MAX_SELECTION: usize = 200;
/// Enough occurrences to see the pattern; more only costs painting.
const MAX_OCCURRENCES: usize = 2000;

#[derive(Default)]
pub(super) struct FileMarks {
    /// Opened from the project search: show its matches.
    pub(super) show_search: bool,
    search: Option<RangeDecorationCollection>,
    selection: Option<RangeDecorationCollection>,
    /// The selection the occurrences were found for.
    selected: Range<usize>,
}

impl Workspace {
    /// Highlights the project search's matches in open file `ix`, if it was
    /// opened from the search.
    pub(super) fn mark_search_matches(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(file) = self.open_files.get(ix) else {
            return;
        };
        let Some((path, editor)) = file.editor() else {
            return;
        };
        if !file.marks.show_search {
            return;
        }
        let text = editor.read(cx).value().to_string();
        let starts = line_starts(&text);
        let ranges: Vec<Range<usize>> = self
            .search
            .matches
            .iter()
            .filter(|found| found.path == path)
            .flat_map(|found| {
                let start = starts.get(found.line.saturating_sub(1) as usize).copied();
                found
                    .ranges
                    .iter()
                    .filter_map(move |range| Some(start? + range.start..start? + range.end))
            })
            .filter(|range| range.end <= text.len())
            .collect();
        let color = cx.theme().yellow.opacity(0.3);
        let editor = editor.clone();
        let marks = fills(&ranges, color);
        let file = &mut self.open_files[ix];
        set_marks(&mut file.marks.search, &editor, marks, cx);
    }

    /// Re-marks every file opened from the search, after a new search.
    pub(super) fn refresh_search_marks(&mut self, cx: &mut Context<Self>) {
        for ix in 0..self.open_files.len() {
            self.mark_search_matches(ix, cx);
        }
    }

    /// Highlights the other places the selected text appears in `path`.
    pub(super) fn mark_selection(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(ix) = self.open_files.iter().position(|file| file.path == path) else {
            return;
        };
        let Some((_, editor)) = self.open_files[ix].editor() else {
            return;
        };
        let editor = editor.clone();
        let selected = editor.read(cx).selected_range();
        if self.open_files[ix].marks.selected == selected {
            return;
        }
        self.open_files[ix].marks.selected = selected.clone();
        let ranges = {
            let state = editor.read(cx);
            let needle = state.selected_text().to_string();
            if worth_marking(&needle) {
                occurrences(&state.value(), &needle, &selected)
            } else {
                Vec::new()
            }
        };
        let color = cx.theme().foreground.opacity(0.12);
        let marks = fills(&ranges, color);
        set_marks(&mut self.open_files[ix].marks.selection, &editor, marks, cx);
    }
}

fn fills(ranges: &[Range<usize>], color: Hsla) -> Vec<RangeDecoration> {
    ranges
        .iter()
        .map(|range| {
            RangeDecoration::new(range.clone())
                .with_style(RangeDecorationStyle::Fill)
                .with_color(color)
        })
        .collect()
}

/// Replaces a file's marks of one kind, creating their collection on first use.
fn set_marks(
    collection: &mut Option<RangeDecorationCollection>,
    editor: &Entity<EditorState>,
    marks: Vec<RangeDecoration>,
    cx: &mut Context<Workspace>,
) {
    match collection {
        Some(collection) => collection.set(marks, cx),
        None if marks.is_empty() => {}
        None => {
            *collection = Some(editor.update(cx, |editor, cx| {
                editor.create_range_decorations_collection(marks, cx)
            }));
        }
    }
}

/// Byte offset of each line's start.
fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(ix, _)| ix + 1))
        .collect()
}

/// A selection worth looking for elsewhere: part of one line, not blank.
fn worth_marking(needle: &str) -> bool {
    !needle.trim().is_empty() && !needle.contains('\n') && needle.len() <= MAX_SELECTION
}

/// Where `needle` appears in `text`, apart from the selection itself.
fn occurrences(text: &str, needle: &str, selected: &Range<usize>) -> Vec<Range<usize>> {
    text.match_indices(needle)
        .map(|(start, found)| start..start + found.len())
        .filter(|range| range != selected)
        .take(MAX_OCCURRENCES)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_other_occurrences_of_a_selection() {
        let text = "let x = 1;\nx + x\n";
        assert_eq!(occurrences(text, "x", &(4..5)), vec![11..12, 15..16]);
        assert!(worth_marking("x"));
        assert!(!worth_marking("  "));
        assert!(!worth_marking("a\nb"));
        assert_eq!(line_starts(text), [0, 11, 17]);
    }
}
