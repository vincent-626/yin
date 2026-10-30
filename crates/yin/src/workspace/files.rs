//! The Files tab: the working tree as folders, and open files in tabs, shown
//! read-only with highlighting, or rendered when they are markdown.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariant, ButtonVariants as _},
    h_flex,
    input::{Editor, EditorState, Input, InputEvent, Position},
    list::ListItem,
    notification::Notification,
    tab::{Tab, TabBar},
    text::{TextView, TextViewState},
    tree::{TreeEvent, TreeItem, TreeState, tree},
    v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, Focusable as _, Hsla, InteractiveElement as _,
    IntoElement, ParentElement as _, ScrollStrategy, SharedString, Styled as _, Subscription,
    Window, div, prelude::FluentBuilder as _, px,
};
use yin_git::{Change, FileStatus};

use super::{FILES_CONTEXT, Workspace};

/// Larger files are not opened; reading code doesn't need them.
const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;

pub(super) struct OpenFile {
    pub(super) path: String,
    content: FileContent,
    /// Rendered markdown instead of its source.
    preview: bool,
    /// The editor differs from what was last read or saved.
    dirty: bool,
    /// The file changed on disk while it had unsaved edits.
    changed_on_disk: bool,
}

impl OpenFile {
    /// The path and editor, once the file has loaded as text.
    pub(super) fn editor(&self) -> Option<(&str, &Entity<EditorState>)> {
        match &self.content {
            FileContent::Text { editor, .. } => Some((&self.path, editor)),
            _ => None,
        }
    }
}

enum FileContent {
    Loading,
    Text {
        /// What was last read from or written to disk.
        text: String,
        editor: Entity<EditorState>,
        markdown: Option<Entity<TextViewState>>,
        _edits: Subscription,
    },
    /// Binary, too large, or unreadable; the reason is shown instead.
    Unavailable(String),
}

fn is_markdown(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown") || lower.ends_with(".mdx")
}

/// The highlighter language for a path, by file name or extension.
pub(super) fn language_for(path: &str) -> &'static str {
    let name = path.rsplit('/').next().unwrap_or(path).to_lowercase();
    if matches!(name.as_str(), "makefile" | "gnumakefile") {
        return "make";
    }
    if name == "cmakelists.txt" {
        return "cmake";
    }
    match name.rsplit_once('.').map(|(_, extension)| extension) {
        Some("rs") => "rust",
        Some("py" | "pyi") => "python",
        Some("js" | "jsx" | "mjs" | "cjs") => "javascript",
        Some("ts" | "mts" | "cts") => "typescript",
        Some("tsx") => "tsx",
        Some("go") => "go",
        Some("sh" | "bash" | "zsh") => "bash",
        Some("yaml" | "yml") => "yaml",
        Some("toml") => "toml",
        Some("json" | "jsonc") => "json",
        Some("md" | "markdown" | "mdx") => "markdown",
        Some("css" | "scss") => "css",
        Some("html" | "htm") => "html",
        Some("c" | "h") => "c",
        Some("cc" | "cpp" | "cxx" | "hpp" | "hxx") => "cpp",
        Some("rb") => "ruby",
        Some("java") => "java",
        Some("swift") => "swift",
        Some("kt" | "kts") => "kotlin",
        Some("lua") => "lua",
        Some("sql") => "sql",
        Some("zig") => "zig",
        Some("proto") => "proto",
        Some("graphql" | "gql") => "graphql",
        Some("scala") => "scala",
        Some("php") => "php",
        Some("diff" | "patch") => "diff",
        _ => "text",
    }
}

/// Reads a file for display, or says why it can't be shown.
fn read_for_display(full: &std::path::Path) -> Result<String, String> {
    let bytes = std::fs::read(full).map_err(|error| error.to_string())?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(format!(
            "This file is {:.1} MB, too large to show.",
            bytes.len() as f64 / 1_048_576.0
        ));
    }
    if bytes[..bytes.len().min(8000)].contains(&0) {
        return Err("This is a binary file.".into());
    }
    String::from_utf8(bytes).map_err(|_| "This file isn't UTF-8 text.".into())
}

