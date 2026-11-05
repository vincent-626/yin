//! Inline blame, as in Zed: after the cursor's line in an open file, who
//! last changed it, when, and why. Blame runs on the editor's text, so
//! unsaved lines read as not committed yet.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::input::EditorState;
use gpui_kit::component::{ActiveTheme as _, input::RopeExt as _};
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, ContentMask, Context, Entity, Focusable as _,
    IntoElement as _, Pixels, SharedString, Styled as _, Task, TextAlign, TextRun, Window, canvas,
    font, point, px,
};
use yin_git::BlameLine;

use super::Workspace;
use super::history::relative_time;

/// Edits settle this long before the file is blamed again.
const AFTER_EDITS: Duration = Duration::from_millis(400);

pub(super) struct LineBlame {
    lines: Arc<Vec<BlameLine>>,
    /// False between an edit and the blame that follows it: lines may have
    /// moved, so nothing shows.
    fresh: bool,
    _task: Option<Task<()>>,
}

impl Workspace {
    /// Blames open file `path` again, now or once edits settle.
    pub(super) fn refresh_line_blame(
        &mut self,
        path: &str,
        after_edits: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self
            .open_files
            .iter()
            .find(|file| file.path == path)
            .and_then(|file| file.editor())
            .map(|(_, editor)| editor.clone())
        else {
            return;
        };
        // Toolchain sources outside the repository have no blame.
        if std::path::Path::new(path).is_absolute() {
            return;
        }
        let repo = self.repo.clone();
        let blamed = path.to_owned();
        let task = cx.spawn(async move |this, cx| {
            if after_edits {
                cx.background_executor().timer(AFTER_EDITS).await;
            }
            let Ok(text) = this.update(cx, |_, cx| editor.read(cx).value().to_string()) else {
                return;
            };
            let result = cx
                .background_spawn({
                    let blamed = blamed.clone();
                    async move { repo.blame_contents(&blamed, &text) }
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(lines) => {
                        if let Some(blame) = this.line_blame.get_mut(&blamed) {
                            blame.lines = Arc::new(lines);
                            blame.fresh = true;
                        }
                    }
                    // Untracked, or not a file git knows.
                    Err(_) => {
                        this.line_blame.remove(&blamed);
                    }
                }
                cx.notify();
            })
            .ok();
        });
        let blame = self
            .line_blame
            .entry(path.to_owned())
            .or_insert_with(|| LineBlame {
                lines: Arc::default(),
                fresh: false,
                _task: None,
            });
        if after_edits {
            blame.fresh = false;
        }
        blame._task = Some(task);
    }

    /// After a commit, checkout or reset, every open file's blame changes.
    pub(super) fn refresh_all_line_blame(&mut self, cx: &mut Context<Self>) {
        let paths: Vec<String> = self
            .open_files
            .iter()
            .map(|file| file.path.clone())
            .collect();
        for path in paths {
            self.refresh_line_blame(&path, false, cx);
        }
    }

    /// Paints the cursor line's blame over `editor`, if `path` has one.
    pub(super) fn render_line_blame(
        &self,
        path: &str,
        editor: &Entity<EditorState>,
        cx: &App,
    ) -> Option<AnyElement> {
        let blame = self.line_blame.get(path).filter(|blame| blame.fresh)?;
        let lines = blame.lines.clone();
        let editor = editor.clone();
        let color = cx.theme().muted_foreground.opacity(0.7);
        let family = cx.theme().mono_font_family.clone();
        let size = cx.theme().mono_font_size;
        // Painted after the editor, so its layout for this frame is known.
        Some(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    paint_line_blame(bounds, &editor, &lines, color, family, size, window, cx)
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .into_any_element(),
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_line_blame(
    bounds: Bounds<Pixels>,
    editor: &Entity<EditorState>,
    lines: &[BlameLine],
    color: gpui_kit::Hsla,
    family: SharedString,
    size: Pixels,
    window: &mut Window,
    cx: &mut App,
) {
    let state = editor.read(cx);
    if !state.focus_handle(cx).is_focused(window) {
        return;
    }
    let row = state.cursor_position().line as usize;
    let Some(line) = lines.get(row) else {
        return;
    };
    let end = state.text().line_end_offset(row);
    let Some(at) = state.range_to_bounds(&(end..end)) else {
        return;
    };
    let text: SharedString = label(line).into();
    let size = size * 0.9;
    let run = TextRun {
        len: text.len(),
        font: font(family),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window.text_system().shape_line(text, size, &[run], None);
    let origin = point(at.right() + px(32.), at.top());
    let line_height = at.size.height;
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        shaped
            .paint(origin, line_height, TextAlign::Left, None, window, cx)
            .ok();
    });
}

/// "Ada, 3 days ago • Fix the parser", as in Zed.
fn label(line: &BlameLine) -> String {
    if !line.is_committed() {
        return "You • Uncommitted changes".into();
    }
    format!(
        "{}, {} • {}",
        line.author,
        relative_time(line.time),
        line.summary
    )
}
