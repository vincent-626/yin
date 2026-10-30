//! The file finder (cmd-p): fuzzy-match every file in the working tree.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme as _, IconName, IndexPath, WindowExt as _,
    command::{Command, CommandItem},
};
use gpui_kit::{Context, Focusable as _, ParentElement as _, Styled as _, Window, px};

use super::Workspace;

/// Results shown at once; a narrower query finds the rest.
const FINDER_LIMIT: usize = 100;

impl Workspace {
    pub(super) fn open_finder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.files_loaded {
            self.load_files(cx);
        }
        self.finder
            .update(cx, |state, cx| state.set_query("", window, cx));
        self.update_finder("", cx);

        let finder = self.finder.clone();
        let this = cx.weak_entity();
        let focus_on_open = Rc::new(Cell::new(true));
        window.open_dialog(cx, move |dialog, window, cx| {
            if focus_on_open.replace(false) {
                let finder = finder.clone();
                window.defer(cx, move |window, cx| {
                    finder.read(cx).focus_handle(cx).focus(window, cx);
                });
            }
            let results = this
                .read_with(cx, |this, _| this.finder_results.clone())
                .unwrap_or_default();
            let items: Vec<CommandItem> = results
                .iter()
                .map(|path| {
                    let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
                    let label = if dir.is_empty() {
                        name.to_owned()
                    } else {
                        format!("{name}    {dir}")
                    };
                    CommandItem::new().label(label).icon(IconName::File)
                })
                .collect();
            let finder = finder.clone();
            let query_owner = this.clone();
            let confirm_owner = this.clone();
            dialog
                .w(px(640.))
                .p_0()
                .close_button(false)
                .content(move |content, _, cx| {
                    let query_owner = query_owner.clone();
                    let confirm_owner = confirm_owner.clone();
                    let results = results.clone();
                    content.child(
                        Command::new(&finder)
                            .bordered(false)
                            .rounded(cx.theme().radius_lg)
                            .filterable(false)
                            .placeholder("Go to file…")
                            .min_h(px(120.))
                            .max_h(px(420.))
                            .items(items.clone())
                            .on_query(move |query, _, cx| {
                                query_owner
                                    .update(cx, |this, cx| this.update_finder(query, cx))
                                    .ok();
                            })
                            .on_confirm(move |index: IndexPath, window, cx| {
                                window.close_dialog(cx);
                                if let Some(path) = results.get(index.row) {
                                    confirm_owner
                                        .update(cx, |this, cx| {
                                            this.open_file(path, None, window, cx)
                                        })
                                        .ok();
                                }
                            }),
                    )
                })
        });
    }

    /// Recomputes the finder's results. Matching is fast enough to run inline:
    /// under a millisecond for a few thousand files.
    fn update_finder(&mut self, query: &str, cx: &mut Context<Self>) {
        self.finder_results = if query.trim().is_empty() {
            // Nothing typed yet: open files, then changed files.
            let mut paths: Vec<String> = self
                .open_files
                .iter()
                .map(|file| file.path.clone())
                .collect();
            for entry in &self.status.entries {
                if !paths.contains(&entry.path) {
                    paths.push(entry.path.clone());
                }
            }
            paths.truncate(FINDER_LIMIT);
            paths
        } else {
            yin_fs::match_paths(&self.files, query, FINDER_LIMIT)
                .into_iter()
                .map(|found| self.files[found.index].clone())
                .collect()
        };
        cx.notify();
    }
}