/// `"42"` or `"42:7"`, one-based.
fn parse_line_target(text: &str) -> Option<(u32, u32)> {
    let mut parts = text.trim().splitn(2, ':');
    let line = parts.next()?.trim().parse().ok().filter(|line| *line > 0)?;
    let column = match parts.next() {
        Some(column) => column.trim().parse().ok().filter(|column| *column > 0)?,
        None => 1,
    };
    Some((line, column))
}

fn tree_items(nodes: &[yin_fs::Node], expanded: &HashSet<String>) -> Vec<TreeItem> {
    nodes
        .iter()
        .map(|node| {
            let item = TreeItem::new(node.path.clone(), node.name.clone());
            if node.is_dir {
                item.expanded(expanded.contains(&node.path))
                    .children(tree_items(&node.children, expanded))
            } else {
                item
            }
        })
        .collect()
}

/// Git state worth showing in the tree: changed files and their folders.
#[derive(Default)]
pub(super) struct TreeDecorations {
    files: HashMap<String, Change>,
    dirs: HashSet<String>,
}

impl TreeDecorations {
    fn from_status(status: &yin_git::Status) -> Self {
        let mut decorations = Self::default();
        for entry in &status.entries {
            let change = match entry.status {
                FileStatus::Untracked => Change::Added,
                FileStatus::Conflicted => Change::Modified,
                FileStatus::Tracked { index, worktree } => {
                    if worktree.is_changed() {
                        worktree
                    } else {
                        index
                    }
                }
            };
            decorations.files.insert(entry.path.clone(), change);
            let mut path = entry.path.as_str();
            while let Some((parent, _)) = path.rsplit_once('/') {
                if !decorations.dirs.insert(parent.to_owned()) {
                    break;
                }
                path = parent;
            }
        }
        decorations
    }
}

impl Workspace {
    pub(super) fn new_file_tree(cx: &mut Context<Self>) -> Entity<TreeState> {
        let tree = cx.new(|cx| TreeState::new(cx));
        cx.subscribe(&tree, |this, _, event: &TreeEvent, _| match event {
            TreeEvent::Expanded(path) => {
                this.expanded_dirs.insert(path.to_string());
            }
            TreeEvent::Collapsed(path) => {
                this.expanded_dirs.remove(path.as_ref());
            }
        })
        .detach();
        tree
    }

