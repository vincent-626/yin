//! The bar above the commit box, where every git action lives: the branch,
//! fetch, pull and push, and a menu with the rest.

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{DropdownMenu as _, PopupMenu},
};
use gpui_kit::{Anchor, Context, IntoElement, ParentElement as _, Styled as _, Window};

use super::{
    CHANGES_CONTEXT, Commit, Fetch, Pull, Push, StageAll, StashPop, StashPush, SwitchBranch,
    SwitchWorktree, UnstageAll, Workspace,
};

impl Workspace {
    /// The branch, or where a detached HEAD points.
    pub(super) fn head_label(&self) -> String {
        let branch = &self.status.branch;
        match (&branch.head, &branch.oid) {
            (Some(head), _) => head.clone(),
            (None, Some(oid)) => format!("detached at {}", &oid[..oid.len().min(7)]),
            (None, None) => "no commits".into(),
        }
    }

    pub(super) fn render_git_bar(&self, _: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focus = self.changes_focus.clone();
        h_flex()
            .flex_none()
            .w_full()
            .min_w_0()
            .gap_1()
            .child(
                h_flex().flex_1().min_w_0().child(
                    Button::new("git-bar-branch")
                        .ghost()
                        .xsmall()
                        .icon(Icon::empty().path("icons/git-branch.svg"))
                        .label(self.head_label())
                        .text_color(cx.theme().muted_foreground)
                        .tooltip_with_action("Switch branch", &SwitchBranch, Some(CHANGES_CONTEXT))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_branch_picker(window, cx)),
                        ),
                ),
            )
            .child(self.render_sync_buttons(cx))
            .child(
                Button::new("git-bar-more")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Ellipsis)
                    .tooltip("More git actions")
                    .dropdown_menu_with_anchor(Anchor::BottomRight, move |menu, _, _| {
                        more_menu(menu, focus.clone())
                    }),
            )
    }
}

fn more_menu(menu: PopupMenu, focus: gpui_kit::FocusHandle) -> PopupMenu {
    menu.action_context(focus)
        .menu("Stage all", Box::new(StageAll))
        .menu("Unstage all", Box::new(UnstageAll))
        .menu("Commit", Box::new(Commit))
        .separator()
        .menu("Stash everything", Box::new(StashPush))
        .menu("Pop a stash", Box::new(StashPop))
        .separator()
        .menu("Switch branch…", Box::new(SwitchBranch))
        .menu("Switch worktree…", Box::new(SwitchWorktree))
        .separator()
        .menu("Fetch", Box::new(Fetch))
        .menu("Pull", Box::new(Pull))
        .menu("Push", Box::new(Push))
}
