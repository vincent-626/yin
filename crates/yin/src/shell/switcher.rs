//! The repository switcher: recent repositories first, then the ones found in
//! the usual code folders. Picking one brings up its window, opening it if
//! needed.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme as _, IconName, IndexPath, WindowExt as _,
    command::{Command, CommandItem},
    h_flex,
};
use gpui_kit::{
    AppContext as _, Context, Focusable as _, ParentElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};

use super::{Shell, repo_name};
use crate::{state, windows};

#[derive(Clone)]
pub struct Entry {
    path: PathBuf,
    name: String,
    location: String,
    /// The repository has a window already.
    is_open: bool,
}

impl Entry {
    fn new(path: PathBuf, open: &[PathBuf]) -> Self {
        Self {
            name: repo_name(&path),
            location: state::tilde(path.parent().unwrap_or(&path)),
            is_open: open.contains(&path),
            path,
        }
    }
}

/// How deep to look under each code folder: `~/Code/org/repo` is two down.
const SEARCH_DEPTH: usize = 3;

impl Shell {
    pub fn open_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let open = windows::open_repos(cx);
        let recent = state::recent_repos();
        self.switcher_entries = recent
            .iter()
            .map(|path| Entry::new(path.clone(), &open))
            .collect();
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async move {
                    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
                        return Vec::new();
                    };
                    let roots: Vec<PathBuf> = yin_fs::CODE_FOLDERS
                        .iter()
                        .map(|folder| home.join(folder))
                        .filter(|root| root.is_dir())
                        .collect();
                    yin_fs::find_repositories(&roots, SEARCH_DEPTH)
                })
                .await;
            this.update(cx, |this, cx| {
                let found = found
                    .into_iter()
                    .filter_map(|path| path.canonicalize().ok())
                    .filter(|path| !recent.contains(path))
                    .map(|path| Entry::new(path, &open));
                this.switcher_entries.extend(found);
                cx.notify();
            })
            .ok();
        })
        .detach();

        let picker = self.switcher.clone();
        picker.update(cx, |state, cx| state.set_query("", window, cx));
        let shell = cx.weak_entity();
        let focus_on_open = Rc::new(Cell::new(true));
        window.open_dialog(cx, move |dialog, window, cx| {
            if focus_on_open.replace(false) {
                let picker = picker.clone();
                window.defer(cx, move |window, cx| {
                    picker.read(cx).focus_handle(cx).focus(window, cx);
                });
            }
            let entries = shell
                .read_with(cx, |shell, _| shell.switcher_entries.clone())
                .unwrap_or_default();
            let muted = cx.theme().muted_foreground;
            let items: Vec<CommandItem> = entries
                .iter()
                .map(|entry| {
                    let (name, location, is_open) =
                        (entry.name.clone(), entry.location.clone(), entry.is_open);
                    CommandItem::new()
                        .label(entry.name.clone())
                        .keywords([entry.location.clone()])
                        .child(move |_, _| {
                            h_flex()
                                .w_full()
                                .min_w_0()
                                .gap_2()
                                .child(div().flex_none().child(name.clone()))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_xs()
                                        .text_color(muted)
                                        .child(location.clone()),
                                )
                                .when(is_open, |this| {
                                    this.child(
                                        div().flex_none().text_xs().text_color(muted).child("Open"),
                                    )
                                })
                        })
                })
                .chain([CommandItem::new()
                    .label("Open folder…")
                    .icon(IconName::FolderOpen)])
                .collect();
            let picker = picker.clone();
            let shell = shell.clone();
            dialog
                .w(px(640.))
                .p_0()
                .close_button(false)
                .content(move |content, _, cx| {
                    let entries = entries.clone();
                    let shell = shell.clone();
                    content.child(
                        Command::new(&picker)
                            .bordered(false)
                            .rounded(cx.theme().radius_lg)
                            .placeholder("Switch repository…")
                            .max_h(px(420.))
                            .items(items.clone())
                            .on_confirm(move |index: IndexPath, window, cx| {
                                window.close_dialog(cx);
                                match entries.get(index.row) {
                                    Some(entry) => windows::open(Some(entry.path.clone()), cx),
                                    None => {
                                        shell.update(cx, |shell, cx| shell.prompt_open(cx)).ok();
                                    }
                                }
                            }),
                    )
                })
        });
    }
}