    /// Rescans the working tree; the tree only rebuilds when files changed.
    pub(super) fn load_files(&mut self, cx: &mut Context<Self>) {
        let root = self.repo.workdir().to_owned();
        self._files_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { yin_fs::scan(&root) })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(files) if !this.files_loaded || *this.files != files => {
                        this.files = Arc::new(files);
                        this.files_loaded = true;
                        this.rebuild_tree(cx);
                    }
                    Ok(_) => {}
                    Err(error) => this.error = Some(error.to_string().into()),
                }
                cx.notify();
            })
            .ok();
        });
    }

    fn rebuild_tree(&mut self, cx: &mut Context<Self>) {
        let items = tree_items(&yin_fs::build_tree(&self.files), &self.expanded_dirs);
        let selected: Option<SharedString> = self
            .file_tree
            .read(cx)
            .selected_item()
            .map(|item| item.id.clone());
        self.file_tree.update(cx, |tree, cx| {
            tree.set_items(items, cx);
            if let Some(id) = selected
                && let Some(ix) = tree.index_of(&id)
            {
                tree.set_selected_index(Some(ix), cx);
            }
        });
    }

    pub(super) fn update_tree_decorations(&mut self) {
        self.tree_decorations = Arc::new(TreeDecorations::from_status(&self.status));
    }

    /// Opens `path` in a tab, or switches to it, optionally at a one-based line.
    pub(super) fn open_file(
        &mut self,
        path: &str,
        line: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Results open beside the search; anything else shows the tree.
        if self.panel != super::Panel::Search {
            self.show_panel(super::Panel::Files, window, cx);
        }
        self.file_tree.update(cx, |tree, cx| {
            let id = SharedString::from(path.to_owned());
            tree.reveal_item(&id, ScrollStrategy::Center, cx);
            if let Some(ix) = tree.index_of(&id) {
                tree.set_selected_index(Some(ix), cx);
            }
        });
        if let Some(ix) = self.open_files.iter().position(|file| file.path == path) {
            self.active_file = Some(ix);
            match line {
                Some(line) => self.go_to_line(ix, line, window, cx),
                None => self.focus_active_file(window, cx),
            }
            cx.notify();
            return;
        }
        self.open_files.push(OpenFile {
            path: path.to_owned(),
            content: FileContent::Loading,
            preview: is_markdown(path),
            dirty: false,
            changed_on_disk: false,
        });
        let ix = self.open_files.len() - 1;
        self.active_file = Some(ix);
        self.load_file(path.to_owned(), line, window, cx);
        cx.notify();
    }

    fn load_file(
        &mut self,
        path: String,
        line: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let full = self.repo.workdir().join(&path);
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { read_for_display(&full) })
                .await;
            this.update_in(cx, |this, window, cx| {
                let Some(ix) = this.open_files.iter().position(|file| file.path == path) else {
                    return;
                };
                this.install_file(ix, result, window, cx);
                match line {
                    Some(line) => this.go_to_line(ix, line, window, cx),
                    None if this.active_file == Some(ix) => this.focus_active_file(window, cx),
                    None => {}
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn install_file(
        &mut self,
        ix: usize,
        result: Result<String, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = self.open_files[ix].path.clone();
        let text = match result {
            Ok(text) => text,
            Err(reason) => {
                self.open_files[ix].content = FileContent::Unavailable(reason);
                return;
            }
        };
        let mut created = None;
        let file = &mut self.open_files[ix];
        match &mut file.content {
            FileContent::Text {
                text: current,
                editor,
                markdown,
                ..
            } => {
                if *current == text {
                    return;
                }
                // Never overwrite unsaved edits; offer a reload instead.
                if file.dirty {
                    file.changed_on_disk = true;
                    return;
                }
                editor.update(cx, |editor, cx| editor.set_value(text.clone(), window, cx));
                if let Some(markdown) = markdown {
                    markdown.update(cx, |markdown, cx| markdown.set_text(&text, cx));
                }
                *current = text;
            }
            content => {
                let editor = cx.new(|cx| {
                    EditorState::new(window, cx)
                        .language(language_for(&path))
                        .line_number(true)
                        .searchable(true)
                        .soft_wrap(false)
                        .default_value(text.clone())
                });
                let markdown = is_markdown(&path)
                    .then(|| cx.new(|cx| TextViewState::markdown(&text, cx).selectable(true)));
                created = Some(editor.clone());
                let edited_path = path.clone();
                let edits = cx.subscribe(&editor, move |this, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.file_edited(&edited_path, cx);
                    }
                });
                *content = FileContent::Text {
                    text,
                    editor,
                    markdown,
                    _edits: edits,
                };
            }
        }
        if let Some(editor) = created {
            self.attach_language_server(&path, language_for(&path), &editor, window, cx);
        }
    }

    /// Paths of open files with unsaved edits.
    pub fn unsaved_files(&self) -> Vec<String> {
        self.open_files
            .iter()
            .filter(|file| file.dirty)
            .map(|file| file.path.clone())
            .collect()
    }

    fn file_edited(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(file) = self.open_files.iter_mut().find(|file| file.path == path) else {
            return;
        };
        if let FileContent::Text {
            text,
            editor,
            markdown,
            ..
        } = &file.content
        {
            let value = editor.read(cx).value();
            file.dirty = value.as_ref() != text.as_str();
            if let Some(markdown) = markdown {
                markdown.update(cx, |markdown, cx| markdown.set_text(&value, cx));
            }
            self.sync_edited_file(path, &value);
            cx.notify();
        }
    }

    pub(super) fn save_active_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(file) = self.active_file.and_then(|ix| self.open_files.get(ix)) else {
            return;
        };
        let FileContent::Text { editor, .. } = &file.content else {
            return;
        };
        let value = editor.read(cx).value().to_string();
        let path = file.path.clone();
        let full = self.repo.workdir().join(&path);
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn({
                    let value = value.clone();
                    async move { std::fs::write(&full, value) }
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(()) => {
                        this.notify_saved(&path);
                        if let Some(file) = this.open_files.iter_mut().find(|f| f.path == path)
                            && let FileContent::Text { text, editor, .. } = &mut file.content
                        {
                            file.dirty = editor.read(cx).value().as_ref() != value;
                            file.changed_on_disk = false;
                            *text = value;
                        }
                    }
                    Err(error) => window.push_notification(
                        Notification::error(format!("Couldn't save {path}: {error}")),
                        cx,
                    ),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Replaces unsaved edits with the file as it is on disk.
    fn reload_from_disk(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(file) = self.open_files.get_mut(ix) else {
            return;
        };
        file.dirty = false;
        file.changed_on_disk = false;
        if let FileContent::Text { text, .. } = &mut file.content {
            // Forget the saved text so the next read always applies.
            text.clear();
        }
        let path = file.path.clone();
        self.load_file(path, None, window, cx);
    }

    /// Picks up changes on disk to open files, such as an agent's edits.
    pub(super) fn reload_open_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for file in &self.open_files {
            let path = file.path.clone();
            let full = self.repo.workdir().join(&path);
            cx.spawn_in(window, async move |this, cx| {
                let result = cx
                    .background_spawn(async move { read_for_display(&full) })
                    .await;
                this.update_in(cx, |this, window, cx| {
                    if let Some(ix) = this.open_files.iter().position(|file| file.path == path) {
                        this.install_file(ix, result, window, cx);
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
    }

    /// Asks for a line, or `line:column`, and moves the cursor there.
    pub(super) fn prompt_go_to_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.active_file else {
            return;
        };
        if !matches!(
            self.open_files.get(ix).map(|file| &file.content),
            Some(FileContent::Text { .. })
        ) {
            return;
        }
        let input = self.goto_input.clone();
        input.update(cx, |input, cx| input.set_value("", window, cx));
        let this = cx.entity().downgrade();
        // The dialog takes focus as it opens; hand it to the input afterwards.
        let focus_on_open = std::rc::Rc::new(std::cell::Cell::new(true));
        window.open_alert_dialog(cx, move |alert, window, cx| {
            if focus_on_open.replace(false) {
                let input = input.clone();
                window.defer(cx, move |window, cx| {
                    input.update(cx, |input, cx| input.focus(window, cx));
                });
            }
            let input = input.clone();
            let this = this.clone();
            alert
                .title("Go to line")
                .child(Input::new(&input))
                .on_ok(move |_, window, cx| {
                    let Some((line, column)) = parse_line_target(&input.read(cx).value()) else {
                        return false;
                    };
                    this.update(cx, |this, cx| {
                        this.go_to_position(ix, line, column, window, cx)
                    })
                    .ok();
                    true
                })
        });
    }

    fn go_to_line(&mut self, ix: usize, line: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.go_to_position(ix, line, 1, window, cx);
    }

    /// Moves the cursor to a one-based line and column.
    fn go_to_position(
        &mut self,
        ix: usize,
        line: u32,
        column: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(file) = self.open_files.get_mut(ix) else {
            return;
        };
        if let FileContent::Text { editor, .. } = &file.content {
            // A line target means reading source, even in markdown.
            file.preview = false;
            let editor = editor.clone();
            editor.update(cx, |editor, cx| {
                editor.set_cursor_position(
                    Position::new(line.saturating_sub(1), column.saturating_sub(1)),
                    window,
                    cx,
                );
            });
            editor.read(cx).focus_handle(cx).focus(window, cx);
        }
    }

    /// Closes a tab, asking first when it has unsaved edits.
    pub(super) fn close_file(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(file) = self.open_files.get(ix) else {
            return;
        };
        if !file.dirty {
            return self.remove_file(ix, cx);
        }
        let path = file.path.clone();
        let this = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let this = this.clone();
            let path = path.clone();
            alert
                .title("Discard unsaved changes?")
                .description(format!("Your edits to {path} will be lost."))
                .confirm()
                .ok_text("Discard")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, _, cx| {
                    this.update(cx, |this, cx| {
                        if let Some(ix) = this.open_files.iter().position(|f| f.path == path) {
                            this.remove_file(ix, cx);
                        }
                    })
                    .ok();
                    true
                })
        });
    }

    fn remove_file(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.open_files.len() {
            return;
        }
        self.close_in_language_server(&self.open_files[ix].path);
        self.open_files.remove(ix);
        self.active_file = match self.active_file {
            _ if self.open_files.is_empty() => None,
            Some(active) if active > ix => Some(active - 1),
            Some(active) => Some(active.min(self.open_files.len() - 1)),
            None => None,
        };
        cx.notify();
    }

    /// Moves to the next or previous open tab, wrapping around.
    pub(super) fn step_open_file(&mut self, direction: isize, cx: &mut Context<Self>) {
        let count = self.open_files.len();
        if count == 0 {
            return;
        }
        let current = self.active_file.unwrap_or(0) as isize;
        self.active_file = Some((current + direction).rem_euclid(count as isize) as usize);
        cx.notify();
    }

    pub(super) fn toggle_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(file) = self.active_file.and_then(|ix| self.open_files.get_mut(ix))
            && is_markdown(&file.path)
        {
            file.preview = !file.preview;
            self.focus_active_file(window, cx);
            cx.notify();
        }
    }

    /// Focuses what the active tab shows: its editor, or for a markdown
    /// preview or unreadable file the pane itself. Focusing a hidden editor
    /// would leave keys with nowhere to go.
    pub(super) fn focus_active_file(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(file) = self.active_file.and_then(|ix| self.open_files.get(ix)) else {
            return;
        };
        match &file.content {
            FileContent::Text { editor, .. } if !file.preview => {
                editor.read(cx).focus_handle(cx).focus(window, cx)
            }
            _ => self.file_pane_focus.focus(window, cx),
        }
    }

    pub(super) fn open_selected_tree_item(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.file_tree.read(cx).selected_entry() else {
            return;
        };
        if entry.is_folder() {
            return;
        }
        let path = entry.item().id.to_string();
        self.open_file(&path, None, window, cx);
    }

    pub(super) fn render_file_tree(&self, cx: &mut Context<Self>) -> AnyElement {
        let owner = cx.entity().downgrade();
        let decorations = self.tree_decorations.clone();
        let theme = cx.theme().clone();
        let color = move |change: Change| -> Hsla {
            match change {
                Change::Added | Change::Copied => theme.green,
                Change::Deleted => theme.red,
                Change::Renamed => theme.blue,
                _ => theme.yellow,
            }
        };
        let muted = cx.theme().muted_foreground;
        let dir_color = cx.theme().yellow.opacity(0.75);
        let error_color = cx.theme().red;
        let error_files = Arc::new(self.diagnostics.error_files.clone());
        let error_dirs = Arc::new(self.diagnostics.error_dirs.clone());
        div()
            .id("file-tree")
            .key_context(FILES_CONTEXT)
            .flex_1()
            .min_h_0()
            .child(
                tree(&self.file_tree, move |ix, entry, _selected, _, _| {
                    let item = entry.item();
                    let path = item.id.to_string();
                    let (icon, text_color) = if entry.is_folder() {
                        let icon = if entry.is_expanded() {
                            IconName::FolderOpen
                        } else {
                            IconName::FolderClosed
                        };
                        let tint = if error_dirs.contains(&path) {
                            Some(error_color)
                        } else {
                            decorations.dirs.contains(&path).then_some(dir_color)
                        };
                        (icon, tint)
                    } else if error_files.contains(&path) {
                        (IconName::File, Some(error_color))
                    } else {
                        (
                            IconName::File,
                            decorations.files.get(&path).copied().map(&color),
                        )
                    };
                    let owner = owner.clone();
                    ListItem::new(ix)
                        .w_full()
                        .h_6()
                        .pl(px(12.) * entry.depth() + px(8.))
                        .pr_2()
                        .child(
                            h_flex()
                                .gap_1p5()
                                .text_sm()
                                .min_w_0()
                                .child(Icon::new(icon).xsmall().text_color(muted))
                                .child(
                                    div()
                                        .truncate()
                                        .when_some(text_color, |this, color| this.text_color(color))
                                        .child(item.label.clone()),
                                ),
                        )
                        .on_click(move |event, window, cx| {
                            if !event.standard_click() {
                                return;
                            }
                            let path = path.clone();
                            owner
                                .update(cx, |this, cx| {
                                    if this.files.binary_search(&path).is_ok() {
                                        this.open_file(&path, None, window, cx);
                                    }
                                })
                                .ok();
                        })
                })
                .size_full(),
            )
            .into_any_element()
    }

    pub(super) fn render_open_files(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(active) = self.active_file.filter(|ix| *ix < self.open_files.len()) else {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_1()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("No open files")
                .child(div().text_xs().child("Pick one in the tree, or press ⌘P"))
                .into_any_element();
        };
        let file = &self.open_files[active];
        let markdown = is_markdown(&file.path);
        let content = match &file.content {
            FileContent::Loading => div().into_any_element(),
            FileContent::Unavailable(reason) => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(reason.clone())
                .into_any_element(),
            FileContent::Text {
                markdown: Some(markdown),
                ..
            } if file.preview => div()
                .id("markdown-preview")
                .size_full()
                .overflow_hidden()
                .child(TextView::new(markdown).size_full().scrollable(true).p_6())
                .into_any_element(),
            FileContent::Text { editor, .. } => Editor::new(editor)
                // Outside the repository: toolchain sources reached by go to definition.
                .readonly(std::path::Path::new(&file.path).is_absolute())
                .bordered(false)
                .size_full()
                .font_family(cx.theme().mono_font_family.clone())
                .text_size(cx.theme().mono_font_size)
                .into_any_element(),
        };
        v_flex()
            .id("file-pane")
            .track_focus(&self.file_pane_focus)
            .size_full()
            .min_w_0()
            .child(
                h_flex()
                    .flex_none()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        TabBar::new("open-files")
                            .flex_1()
                            .min_w_0()
                            .selected_index(active)
                            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                                this.active_file = Some(*ix);
                                cx.notify();
                            }))
                            .children(self.open_files.iter().enumerate().map(|(ix, file)| {
                                let name = file.path.rsplit('/').next().unwrap_or(&file.path);
                                let label = if file.dirty {
                                    format!("● {name}")
                                } else {
                                    name.to_owned()
                                };
                                Tab::new().label(label).suffix(
                                    Button::new(("close-file", ix))
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::Close)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            cx.stop_propagation();
                                            this.close_file(ix, window, cx);
                                        })),
                                )
                            })),
                    )
                    .when(markdown, |this| {
                        this.child(
                            Button::new("toggle-preview")
                                .ghost()
                                .xsmall()
                                .mr_2()
                                .label(if file.preview { "Source" } else { "Preview" })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.toggle_preview(window, cx)
                                })),
                        )
                    }),
            )
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(file.path.clone()),
            )
            .when(file.changed_on_disk, |this| {
                this.child(
                    h_flex()
                        .flex_none()
                        .px_3()
                        .py_1p5()
                        .gap_2()
                        .text_xs()
                        .bg(cx.theme().warning.opacity(0.15))
                        .child(
                            div().flex_1().child(
                                "Changed on disk while you were editing. Saving overwrites it.",
                            ),
                        )
                        .child(
                            Button::new("reload-file")
                                .xsmall()
                                .ghost()
                                .label("Reload")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.reload_from_disk(active, window, cx)
                                })),
                        ),
                )
            })
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_line_targets() {
        assert_eq!(parse_line_target("42"), Some((42, 1)));
        assert_eq!(parse_line_target(" 42:7 "), Some((42, 7)));
        assert_eq!(parse_line_target("0"), None);
        assert_eq!(parse_line_target("x"), None);
        assert_eq!(parse_line_target("3:"), None);
    }

    #[test]
    fn languages_by_name_and_extension() {
        assert_eq!(language_for("crates/yin/src/main.rs"), "rust");
        assert_eq!(language_for("web/App.TSX"), "tsx");
        assert_eq!(language_for("Makefile"), "make");
        assert_eq!(language_for("notes"), "text");
        assert_eq!(language_for("a.b/c"), "text");
    }

    #[test]
    fn decorations_mark_changed_files_and_their_folders() {
        let status = yin_git::Status {
            branch: Default::default(),
            entries: vec![yin_git::StatusEntry {
                path: "src/ui/view.rs".into(),
                orig_path: None,
                status: FileStatus::Untracked,
            }],
        };
        let decorations = TreeDecorations::from_status(&status);
        assert_eq!(
            decorations.files.get("src/ui/view.rs"),
            Some(&Change::Added)
        );
        assert!(decorations.dirs.contains("src") && decorations.dirs.contains("src/ui"));
        assert!(!decorations.dirs.contains("src/ui/view.rs"));
    }
}
