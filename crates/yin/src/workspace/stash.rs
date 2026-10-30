//! Stashing: push everything, pop or drop a stash, and its row in the list.

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariant, ButtonVariants as _},
    h_flex,
    list::ListItem,
};
use gpui_kit::{
    AnyElement, App, Context, IntoElement as _, ParentElement as _, Styled as _, WeakEntity,
    Window, div,
};
use yin_git::Stash;

use super::{Section, Workspace};

impl Workspace {
    pub(super) fn stash_push(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.status.entries.is_empty() {
            window.push_notification("Nothing to stash.", cx);
            return;
        }
        self.run(|repo| repo.stash_push(None), window, cx);
    }

    /// Pops the selected stash, or the latest when no stash is selected.
    pub(super) fn stash_pop(
        &mut self,
        stash: Option<Stash>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(stash) = stash
            .or_else(|| self.selected_stash())
            .or_else(|| self.stashes.first().cloned())
        else {
            window.push_notification("No stashes.", cx);
            return;
        };
        self.run(move |repo| repo.stash_pop(&stash), window, cx);
    }

    pub(super) fn selected_stash(&self) -> Option<Stash> {
        let key = self.selected_key()?;
        if key.section != Section::Stashes {
            return None;
        }
        self.stash_for(&key.path)
    }

    pub(super) fn stash_for(&self, reference: &str) -> Option<Stash> {
        self.stashes
            .iter()
            .find(|stash| stash.reference() == reference)
            .cloned()
    }

    pub(super) fn confirm_drop_stash(
        &mut self,
        stash: Stash,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let this = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let this = this.clone();
            let stash = stash.clone();
            alert
                .title("Drop this stash?")
                .description(format!("{} will be deleted.", stash.message))
                .confirm()
                .ok_text("Drop")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    let stash = stash.clone();
                    this.update(cx, |this, cx| {
                        this.run(move |repo| repo.stash_drop(&stash), window, cx)
                    })
                    .ok();
                    true
                })
        });
    }
}

pub(super) fn render_stash_row(
    ix: usize,
    stash: &Stash,
    is_selected: bool,
    focused: bool,
    owner: &WeakEntity<Workspace>,
    cx: &App,
) -> AnyElement {
    let select_owner = owner.clone();
    let pop_owner = owner.clone();
    let pop = stash.clone();
    ListItem::new(("stash", ix))
        .w_full()
        .h_7()
        .px_3()
        .selected(is_selected)
        .secondary_selected(is_selected && focused)
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .gap_2()
                .text_sm()
                .child(
                    Icon::new(IconName::Inbox)
                        .xsmall()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(stash.message.clone()),
                )
                .child(
                    Button::new(("pop-stash", ix))
                        .ghost()
                        .xsmall()
                        .label("Pop")
                        .on_click(move |_, window, cx| {
                            let pop = pop.clone();
                            pop_owner
                                .update(cx, |this, cx| this.stash_pop(Some(pop), window, cx))
                                .ok();
                        }),
                ),
        )
        .on_click(move |_, _, cx| {
            select_owner
                .update(cx, |this, cx| this.select(Some(ix), cx))
                .ok();
        })
        .into_any_element()
}
