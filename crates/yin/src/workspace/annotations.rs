//! What shows inside the diffs besides code: your review notes, and the
//! review threads of the branch's pull request.
//!
//! Notes are written in a box floating over the diff rather than inline: the
//! diff binds single letters (space, s, n), which would swallow typing in an
//! editor placed inside it.

use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    diff::{Diff, DiffAnnotation, DiffLinePosition, DiffLineRange, DiffSide, DiffState},
    h_flex,
    input::{InputEvent, Textarea, TextareaState},
    kbd::Kbd,
    v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, ClipboardItem, Context, ElementId, Entity, Focusable as _,
    IntoElement as _, ParentElement as _, Styled as _, Window, div, prelude::FluentBuilder as _,
};
use yin_git::{FilePatch, ReviewThread, Side};

use super::{Panel, Workspace};
use crate::notes::{self, Note};

/// The two diffs notes can be written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DiffPane {
    Changes,
    History,
}

/// A note being written.
pub(super) struct NoteDraft {
    pane: DiffPane,
    note: Note,
    input: Entity<TextareaState>,
}

/// What an annotation shows.
enum Item {
    Note(Note, NotePlace),
    Thread(ReviewThread),
}

#[derive(Clone, Copy)]
enum NotePlace {
    /// Below this line of the note's side.
    Line(usize),
    /// Under the file's header: its code is gone.
    File,
}

impl Workspace {
    fn diff_in(&self, pane: DiffPane) -> &Entity<DiffState> {
        match pane {
            DiffPane::Changes => &self.all_diff,
            DiffPane::History => &self.history_diff,
        }
    }

    /// The diff the main pane shows, if notes can be written in it.
    pub(super) fn active_pane(&self) -> Option<DiffPane> {
        match self.panel {
            Panel::Changes if !self.shows_one_file() => Some(DiffPane::Changes),
            Panel::History => Some(DiffPane::History),
            _ => None,
        }
    }

