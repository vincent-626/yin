//! The worktree picker: every worktree of this repository with its branch
//! and how many files it has changed, to switch the window to one.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme as _, IconName, IndexPath, WindowExt as _,
    command::{Command, CommandItem},
    notification::Notification,
};
use gpui_kit::{
    AppContext as _, Context, EventEmitter, Focusable as _, ParentElement as _, Styled as _,
    Window, px,
};
use yin_git::Repository;

use super::Workspace;

/// Asks the window to open another repository or worktree.
pub struct OpenRepository(pub PathBuf);

impl EventEmitter<OpenRepository> for Workspace {}

#[derive(Clone)]
struct Entry {
    path: PathBuf,
    label: String,
    is_current: bool,
}

impl Workspace {
    pub(super) fn open_worktree_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        cx.spawn_in(window, async move |this, cx| {
            let entries = cx
                .background_spawn(async move {
                    let current = repo.workdir().canonicalize()?;
                    let entries = repo
                        .worktrees()?
                        .into_iter()
                        .filter(|worktree| !worktree.is_bare)
                        .map(|worktree| {
                            let path = worktree.path.canonicalize().unwrap_or(worktree.path);
                            let changes = Repository::open(&path)
                                .and_then(|repo| repo.status())
                                .map(|status| status.entries.len());
                            let branch = worktree.branch.unwrap_or_else(|| {
                                let oid = worktree.oid.unwrap_or_default();
                                format!("detached at {}", &oid[..oid.len().min(7)])
                            });
                            let state = match changes {
                                Ok(0) => "clean".to_owned(),
                                Ok(1) => "1 change".to_owned(),
                                Ok(n) => format!("{n} changes"),
                                Err(_) => "missing".to_owned(),
                            };
                            Entry {
                                label: format!(
                                    "{branch} · {state} · {}",
                                    crate::state::tilde(&path)
                                ),
                                is_current: path == current,
                                path,
                            }
                        })
                        .collect::<Vec<_>>();
                    anyhow::Ok(entries)
                })
                .await;
            this.update_in(cx, |this, window, cx| match entries {
                Ok(entries) => this.show_worktree_picker(entries, window, cx),
                Err(error) => window.push_notification(Notification::error(error.to_string()), cx),
            })
            .ok();
        })
        .detach();
    }

    fn show_worktree_picker(
        &mut self,
        entries: Vec<Entry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let picker = self.worktree_picker.clone();
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
            let items: Vec<CommandItem> = entries
                .iter()
                .map(|entry| {
                    CommandItem::new()
                        .label(entry.label.clone())
                        .icon(IconName::FolderOpen)
                        .checked(entry.is_current)
                })
                .collect();
            let entries = entries.clone();
            let picker = picker.clone();
            let this = this.clone();
            dialog
                .w(px(720.))
                .p_0()
                .close_button(false)
                .content(move |content, _, cx| {
                    let entries = entries.clone();
                    let this = this.clone();
                    content.child(
                        Command::new(&picker)
                            .bordered(false)
                            .rounded(cx.theme().radius_lg)
                            .placeholder("Switch worktree…")
                            .max_h(px(360.))
                            .items(items.clone())
                            .on_confirm(move |index: IndexPath, window, cx| {
                                window.close_dialog(cx);
                                let Some(entry) = entries.get(index.row) else {
                                    return;
                                };
                                if entry.is_current {
                                    return;
                                }
                                let path = entry.path.clone();
                                this.update(cx, |_, cx| cx.emit(OpenRepository(path))).ok();
                            }),
                    )
                })
        });
    }
}
