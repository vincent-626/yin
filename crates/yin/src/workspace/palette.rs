//! The command palette (cmd-shift-p): every action by name, with its key.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::{IndexPath, command::Command, command::CommandItem};
use gpui_kit::{Action, px};

use super::*;

/// Each entry's label and a constructor for its action.
fn commands() -> Vec<(&'static str, fn() -> Box<dyn Action>)> {
    vec![
        ("Commit", || Box::new(Commit)),
        ("Write commit message", || Box::new(FocusCommitMessage)),
        ("Stage all", || Box::new(StageAll)),
        ("Unstage all", || Box::new(UnstageAll)),
        ("Stage or unstage file", || Box::new(ToggleStaged)),
        ("Discard file", || Box::new(Discard)),
        ("Review all changes / one file", || {
            Box::new(ToggleReviewAll)
        }),
        ("Mark file reviewed", || Box::new(ToggleReviewed)),
        ("Mark everything reviewed", || Box::new(MarkAllReviewed)),
        ("Hide or show reviewed files", || {
            Box::new(ToggleHideReviewed)
        }),
        ("Switch branch", || Box::new(SwitchBranch)),
        ("Switch worktree", || Box::new(SwitchWorktree)),
        ("Fetch", || Box::new(Fetch)),
        ("Pull", || Box::new(Pull)),
        ("Push", || Box::new(Push)),
        ("Stash all changes", || Box::new(StashPush)),
        ("Pop stash", || Box::new(StashPop)),
        ("Show changes", || Box::new(ShowChanges)),
        ("Show history", || Box::new(ShowHistory)),
        ("Show files", || Box::new(ShowFiles)),
        ("Search the project", || Box::new(ProjectSearch)),
        ("Go to file", || Box::new(FindFile)),
        ("Go to line", || Box::new(GoToLine)),
        ("Find references", || Box::new(FindReferences)),
        ("Save file", || Box::new(SaveFile)),
        ("Close tab", || Box::new(CloseFile)),
        ("Markdown preview / source", || Box::new(TogglePreview)),
        ("Unified / split diff", || Box::new(ToggleDiffMode)),
        ("Toggle sidebar", || Box::new(ToggleSidebar)),
        ("Refresh", || Box::new(Refresh)),
        ("Open repository", || Box::new(crate::shell::Open)),
        ("Quit", || Box::new(crate::shell::Quit)),
    ]
}

/// `⌘⇧P` style text for an action's binding in any context; the most
/// recently added (a user's keymap) wins.
fn key_hint(action: &dyn Action, cx: &App) -> Option<String> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let binding = keymap.bindings_for_action(action).last()?;
    Some(
        binding
            .keystrokes()
            .iter()
            .map(|keystroke| {
                keystroke
                    .display(gpui_kit::OperatingSystem::Mac)
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join(" "),
    )
}

impl Workspace {
    pub(super) fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Actions run where focus was, so they behave like their keys.
        let previous = window.focused(cx);
        let palette = self.palette.clone();
        palette.update(cx, |state, cx| state.set_query("", window, cx));
        let items: Vec<CommandItem> = commands()
            .iter()
            .map(|(label, action)| {
                let label = match key_hint(&*action(), cx) {
                    Some(key) => format!("{label}    {key}"),
                    None => (*label).to_owned(),
                };
                CommandItem::new().label(label)
            })
            .collect();
        let focus_on_open = Rc::new(Cell::new(true));
        window.open_dialog(cx, move |dialog, window, cx| {
            if focus_on_open.replace(false) {
                let palette = palette.clone();
                window.defer(cx, move |window, cx| {
                    palette.read(cx).focus_handle(cx).focus(window, cx);
                });
            }
            let palette = palette.clone();
            let previous = previous.clone();
            let items = items.clone();
            dialog
                .w(px(560.))
                .p_0()
                .close_button(false)
                .content(move |content, _, cx| {
                    let previous = previous.clone();
                    content.child(
                        Command::new(&palette)
                            .bordered(false)
                            .rounded(cx.theme().radius_lg)
                            .placeholder("Run a command…")
                            .max_h(px(420.))
                            .items(items.clone())
                            .on_confirm(move |index: IndexPath, window, cx| {
                                window.close_dialog(cx);
                                let Some((_, action)) = commands().into_iter().nth(index.row)
                                else {
                                    return;
                                };
                                let previous = previous.clone();
                                window.defer(cx, move |window, cx| {
                                    if let Some(handle) = previous {
                                        handle.focus(window, cx);
                                    }
                                    window.dispatch_action(action(), cx);
                                });
                            }),
                    )
                })
        });
    }
}
