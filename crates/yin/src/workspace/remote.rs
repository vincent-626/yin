//! Fetch, pull and push. They run in their own task so a slow network never
//! blocks staging or committing.

use gpui_kit::component::{
    Disableable as _, IconName, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariant, ButtonVariants as _},
    h_flex,
    notification::Notification,
};
use gpui_kit::{AppContext as _, Context, IntoElement, ParentElement as _, Styled as _, Window};

use super::{Fetch, Pull, Push, Workspace};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RemoteOp {
    Fetch,
    Pull,
    Push,
    /// Rebase local commits onto a diverged upstream.
    PullRebase,
    /// Push over a remote that moved, unless it moved again since fetch.
    ForcePush,
}

impl RemoteOp {
    fn success(self, upstream: Option<&str>) -> Option<String> {
        let upstream = upstream.unwrap_or("the remote");
        match self {
            // The ahead/behind counts already show what fetch found.
            Self::Fetch => None,
            Self::Pull => Some(format!("Pulled from {upstream}")),
            Self::PullRebase => Some(format!("Rebased onto {upstream}")),
            Self::Push | Self::ForcePush => Some(format!("Pushed to {upstream}")),
        }
    }
}

impl Workspace {
    pub(super) fn run_remote(&mut self, op: RemoteOp, window: &mut Window, cx: &mut Context<Self>) {
        if self.remote_op.is_some() {
            return;
        }
        let repo = self.repo.clone();
        let branch = self.status.branch.clone();
        self.remote_op = Some(op);
        cx.notify();
        self._remote_task = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    match op {
                        RemoteOp::Fetch => repo.fetch(),
                        RemoteOp::Pull => repo.pull(),
                        RemoteOp::Push => repo.push(&branch),
                        RemoteOp::PullRebase => repo.pull_rebase(),
                        RemoteOp::ForcePush => repo.push_force_with_lease(),
                    }
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.remote_op = None;
                this.refresh(cx);
                match result {
                    Ok(()) => {
                        // Read the upstream after refresh lands; push may have set it.
                        let upstream = this.status.branch.upstream.clone();
                        if let Some(message) = op.success(upstream.as_deref()) {
                            window.push_notification(Notification::success(message), cx);
                        }
                    }
                    Err(error) => {
                        let text = error.to_string();
                        if op == RemoteOp::Pull && yin_git::is_diverged_error(&text) {
                            this.offer_rebase(window, cx);
                        } else if op == RemoteOp::Push && yin_git::is_rejected_push(&text) {
                            this.offer_force_push(window, cx);
                        } else {
                            window.push_notification(Notification::error(text), cx)
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        });
    }

    fn offer_rebase(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let branch = &self.status.branch;
        let description = format!(
            "You have {} commit{} the remote doesn't, and it has {} you don't. \
             Rebase yours on top of {}?",
            branch.ahead,
            if branch.ahead == 1 { "" } else { "s" },
            branch.behind,
            branch.upstream.as_deref().unwrap_or("the upstream"),
        );
        self.confirm_remote(
            "Branches have diverged",
            description,
            "Rebase",
            false,
            RemoteOp::PullRebase,
            window,
            cx,
        );
    }

    fn offer_force_push(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_remote(
            "Push was rejected",
            "The remote has commits your branch doesn't. Force pushing replaces them, \
             unless the remote changed again since your last fetch."
                .into(),
            "Force push",
            true,
            RemoteOp::ForcePush,
            window,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn confirm_remote(
        &mut self,
        title: &'static str,
        description: String,
        ok: &'static str,
        danger: bool,
        op: RemoteOp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let this = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let this = this.clone();
            let alert = alert
                .title(title)
                .description(description.clone())
                .confirm()
                .ok_text(ok);
            let alert = if danger {
                alert.ok_variant(ButtonVariant::Danger)
            } else {
                alert
            };
            alert.on_ok(move |_, window, cx| {
                this.update(cx, |this, cx| this.run_remote(op, window, cx))
                    .ok();
                true
            })
        });
    }

    pub(super) fn render_sync_buttons(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let branch = &self.status.branch;
        let running = self.remote_op;
        let has_head = branch.head.is_some();
        let count = |n: u32| if n > 0 { n.to_string() } else { String::new() };
        h_flex()
            .gap_1()
            .child(
                Button::new("fetch")
                    .ghost()
                    .xsmall()
                    .icon(IconName::RefreshCw)
                    .loading(running == Some(RemoteOp::Fetch))
                    .disabled(running.is_some())
                    .tooltip_with_action("Fetch", &Fetch, Some("Changes"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.run_remote(RemoteOp::Fetch, window, cx)
                    })),
            )
            .child(
                Button::new("pull")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ArrowDown)
                    .label(count(branch.behind))
                    .loading(running == Some(RemoteOp::Pull))
                    .disabled(running.is_some() || branch.upstream.is_none())
                    .tooltip_with_action("Pull (fast-forward only)", &Pull, Some("Changes"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.run_remote(RemoteOp::Pull, window, cx)
                    })),
            )
            .child(
                Button::new("push")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ArrowUp)
                    .label(count(branch.ahead))
                    .loading(running == Some(RemoteOp::Push))
                    .disabled(running.is_some() || !has_head)
                    .tooltip_with_action(
                        if branch.upstream.is_some() {
                            "Push"
                        } else {
                            "Publish branch"
                        },
                        &Push,
                        Some("Changes"),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.run_remote(RemoteOp::Push, window, cx)
                    })),
            )
    }
}