    /// Starts a note on the lines selected in the visible diff.
    pub(super) fn add_note_for_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = self.active_pane() else {
            return;
        };
        match self.diff_in(pane).read(cx).selected_lines() {
            Some(range) => self.start_note(&range, pane, window, cx),
            None => window.push_notification("Select lines in the diff first.", cx),
        }
    }

    pub(super) fn start_note(
        &mut self,
        range: &DiffLineRange,
        pane: DiffPane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let old_side = range.end_side() == DiffSide::Original;
        let (start, end) = if range.side() == range.end_side() {
            (
                range.start().min(range.end()),
                range.start().max(range.end()),
            )
        } else {
            (range.end(), range.end())
        };
        let path = range.path().to_string();
        let patch = match pane {
            DiffPane::Changes => &self.all_patch,
            DiffPane::History => &self.history_patch,
        };
        let side = if old_side { Side::Old } else { Side::New };
        let code = FilePatch::lines_in(patch, &path, side, start, end);
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("What should change here?")
                .auto_grow(2, 10)
        });
        cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter {
                secondary: true, ..
            } = event
            {
                this.save_note(window, cx);
            }
        })
        .detach();
        input.read(cx).focus_handle(cx).focus(window, cx);
        self.note_draft = Some(NoteDraft {
            pane,
            note: Note {
                id: 0,
                path,
                old_side,
                start,
                end,
                body: String::new(),
                code,
                on_commits: pane == DiffPane::History,
                outdated: false,
            },
            input,
        });
        self.diff_in(pane)
            .update(cx, |diff, cx| diff.set_selected_lines(None, cx));
        cx.notify();
    }

    fn save_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.note_draft.take() else {
            return;
        };
        let body = draft.input.read(cx).value().trim().to_owned();
        if !body.is_empty() {
            self.notes.add(Note { body, ..draft.note });
            self.place_history_notes();
        }
        self.focus_diff(window, cx);
        cx.notify();
    }

    fn cancel_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.note_draft = None;
        self.focus_diff(window, cx);
        cx.notify();
    }

    fn delete_note(&mut self, id: u64, cx: &mut Context<Self>) {
        self.notes.remove(id);
        self.place_history_notes();
        cx.notify();
    }

    /// Moves notes after the working tree's changes, or marks them outdated.
    pub(super) fn follow_notes(&mut self) {
        let hidden = self.hidden_paths();
        self.notes
            .follow(&self.all_patch, &|path| hidden.contains(path));
    }

    /// Where notes sit in the History diff: wherever their code shows there.
    pub(super) fn place_history_notes(&mut self) {
        self.history_note_places = self
            .notes
            .items()
            .iter()
            .filter_map(|note| Some((note.id, note.find_in(&self.history_patch)?.1)))
            .collect();
    }

    /// Where `note` shows in `pane`'s diff: on a line, with its file when its
    /// code is gone, or not at all.
    fn note_place(&self, note: &Note, pane: DiffPane) -> Option<NotePlace> {
        match pane {
            DiffPane::Changes if note.on_commits => None,
            DiffPane::Changes if note.outdated => Some(NotePlace::File),
            DiffPane::Changes => Some(NotePlace::Line(note.end)),
            DiffPane::History => self
                .history_note_places
                .iter()
                .find(|(id, _)| *id == note.id)
                .map(|(_, line)| NotePlace::Line(*line)),
        }
    }

    /// Copies every note, and the pull request's open review threads, as one
    /// prompt for an agent.
    pub(super) fn copy_review_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let open_threads = self
            .review_threads
            .iter()
            .filter(|t| !t.is_resolved)
            .count();
        let total = self.notes.len() + open_threads;
        if total == 0 {
            window.push_notification("No review notes yet. Select lines and press n.", cx);
            return;
        }
        let prompt = notes::prompt(self.notes.items(), &self.review_threads);
        cx.write_to_clipboard(ClipboardItem::new_string(prompt));
        window.push_notification(
            format!(
                "Copied {} as a prompt",
                super::history::count(total, "review comment")
            ),
            cx,
        );
    }

    pub(super) fn clear_notes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.notes.is_empty() {
            return;
        }
        let this = cx.entity().downgrade();
        let count = super::history::count(self.notes.len(), "note");
        window.open_alert_dialog(cx, move |alert, _, _| {
            let this = this.clone();
            alert
                .title(format!("Delete {count}?"))
                .description("Review notes are kept only on this Mac.")
                .confirm()
                .ok_text("Delete")
                .ok_variant(gpui_kit::component::button::ButtonVariant::Danger)
                .on_ok(move |_, _, cx| {
                    this.update(cx, |this, cx| {
                        this.notes.clear();
                        cx.notify();
                    })
                    .ok();
                    true
                })
        });
    }

    /// Adds notes (and, while comparing for the pull request, its review
    /// threads) to `diff`, and the + button that starts a note.
    pub(super) fn annotate(&self, diff: Diff, pane: DiffPane, cx: &mut Context<Self>) -> Diff {
        let mut items: Vec<(ElementId, Item)> = self
            .notes
            .items()
            .iter()
            .filter_map(|note| {
                let place = self.note_place(note, pane)?;
                Some((
                    ElementId::from(format!("note-{}", note.id)),
                    Item::Note(note.clone(), place),
                ))
            })
            .collect();
        if pane == DiffPane::History && self.shows_pull_request() {
            items.extend(self.review_threads.iter().enumerate().map(|(ix, thread)| {
                (
                    ElementId::from(format!("thread-{ix}")),
                    Item::Thread(thread.clone()),
                )
            }));
        }
        let annotations: Vec<DiffAnnotation> = items
            .iter()
            .map(|(id, item)| match item {
                Item::Note(note, NotePlace::Line(line)) => DiffAnnotation::line(
                    id.clone(),
                    DiffLinePosition::new(note.path.clone(), side(note.old_side), *line),
                ),
                Item::Note(note, NotePlace::File) => {
                    DiffAnnotation::file(id.clone(), note.path.clone())
                }
                Item::Thread(thread) => match thread.line {
                    Some(line) => DiffAnnotation::line(
                        id.clone(),
                        DiffLinePosition::new(
                            thread.path.clone(),
                            side(thread.side == Side::Old),
                            line,
                        ),
                    ),
                    // Its code changed since; show it with the file.
                    None => DiffAnnotation::file(id.clone(), thread.path.clone()),
                },
            })
            .collect();
        let items = Rc::new(items);
        let owner = cx.entity().downgrade();
        let add_owner = owner.clone();
        diff.annotations(annotations)
            .render_annotation(move |annotation, _, cx| {
                let Some((_, item)) = items.iter().find(|(id, _)| id == annotation.id()) else {
                    return div().into_any_element();
                };
                match item {
                    Item::Note(note, _) => render_note(note, &owner, cx),
                    Item::Thread(thread) => render_thread(thread, cx),
                }
            })
            .on_add_annotation(move |range, window, cx| {
                add_owner
                    .update(cx, |this, cx| this.start_note(range, pane, window, cx))
                    .ok();
            })
    }

    /// The note editor, floating over `pane`'s diff while a note is written.
    pub(super) fn render_note_draft(
        &self,
        pane: DiffPane,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let draft = self
            .note_draft
            .as_ref()
            .filter(|draft| draft.pane == pane)?;
        Some(
            v_flex()
                .absolute()
                .bottom_3()
                .left_4()
                .right_4()
                .p_3()
                .gap_2()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().border)
                .shadow_lg()
                .bg(cx.theme().popover)
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("Note on {}", draft.note.location())),
                )
                .child(Textarea::new(&draft.input).small())
                .child(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("Kept on this Mac. Copy notes as a prompt for an agent."),
                        )
                        .child(
                            Button::new("cancel-note")
                                .ghost()
                                .small()
                                .label("Cancel")
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.cancel_note(window, cx)),
                                ),
                        )
                        .child(
                            Button::new("save-note")
                                .primary()
                                .small()
                                .label("Add note")
                                .children(Kbd::binding_for_action(
                                    &super::Commit,
                                    Some("Workspace"),
                                    window,
                                ))
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.save_note(window, cx)),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }

    /// The count of notes and open threads, with what to do with them.
    pub(super) fn render_notes_strip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let open_threads = if self.pull_request.is_some() {
            self.review_threads
                .iter()
                .filter(|t| !t.is_resolved)
                .count()
        } else {
            0
        };
        if self.notes.is_empty() && open_threads == 0 {
            return None;
        }
        let mut parts = Vec::new();
        if !self.notes.is_empty() {
            let mut notes = super::history::count(self.notes.len(), "note");
            let outdated = self.notes.outdated();
            if outdated > 0 {
                notes.push_str(&format!(" ({outdated} outdated)"));
            }
            parts.push(notes);
        }
        if open_threads > 0 {
            parts.push(super::history::count(open_threads, "open PR thread"));
        }
        Some(
            h_flex()
                .flex_none()
                .px_3()
                .py_1()
                .gap_1()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .border_t_1()
                .border_color(cx.theme().border)
                .child(Icon::empty().path("icons/message-square-text.svg").xsmall())
                .child(div().flex_1().min_w_0().truncate().child(parts.join(" · ")))
                .child(
                    Button::new("copy-prompt")
                        .ghost()
                        .xsmall()
                        .label("Copy as prompt")
                        .tooltip_with_action(
                            "Copy notes and open threads as a prompt for an agent",
                            &super::CopyReviewPrompt,
                            Some("Workspace"),
                        )
                        .on_click(
                            cx.listener(|this, _, window, cx| this.copy_review_prompt(window, cx)),
                        ),
                )
                .when(!self.notes.is_empty(), |this| {
                    this.child(
                        Button::new("clear-notes")
                            .ghost()
                            .xsmall()
                            .icon(Icon::empty().path("icons/trash.svg"))
                            .tooltip("Delete all notes")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.clear_notes(window, cx)),
                            ),
                    )
                })
                .into_any_element(),
        )
    }
}

