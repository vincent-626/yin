//! The branch picker: switch to a local or remote branch, or create one.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, IndexPath, WindowExt as _,
    command::{Command, CommandGroup, CommandItem},
};
use gpui_kit::{
    AppContext as _, Context, Focusable as _, ParentElement as _, Styled as _, Window, px,
};
use yin_git::BranchRef;

use super::Workspace;

/// What a picker row does when confirmed.
#[derive(Clone, Debug)]
enum Choice {
    Create(String),
    Switch(BranchRef),
}

/// The picker's groups for `query`, skipping empty ones. Rendering and
/// confirmation both derive from this, so index paths agree.
fn sections(branches: &[BranchRef], query: &str) -> Vec<(&'static str, Vec<Choice>)> {
    let query = query.trim();
    let mut sections = Vec::new();
    if !query.is_empty() && !branches.iter().any(|b| b.local_name() == query) {
        sections.push(("New", vec![Choice::Create(query.to_owned())]));
    }
    let (remote, local): (Vec<_>, Vec<_>) = branches.iter().cloned().partition(|b| b.is_remote);
    for (label, group) in [("Local", local), ("Remote", remote)] {
        if !group.is_empty() {
            sections.push((label, group.into_iter().map(Choice::Switch).collect()));
        }
    }
    sections
}

impl Workspace {
    pub(super) fn open_branch_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        cx.spawn_in(window, async move |this, cx| {
            let branches = cx.background_spawn(async move { repo.branches() }).await;
            this.update_in(cx, |this, window, cx| match branches {
                Ok(branches) => {
                    this.branches = branches;
                    this.show_branch_picker(window, cx);
                }
                Err(error) => window.push_notification(
                    gpui_kit::component::notification::Notification::error(error.to_string()),
                    cx,
                ),
            })
            .ok();
        })
        .detach();
    }

    fn show_branch_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picker = self.branch_picker.clone();
        picker.update(cx, |state, cx| state.set_query("", window, cx));
        let this = cx.weak_entity();
        let focus_on_open = Rc::new(Cell::new(true));
        window.open_dialog(cx, move |dialog, window, cx| {
            if focus_on_open.replace(false) {
                let picker = picker.clone();
                window.defer(cx, move |window, cx| {
                    picker.read(cx).focus_handle(cx).focus(window, cx);
                });
            }
            let query = picker.read(cx).query(cx).to_string();
            let sections = this
                .read_with(cx, |this, _| sections(&this.branches, &query))
                .unwrap_or_default();
            let groups = sections.into_iter().map(|(label, choices)| {
                CommandGroup::new()
                    .label(label)
                    .items(choices.into_iter().map(|choice| {
                        match choice {
                            Choice::Create(name) => CommandItem::new()
                                .label(format!("Create branch “{name}”"))
                                .icon(IconName::Plus)
                                .keywords([name]),
                            Choice::Switch(branch) => CommandItem::new()
                                .label(branch.name.clone())
                                .icon(Icon::empty().path("icons/git-branch.svg"))
                                .checked(branch.is_head)
                                .keywords([branch.subject.clone()]),
                        }
                    }))
            });
            let confirm = this.clone();
            let query_for_confirm = query.clone();
            let picker = picker.clone();
            dialog
                .w(px(520.))
                .p_0()
                .close_button(false)
                .content(move |content, _, cx| {
                    let mut command = Command::new(&picker)
                        .bordered(false)
                        .rounded(cx.theme().radius_lg)
                        .placeholder("Switch to or create a branch…")
                        .min_h(px(120.))
                        .max_h(px(360.));
                    for group in groups.clone() {
                        command = command.group(group);
                    }
                    let confirm = confirm.clone();
                    let query = query_for_confirm.clone();
                    content.child(command.on_confirm(move |index: IndexPath, window, cx| {
                        window.close_dialog(cx);
                        confirm
                            .update(cx, |this, cx| {
                                this.confirm_branch(&query, index, window, cx)
                            })
                            .ok();
                    }))
                })
        });
    }

    fn confirm_branch(
        &mut self,
        query: &str,
        index: IndexPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let choice = sections(&self.branches, query)
            .into_iter()
            .nth(index.section)
            .and_then(|(_, choices)| choices.into_iter().nth(index.row));
        match choice {
            Some(Choice::Create(name)) => {
                self.run(move |repo| repo.create_branch(&name), window, cx)
            }
            Some(Choice::Switch(branch)) if !branch.is_head => {
                self.run(move |repo| repo.switch(&branch), window, cx)
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn branch(name: &str, is_remote: bool, is_head: bool) -> BranchRef {
        BranchRef {
            name: name.into(),
            is_remote,
            is_head,
            upstream: None,
            time: 0,
            subject: String::new(),
        }
    }

    #[test]
    fn offers_create_only_for_new_names() {
        let branches = [
            branch("main", false, true),
            branch("origin/topic", true, false),
        ];
        let labels = |query| -> Vec<&str> {
            sections(&branches, query)
                .iter()
                .map(|(label, _)| *label)
                .collect()
        };
        assert_eq!(labels(""), ["Local", "Remote"]);
        assert_eq!(labels("feature"), ["New", "Local", "Remote"]);
        assert_eq!(labels("main"), ["Local", "Remote"]);
        // Creating "topic" would collide with the tracking branch switch offers.
        assert_eq!(labels("topic"), ["Local", "Remote"]);
    }
}