fn side(old: bool) -> DiffSide {
    if old {
        DiffSide::Original
    } else {
        DiffSide::Modified
    }
}

/// An annotation's card.
fn card(cx: &gpui_kit::App) -> gpui_kit::Div {
    v_flex()
        .w_full()
        .my_1()
        .px_3()
        .py_2()
        .gap_1()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().popover)
}

fn render_note(
    note: &Note,
    owner: &gpui_kit::WeakEntity<Workspace>,
    cx: &mut gpui_kit::App,
) -> AnyElement {
    let (owner, id) = (owner.clone(), note.id);
    let muted = cx.theme().muted_foreground;
    card(cx)
        .child(
            h_flex()
                .gap_2()
                .text_xs()
                .text_color(muted)
                .child(Icon::empty().path("icons/square-pen.svg").xsmall())
                .child(div().child("Your note"))
                .when(note.outdated, |this| {
                    this.child(
                        div()
                            .text_color(cx.theme().warning)
                            .child(format!("Outdated · was line {}", note.end)),
                    )
                })
                .child(div().flex_1())
                .child(
                    Button::new(ElementId::from(format!("delete-note-{id}")))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .tooltip("Delete note")
                        .on_click(move |_, _, cx| {
                            owner.update(cx, |this, cx| this.delete_note(id, cx)).ok();
                        }),
                ),
        )
        // An outdated note quotes what it was about, since the diff no
        // longer shows it.
        .when(note.outdated && !note.code.is_empty(), |this| {
            this.child(
                div()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .bg(cx.theme().secondary)
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_xs()
                    .text_color(muted)
                    .children(note.code.iter().map(|line| div().child(line.clone()))),
            )
        })
        .child(div().text_sm().child(note.body.clone()))
        .into_any_element()
}

fn render_thread(thread: &ReviewThread, cx: &mut gpui_kit::App) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let status = match (thread.is_resolved, thread.line) {
        (true, _) => Some("Resolved"),
        (false, None) => Some("Outdated"),
        _ => None,
    };
    let url = thread.comments.first().map(|comment| comment.url.clone());
    card(cx)
        .when(thread.is_resolved, |this| this.opacity(0.6))
        .child(
            h_flex()
                .gap_2()
                .text_xs()
                .text_color(muted)
                .child(Icon::empty().path("icons/github.svg").xsmall())
                .child(div().child("Pull request review"))
                .when_some(status, |this, status| this.child(div().child(status)))
                .when(thread.line.is_none(), |this| {
                    this.when_some(thread.original_line, |this, line| {
                        this.child(div().child(format!("was line {line}")))
                    })
                })
                .child(div().flex_1())
                .when_some(url, |this, url| {
                    this.child(
                        Button::new(ElementId::from(format!("open-thread-{url}")))
                            .ghost()
                            .xsmall()
                            .icon(Icon::empty().path("icons/external-link.svg"))
                            .tooltip("Open on GitHub")
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    )
                }),
        )
        .children(thread.comments.iter().map(|comment| {
            v_flex()
                .gap_0p5()
                .child(
                    div()
                        .text_xs()
                        .font_semibold()
                        .child(comment.author.clone()),
                )
                .child(div().text_sm().child(comment.body.clone()))
        }))
        .into_any_element()
}
