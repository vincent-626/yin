use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, StyledExt as _,
    TitleBar, WindowExt as _,
    button::{Button, ButtonVariant, ButtonVariants as _},
    checkbox::Checkbox,
    command::CommandState,
    diff::{Diff, DiffFile, DiffMode, DiffState},
    h_flex,
    input::{InputEvent, Textarea, TextareaState},
    list::ListItem,
    notification::Notification,
    resizable::{h_resizable, resizable_panel},
    scroll::Scrollbar,
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable as _, Hsla,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render, ScrollStrategy,
    SharedString, Styled as _, Task, UniformListScrollHandle, Window, div,
    prelude::FluentBuilder as _, uniform_list,
};
use yin_git::{Change, Repository, Status};

use crate::changes::{self, FileKey, Row, Section};
use crate::watcher::{self, RepoWatcher};

mod annotations;
mod blame;
mod branches;
mod conflicts;
mod files;
mod finder;
mod git_bar;
mod history;
mod lsp;
mod palette;
mod preview_find;
mod pull_request;
mod remote;
mod review_all;
mod reviewing;
mod worktrees;

pub use worktrees::OpenRepository;
mod search;
mod staging;
mod stash;
mod style;

use remote::RemoteOp;

pub(crate) mod actions {
    gpui_kit::actions!(
        yin,
        [
            Refresh,
            ToggleSidebar,
            ToggleDiffMode,
            SelectNext,
            SelectPrevious,
            ToggleStaged,
            StageAll,
            UnstageAll,
            FocusDiff,
            FocusChanges,
            Commit,
            FocusCommitMessage,
            Discard,
            Fetch,
            Pull,
            Push,
            SwitchBranch,
            StageLines,
            StageHunk,
            ShowChanges,
            ShowHistory,
            StashPush,
            StashPop,
            ShowFiles,
            CloseFile,
            TogglePreview,
            OpenSelected,
            FindFile,
            ProjectSearch,
            SaveFile,
            GoToLine,
            FindReferences,
            ToggleReviewed,
            MarkAllReviewed,
            ToggleHideReviewed,
            SwitchWorktree,
            CommandPalette,
            FileHistory,
            ToggleBlame,
            CopyCommitHash,
            SelectNextExtend,
            SelectPreviousExtend,
            CompareBranch,
            AddNote,
            CopyReviewPrompt,
            ClearNotes,
            CreatePullRequest,
            ReviewPullRequest,
            OpenPullRequest,
            FindInPreview,
            ClosePreviewFind,
        ]
    );
}
use actions::*;

const CHANGES_CONTEXT: &str = "Changes";
const HISTORY_CONTEXT: &str = "History";
const FILES_CONTEXT: &str = "Files";
const SEARCH_CONTEXT: &str = "SearchResults";

/// What the sidebar shows; the main pane follows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Panel {
    Changes,
    History,
    Files,
    Search,
}

impl Panel {
    fn name(self) -> &'static str {
        match self {
            Panel::Changes => "changes",
            Panel::History => "history",
            Panel::Files => "files",
            Panel::Search => "search",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|panel| panel.name() == name)
    }

    const ALL: [Panel; 4] = [Panel::Files, Panel::Changes, Panel::History, Panel::Search];

    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|panel| *panel == self)
            .unwrap_or(0)
    }
}

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-r", Refresh, Some("Workspace")),
        KeyBinding::new("cmd-b", ToggleSidebar, Some("Workspace")),
        KeyBinding::new("cmd-shift-d", ToggleDiffMode, Some("Workspace")),
        KeyBinding::new("cmd-alt-s", StageAll, Some("Workspace")),
        KeyBinding::new("cmd-alt-u", UnstageAll, Some("Workspace")),
        // Not at Workspace level: text inputs must still receive [ and ].
        KeyBinding::new("]", SelectNext, Some(CHANGES_CONTEXT)),
        KeyBinding::new("[", SelectPrevious, Some(CHANGES_CONTEXT)),
        KeyBinding::new("]", SelectNext, Some(HISTORY_CONTEXT)),
        KeyBinding::new("[", SelectPrevious, Some(HISTORY_CONTEXT)),
        KeyBinding::new("]", SelectNext, Some(FILES_CONTEXT)),
        KeyBinding::new("[", SelectPrevious, Some(FILES_CONTEXT)),
        KeyBinding::new("]", SelectNext, Some(SEARCH_CONTEXT)),
        KeyBinding::new("[", SelectPrevious, Some(SEARCH_CONTEXT)),
        KeyBinding::new("]", SelectNext, Some("Diff")),
        KeyBinding::new("[", SelectPrevious, Some("Diff")),
        KeyBinding::new("escape", FocusChanges, Some("Workspace")),
        KeyBinding::new("down", SelectNext, Some(CHANGES_CONTEXT)),
        KeyBinding::new("j", SelectNext, Some(CHANGES_CONTEXT)),
        KeyBinding::new("up", SelectPrevious, Some(CHANGES_CONTEXT)),
        KeyBinding::new("k", SelectPrevious, Some(CHANGES_CONTEXT)),
        KeyBinding::new("space", ToggleStaged, Some(CHANGES_CONTEXT)),
        KeyBinding::new("enter", FocusDiff, Some(CHANGES_CONTEXT)),
        KeyBinding::new("cmd-enter", Commit, Some("Workspace")),
        KeyBinding::new("c", FocusCommitMessage, Some(CHANGES_CONTEXT)),
        KeyBinding::new("cmd-backspace", Discard, Some(CHANGES_CONTEXT)),
        KeyBinding::new("f", Fetch, Some(CHANGES_CONTEXT)),
        KeyBinding::new("p", Pull, Some(CHANGES_CONTEXT)),
        KeyBinding::new("shift-p", Push, Some(CHANGES_CONTEXT)),
        KeyBinding::new("b", SwitchBranch, Some(CHANGES_CONTEXT)),
        KeyBinding::new("r", ToggleReviewed, Some(CHANGES_CONTEXT)),
        KeyBinding::new("shift-r", MarkAllReviewed, Some(CHANGES_CONTEXT)),
        KeyBinding::new("h", ToggleHideReviewed, Some(CHANGES_CONTEXT)),
        KeyBinding::new("w", SwitchWorktree, Some(CHANGES_CONTEXT)),
        KeyBinding::new("cmd-alt-w", SwitchWorktree, Some("Workspace")),
        KeyBinding::new("s", StashPush, Some(CHANGES_CONTEXT)),
        KeyBinding::new("shift-s", StashPop, Some(CHANGES_CONTEXT)),
        KeyBinding::new("cmd-1", ShowFiles, Some("Workspace")),
        KeyBinding::new("cmd-2", ShowChanges, Some("Workspace")),
        KeyBinding::new("cmd-3", ShowHistory, Some("Workspace")),
        KeyBinding::new("cmd-shift-e", ShowFiles, Some("Workspace")),
        KeyBinding::new("cmd-w", CloseFile, Some("Workspace")),
        KeyBinding::new("cmd-s", SaveFile, Some("Workspace")),
        KeyBinding::new("ctrl-g", GoToLine, Some("Workspace")),
        KeyBinding::new("shift-f12", FindReferences, Some("Workspace")),
        KeyBinding::new(
            "f12",
            gpui_kit::component::input::GoToDefinition,
            Some("Input"),
        ),
        KeyBinding::new("cmd-p", FindFile, Some("Workspace")),
        KeyBinding::new("cmd-shift-p", CommandPalette, Some("Workspace")),
        KeyBinding::new("cmd-alt-h", FileHistory, Some("Workspace")),
        KeyBinding::new("cmd-alt-b", ToggleBlame, Some("Workspace")),
        KeyBinding::new("cmd-shift-f", ProjectSearch, Some("Workspace")),
        // The editor binds cmd-shift-f to replace; keep it for project search,
        // as Zed does, and move replace to cmd-alt-f.
        KeyBinding::new("cmd-shift-f", ProjectSearch, Some("Input")),
        KeyBinding::new(
            "cmd-alt-f",
            gpui_kit::component::input::Replace,
            Some("Input"),
        ),
        KeyBinding::new("down", SelectNext, Some(SEARCH_CONTEXT)),
        KeyBinding::new("j", SelectNext, Some(SEARCH_CONTEXT)),
        KeyBinding::new("up", SelectPrevious, Some(SEARCH_CONTEXT)),
        KeyBinding::new("k", SelectPrevious, Some(SEARCH_CONTEXT)),
        KeyBinding::new("enter", OpenSelected, Some(SEARCH_CONTEXT)),
        KeyBinding::new("cmd-shift-v", TogglePreview, Some("Workspace")),
        KeyBinding::new("enter", OpenSelected, Some(FILES_CONTEXT)),
        KeyBinding::new("down", SelectNext, Some(HISTORY_CONTEXT)),
        KeyBinding::new("j", SelectNext, Some(HISTORY_CONTEXT)),
        KeyBinding::new("up", SelectPrevious, Some(HISTORY_CONTEXT)),
        KeyBinding::new("k", SelectPrevious, Some(HISTORY_CONTEXT)),
        KeyBinding::new("shift-down", SelectNextExtend, Some(HISTORY_CONTEXT)),
        KeyBinding::new("shift-j", SelectNextExtend, Some(HISTORY_CONTEXT)),
        KeyBinding::new("shift-up", SelectPreviousExtend, Some(HISTORY_CONTEXT)),
        KeyBinding::new("shift-k", SelectPreviousExtend, Some(HISTORY_CONTEXT)),
        KeyBinding::new("c", CompareBranch, Some(HISTORY_CONTEXT)),
        KeyBinding::new("enter", FocusDiff, Some(HISTORY_CONTEXT)),
        KeyBinding::new("cmd-c", CopyCommitHash, Some(HISTORY_CONTEXT)),
        KeyBinding::new("b", SwitchBranch, Some(HISTORY_CONTEXT)),
        KeyBinding::new("space", StageLines, Some("Diff")),
        KeyBinding::new("s", StageHunk, Some("Diff")),
        KeyBinding::new("n", AddNote, Some("Diff")),
        KeyBinding::new("cmd-alt-c", CopyReviewPrompt, Some("Workspace")),
        KeyBinding::new("cmd-f", FindInPreview, Some("MarkdownPreview")),
        KeyBinding::new("cmd-f", FindInPreview, Some("PreviewFind")),
        KeyBinding::new("escape", ClosePreviewFind, Some("PreviewFind")),
    ]);
}

pub struct Workspace {
    repo: Repository,
    status: Status,
    stashes: Vec<yin_git::Stash>,
    in_progress: Option<yin_git::InProgress>,
    /// Git's prepared merge message, waiting to fill an empty commit box.
    merge_message: Option<String>,
    reviews: crate::review::Reviews,
    review_hashes: std::collections::HashMap<String, u64>,
    reviewed: Arc<std::collections::HashSet<String>>,
    hide_reviewed: bool,
    /// Every change against HEAD, in one scrolling diff.
    all_diff: Entity<DiffState>,
    all_patch: String,
    _all_task: Task<()>,
    rows: Arc<Vec<Row>>,
    selected: Option<usize>,
    loaded: bool,
    /// The file the diff pane shows, and the patch it was built from.
    diff: Entity<DiffState>,
    diff_key: Option<FileKey>,
    diff_patch: String,
    diff_mode: DiffMode,
    error: Option<SharedString>,
    commit_message: Entity<TextareaState>,
    amend: bool,
    /// A git write is running.
    busy: bool,
    sidebar_visible: bool,
    changes_focus: FocusHandle,
    changes_scroll: UniformListScrollHandle,
    status_generation: usize,
    diff_generation: usize,
    _status_task: Task<()>,
    _diff_task: Task<()>,
    _operation_task: Task<()>,
    _watcher: Option<RepoWatcher>,
    _watch_task: Task<()>,
    remote_op: Option<RemoteOp>,
    panel: Panel,
    commits: Arc<Vec<yin_git::Commit>>,
    commit_selected: Option<usize>,
    /// The other end of a run of selected commits.
    commit_anchor: Option<usize>,
    /// The History tab compares two branches.
    history_compare: Option<history::Compare>,
    /// The branches HEAD's branch is stacked with.
    stack: Option<yin_git::Stack>,
    /// The patch the History diff shows.
    history_patch: String,
    /// The branch picker picks a branch to compare with.
    picking_compare: bool,
    notes: crate::notes::Notes,
    note_draft: Option<annotations::NoteDraft>,
    pull_request: Option<yin_git::PullRequest>,
    review_threads: Vec<yin_git::ReviewThread>,
    pr_check: Option<pull_request::PrCheck>,
    preview_find: Option<preview_find::PreviewFind>,
    _pr_task: Task<()>,
    commit_detail: Option<history::CommitDetail>,
    history_diff: Entity<DiffState>,
    history_focus: FocusHandle,
    history_scroll: UniformListScrollHandle,
    history_loaded: bool,
    /// HEAD when the history was read; a change means it is stale.
    history_head: Option<String>,
    _history_task: Task<()>,
    _commit_task: Task<()>,
    /// The History tab shows only this file's commits.
    history_path: Option<String>,
    /// Long commit messages show in full.
    commit_body_expanded: bool,
    /// A commit to select once the history loads.
    pending_commit: Option<String>,
    /// Blame for one open file, when shown.
    blame: Option<(String, Arc<Vec<yin_git::BlameLine>>)>,
    files: Arc<Vec<String>>,
    files_loaded: bool,
    /// The status paths that add, remove or move files, when the tree was
    /// last scanned: edits alone don't change the tree, so they don't rescan.
    tree_shape: Option<Vec<String>>,
    file_tree: Entity<gpui_kit::component::tree::TreeState>,
    expanded_dirs: std::collections::HashSet<String>,
    tree_decorations: Arc<files::TreeDecorations>,
    open_files: Vec<files::OpenFile>,
    active_file: Option<usize>,
    file_pane_focus: FocusHandle,
    panes: Entity<gpui_kit::component::resizable::ResizableState>,
    /// Sidebar width in points, once dragged or restored.
    sidebar_width: Option<f32>,
    _files_task: Task<()>,
    finder: Entity<CommandState>,
    finder_results: Vec<String>,
    search: search::SearchState,
    search_input: Entity<gpui_kit::component::input::InputState>,
    search_focus: FocusHandle,
    search_scroll: UniformListScrollHandle,
    _search_task: Task<()>,
    goto_input: Entity<gpui_kit::component::input::InputState>,
    servers: lsp::Servers,
    diagnostics: lsp::Diagnostics,
    branches: Vec<yin_git::BranchRef>,
    branch_picker: Entity<CommandState>,
    worktree_picker: Entity<CommandState>,
    palette: Entity<CommandState>,
    _remote_task: Task<()>,
    _relayout_task: Task<()>,
}

impl Workspace {
    pub fn new(repo: Repository, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let reviews = crate::review::Reviews::load(repo.workdir());
        let notes = crate::notes::Notes::load(repo.workdir());
        window.set_window_title(&repo.name());
        let diff = cx.new(|cx| DiffState::new([], cx));
        cx.observe(&diff, |_, _, cx| cx.notify()).detach();
        let panes = cx.new(|_| gpui_kit::component::resizable::ResizableState::default());
        cx.subscribe(
            &panes,
            |this, panes, _: &gpui_kit::component::resizable::ResizablePanelEvent, cx| {
                if this.sidebar_visible
                    && let Some(width) = panes.read(cx).sizes().first()
                {
                    this.sidebar_width = Some((*width).into());
                }
            },
        )
        .detach();
        let all_diff = cx.new(|cx| DiffState::new([], cx));
        cx.observe(&all_diff, |_, _, cx| cx.notify()).detach();
        let history_diff = cx.new(|cx| DiffState::new([], cx));
        cx.observe(&history_diff, |_, _, cx| cx.notify()).detach();
        cx.observe_window_bounds(window, |this, _, cx| this.schedule_relayout(cx))
            .detach();
        let changes_focus = cx.focus_handle().tab_stop(true);
        changes_focus.focus(window, cx);
        let commit_message = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Commit message")
                .auto_grow(3, 12)
        });
        cx.subscribe_in(
            &commit_message,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter {
                    secondary: true, ..
                } = event
                {
                    this.commit(window, cx);
                }
            },
        )
        .detach();
        let mut this = Self {
            repo,
            status: Status::default(),
            stashes: Vec::new(),
            in_progress: None,
            merge_message: None,
            reviews,
            review_hashes: Default::default(),
            reviewed: Arc::default(),
            hide_reviewed: false,
            all_diff,
            all_patch: String::new(),
            _all_task: Task::ready(()),
            rows: Arc::default(),
            selected: None,
            loaded: false,
            diff,
            diff_key: None,
            diff_patch: String::new(),
            diff_mode: DiffMode::Unified,
            error: None,
            commit_message,
            amend: false,
            busy: false,
            sidebar_visible: true,
            changes_focus,
            changes_scroll: UniformListScrollHandle::new(),
            status_generation: 0,
            diff_generation: 0,
            _status_task: Task::ready(()),
            _diff_task: Task::ready(()),
            _operation_task: Task::ready(()),
            _watcher: None,
            _watch_task: Task::ready(()),
            remote_op: None,
            panel: Panel::Changes,
            commits: Arc::default(),
            commit_selected: None,
            commit_anchor: None,
            history_compare: None,
            stack: None,
            history_patch: String::new(),
            picking_compare: false,
            notes,
            note_draft: None,
            pull_request: None,
            review_threads: Vec::new(),
            pr_check: None,
            preview_find: None,
            _pr_task: Task::ready(()),
            commit_detail: None,
            history_diff,
            history_focus: cx.focus_handle().tab_stop(true),
            history_scroll: UniformListScrollHandle::new(),
            history_loaded: false,
            history_head: None,
            _history_task: Task::ready(()),
            _commit_task: Task::ready(()),
            history_path: None,
            commit_body_expanded: false,
            pending_commit: None,
            blame: None,
            files: Arc::default(),
            files_loaded: false,
            tree_shape: None,
            file_tree: Self::new_file_tree(cx),
            expanded_dirs: Default::default(),
            tree_decorations: Arc::default(),
            open_files: Vec::new(),
            active_file: None,
            file_pane_focus: cx.focus_handle(),
            panes,
            sidebar_width: None,
            _files_task: Task::ready(()),
            finder: cx.new(|cx| CommandState::new(window, cx)),
            finder_results: Vec::new(),
            search: Default::default(),
            search_input: Self::new_search_input(window, cx),
            search_focus: cx.focus_handle().tab_stop(true),
            search_scroll: UniformListScrollHandle::new(),
            _search_task: Task::ready(()),
            servers: Default::default(),
            diagnostics: Default::default(),
            goto_input: cx.new(|cx| {
                gpui_kit::component::input::InputState::new(window, cx)
                    .placeholder("Line, or line:column")
            }),
            branches: Vec::new(),
            branch_picker: cx.new(|cx| CommandState::new(window, cx)),
            worktree_picker: cx.new(|cx| CommandState::new(window, cx)),
            palette: cx.new(|cx| CommandState::new(window, cx)),
            _remote_task: Task::ready(()),
            _relayout_task: Task::ready(()),
        };
        // The test scheduler is deterministic and rejects wakeups from the
        // FSEvents thread; tests refresh after each operation anyway.
        if !cfg!(test) {
            this.watch(window, cx);
        }
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.refresh(cx);
                this.reload_open_files(window, cx);
            } else {
                this.save_session(cx);
            }
        })
        .detach();
        this.refresh(cx);
        if !cfg!(test) {
            this.restore_session(window, cx);
        }
        this
    }

    /// Refreshes after filesystem changes, batching bursts such as a build or
    /// an agent rewriting many files.
    fn watch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (watcher, events) = match watcher::watch(&self.repo) {
            Ok(watch) => watch,
            Err(error) => {
                self.error = Some(format!("not watching for changes: {error}").into());
                return;
            }
        };
        self._watcher = Some(watcher);
        self._watch_task = cx.spawn_in(window, async move |this, cx| {
            while events.recv().await.is_ok() {
                // Long enough that an agent writing steadily costs a few
                // refreshes a second, short enough to feel live.
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
                while events.try_recv().is_ok() {}
                let refreshed = this.update_in(cx, |this, window, cx| {
                    this.refresh(cx);
                    this.reload_open_files(window, cx);
                });
                if refreshed.is_err() {
                    break;
                }
            }
        });
    }

    /// The Diff learns its viewport while painting, and a notify during paint
    /// doesn't schedule another frame, so rows can keep a stale width: the
    /// window opens at a default size and then grows, or the user resizes it.
    /// Redraw twice, once to measure and once to lay out at the new width.
    fn schedule_relayout(&mut self, cx: &mut Context<Self>) {
        self._relayout_task = cx.spawn(async move |this, cx| {
            for _ in 0..2 {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let Ok(()) = this.update(cx, |this, cx| {
                    this.diff.update(cx, |_, cx| cx.notify());
                    this.history_diff.update(cx, |_, cx| cx.notify());
                    this.all_diff.update(cx, |_, cx| cx.notify());
                }) else {
                    return;
                };
            }
        });
    }

    #[cfg(test)]
    pub fn selected_path(&self) -> Option<String> {
        self.selected_key().map(|key| key.path)
    }

    #[cfg(test)]
    pub fn diff_state(&self) -> Entity<DiffState> {
        if self.shows_one_file() {
            self.diff.clone()
        } else {
            self.all_diff.clone()
        }
    }

    /// The files the History diff shows, and its title-bar summary.
    #[cfg(test)]
    pub fn history_files(&self, cx: &App) -> Vec<String> {
        let files = self.history_diff.read(cx).files();
        files.iter().map(|file| file.path().to_string()).collect()
    }

    #[cfg(test)]
    pub fn compare_for_test(&mut self, base: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.compare(base.into(), "HEAD".into(), window, cx);
    }

    #[cfg(test)]
    pub fn notes(&self) -> &[crate::notes::Note] {
        self.notes.items()
    }

    #[cfg(test)]
    pub fn history_commit_count(&self) -> usize {
        self.commits.len()
    }

    #[cfg(test)]
    pub fn commit_message_text(&self, cx: &App) -> String {
        self.commit_message.read(cx).value().to_string()
    }

    /// Saves open tabs and layout for the next time this repository opens.
    pub fn save_session(&self, _: &App) {
        let session = crate::state::Session {
            open_files: self
                .open_files
                .iter()
                .map(|file| file.path.clone())
                .collect(),
            active_file: self.active_file,
            panel: self.panel.name().to_owned(),
            sidebar_hidden: !self.sidebar_visible,
            split_diff: self.diff_mode == DiffMode::Split,
            sidebar_width: self.sidebar_width,
        };
        crate::state::save_session(self.repo.workdir(), &session);
    }

    fn restore_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = crate::state::load_session(self.repo.workdir()) else {
            return;
        };
        for path in &session.open_files {
            if self.repo.workdir().join(path).is_file() {
                self.open_file(path, None, window, cx);
            }
        }
        self.active_file = session
            .active_file
            .filter(|ix| *ix < self.open_files.len())
            .or(self.active_file);
        if session.split_diff {
            self.diff_mode = DiffMode::Split;
            for diff in [&self.diff, &self.all_diff, &self.history_diff] {
                diff.update(cx, |diff, cx| diff.set_mode(DiffMode::Split, cx));
            }
        }
        self.sidebar_width = session.sidebar_width.filter(|width| *width >= 150.);
        let panel = Panel::from_name(&session.panel).unwrap_or(Panel::Changes);
        self.show_panel(panel, window, cx);
        self.sidebar_visible = !session.sidebar_hidden;
    }

    fn selected_key(&self) -> Option<FileKey> {
        self.rows.get(self.selected?)?.key()
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.status_generation += 1;
        let generation = self.status_generation;
        let repo = self.repo.clone();
        self._status_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let status = repo.status()?;
                    let in_progress = repo.in_progress()?;
                    let merge_message = match in_progress {
                        Some(yin_git::InProgress::Merge) => repo.merge_message()?,
                        _ => None,
                    };
                    let hashes = reviewing::hash_changed_files(repo.workdir(), &status);
                    anyhow::Ok((
                        status,
                        repo.stashes()?,
                        (in_progress, merge_message),
                        hashes,
                    ))
                })
                .await;
            this.update(cx, |this, cx| {
                if this.status_generation != generation {
                    return;
                }
                match result {
                    Ok((status, stashes, (in_progress, merge_message), hashes)) => {
                        // Offer the message once per merge, not after every refresh.
                        if in_progress != this.in_progress {
                            this.merge_message = merge_message;
                        }
                        this.in_progress = in_progress;
                        this.install_review_hashes(hashes);
                        this.install_status(status, stashes, cx)
                    }
                    Err(error) => this.error = Some(error.to_string().into()),
                }
                cx.notify();
            })
            .ok();
        });
    }

    fn install_status(
        &mut self,
        status: Status,
        stashes: Vec<yin_git::Stash>,
        cx: &mut Context<Self>,
    ) {
        self.stashes = stashes;
        self.status = status;
        self.loaded = true;
        self.rebuild_rows(cx);
        if self.history_loaded && self.history_head != self.status.branch.oid {
            self.load_history(cx);
        }
        self.update_tree_decorations();
        let shape = tree_shape(&self.status);
        if self.files_loaded && self.tree_shape.as_ref() != Some(&shape) {
            self.load_files(cx);
        }
        self.tree_shape = Some(shape);
        self.check_pull_request(false, cx);
        // Reload even when the selection is unchanged: the file may have changed.
        self.load_diff(cx);
    }

    /// Rebuilds the Changes list from the last status, keeping the selection.
    fn rebuild_rows(&mut self, cx: &mut Context<Self>) {
        let previous = self.selected_key();
        let rows = changes::rows(&self.status, &self.stashes, &self.hidden_paths());
        let selected = changes::reselect(&rows, previous.as_ref(), self.selected);
        self.rows = Arc::new(rows);
        self.selected = selected;
        if self.selected_key() != previous {
            self.load_diff(cx);
        }
        self.load_all_diff(cx);
        cx.notify();
    }

    fn select(&mut self, ix: Option<usize>, cx: &mut Context<Self>) {
        if ix == self.selected {
            return;
        }
        self.selected = ix;
        if let Some(ix) = ix {
            self.changes_scroll
                .scroll_to_item(ix, ScrollStrategy::Nearest);
        }
        self.load_diff(cx);
        self.scroll_all_diff_to_selection(cx);
        cx.notify();
    }

    /// Loads the selected conflict or stash for the main pane; other rows
    /// show in the changes diff, so building their diff here would be waste.
    fn load_diff(&mut self, cx: &mut Context<Self>) {
        self.diff_generation += 1;
        let generation = self.diff_generation;
        if !self.shows_one_file() {
            if self.diff_key.take().is_some() {
                self.diff_patch.clear();
                self.diff.update(cx, |state, cx| state.set_files([], cx));
            }
            return;
        }
        let Some(key) = self.selected_key() else {
            self.diff_key = None;
            self.diff_patch.clear();
            self.diff.update(cx, |state, cx| state.set_files([], cx));
            return;
        };
        let repo = self.repo.clone();
        let stash = self.stash_for(&key.path);
        self._diff_task = cx.spawn(async move |this, cx| {
            let target = key.section.diff_target();
            let key_section = key.section;
            let path = key.path.clone();
            let result = cx
                .background_spawn(async move {
                    match (target, stash) {
                        // A conflicted file is shown whole, markers included.
                        _ if key_section == Section::Conflicts => repo.read_file(&path),
                        (Some(target), _) => repo.diff(&path, target),
                        (None, Some(stash)) => repo.stash_patch(&stash),
                        (None, None) => Ok(String::new()),
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                if this.diff_generation != generation {
                    return;
                }
                this.install_diff(key, result, cx)
            })
            .ok();
        });
    }

    fn install_diff(
        &mut self,
        key: FileKey,
        result: anyhow::Result<String>,
        cx: &mut Context<Self>,
    ) {
        let patch = match result {
            Ok(patch) => patch,
            Err(error) => {
                self.error = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        // Same file, same patch: keep the scroll position and selection.
        if self.diff_key.as_ref() == Some(&key) && self.diff_patch == patch {
            return;
        }
        let parsed = if key.section == Section::Conflicts {
            DiffFile::parse_conflicts(key.path.clone(), &patch).map(|file| vec![file])
        } else {
            DiffFile::parse(&patch)
        };
        let files = match parsed {
            Ok(files) => files,
            Err(error) => {
                self.error = Some(format!("couldn't read diff of {}: {error}", key.path).into());
                Vec::new()
            }
        };
        self.diff_key = Some(key);
        self.diff_patch = patch;
        self.diff.update(cx, |state, cx| state.set_files(files, cx));
        self.schedule_relayout(cx);
        cx.notify();
    }

    /// Runs a git write off the main thread, then refreshes.
    fn run(
        &mut self,
        operation: impl FnOnce(&Repository) -> anyhow::Result<()> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_then(operation, |_, _, _| {}, window, cx);
    }

    /// Like [`Self::run`], calling `on_success` when the operation succeeds.
    /// Failures, such as hook output, become an error notification.
    fn run_then(
        &mut self,
        operation: impl FnOnce(&Repository) -> anyhow::Result<()> + Send + 'static,
        on_success: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo = self.repo.clone();
        self.busy = true;
        cx.notify();
        self._operation_task = cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_spawn(async move { operation(&repo) }).await;
            this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(()) => on_success(this, window, cx),
                    Err(error) => {
                        window.push_notification(Notification::error(error.to_string()), cx)
                    }
                }
                this.refresh(cx);
                cx.notify();
            })
            .ok();
        });
    }

    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let message = self.commit_message.read(cx).value().trim().to_string();
        let amend = self.amend;
        if self.busy || message.is_empty() || (!amend && self.status.staged().next().is_none()) {
            return;
        }
        self.run_then(
            move |repo| repo.commit(&message, amend),
            |this, window, cx| {
                this.amend = false;
                this.commit_message
                    .update(cx, |state, cx| state.set_value("", window, cx));
                this.changes_focus.focus(window, cx);
            },
            window,
            cx,
        );
    }

    fn toggle_amend(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.amend = !self.amend;
        cx.notify();
        if !self.amend || !self.commit_message.read(cx).value().trim().is_empty() {
            return;
        }
        // Start from HEAD's message, as `git commit --amend` would.
        let repo = self.repo.clone();
        cx.spawn_in(window, async move |this, cx| {
            let message = cx
                .background_spawn(async move { repo.head_message() })
                .await;
            this.update_in(cx, |this, window, cx| {
                if let Ok(message) = message
                    && this.amend
                    && this.commit_message.read(cx).value().is_empty()
                {
                    this.commit_message
                        .update(cx, |state, cx| state.set_value(message, window, cx));
                }
            })
            .ok();
        })
        .detach();
    }

    fn toggle_staged_at(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.rows.get(ix).and_then(Row::key) else {
            return;
        };
        match key.section {
            Section::Stashes => return,
            Section::Conflicts => return self.mark_resolved(key, window, cx),
            _ => {}
        }
        self.run(
            move |repo| {
                if key.section.is_staged() {
                    repo.unstage(&[&key.path])
                } else {
                    repo.stage(&[&key.path])
                }
            },
            window,
            cx,
        );
    }

    fn toggle_section(&mut self, section: Section, window: &mut Window, cx: &mut Context<Self>) {
        let paths: Vec<String> = self
            .rows
            .iter()
            .filter_map(|row| match row {
                Row::File { section: s, entry } if *s == section => Some(entry.path.clone()),
                _ => None,
            })
            .collect();
        self.run(
            move |repo| {
                let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
                if section.is_staged() {
                    repo.unstage(&paths)
                } else {
                    repo.stage(&paths)
                }
            },
            window,
            cx,
        );
    }

    /// Asks, then throws away the selected file's unstaged changes. Untracked
    /// files go to the Trash so a mistake is recoverable.
    fn discard_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.selected_key() else {
            return;
        };
        let untracked = match key.section {
            Section::Changes => false,
            Section::Untracked => true,
            Section::Staged | Section::Conflicts => {
                window.push_notification("Unstage the file before discarding it.", cx);
                return;
            }
            Section::Stashes => {
                if let Some(stash) = self.stash_for(&key.path) {
                    self.confirm_drop_stash(stash, window, cx);
                }
                return;
            }
        };
        let name = key.path.rsplit('/').next().unwrap_or(&key.path).to_owned();
        let (title, description, ok) = if untracked {
            (
                format!("Move {name} to the Trash?"),
                format!(
                    "{} is untracked. You can restore it from the Trash.",
                    key.path
                ),
                "Move to Trash",
            )
        } else {
            (
                format!("Discard changes to {name}?"),
                format!(
                    "Unstaged changes to {} will be lost. Staged changes are kept.",
                    key.path
                ),
                "Discard",
            )
        };
        let this = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let this = this.clone();
            let key = key.clone();
            alert
                .title(title.clone())
                .description(description.clone())
                .confirm()
                .ok_text(ok)
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    let key = key.clone();
                    this.update(cx, |this, cx| {
                        this.run(
                            move |repo| {
                                if untracked {
                                    move_to_trash(&repo.workdir().join(&key.path))
                                } else {
                                    repo.discard(&[&key.path])
                                }
                            },
                            window,
                            cx,
                        )
                    })
                    .ok();
                    true
                })
        });
    }

    fn focus_diff(&self, window: &mut Window, cx: &mut Context<Self>) {
        let diff = match self.panel {
            Panel::Changes if self.shows_one_file() => &self.diff,
            Panel::Changes => &self.all_diff,
            Panel::History => &self.history_diff,
            Panel::Files | Panel::Search => return self.focus_active_file(window, cx),
        };
        if !diff.read(cx).files().is_empty() {
            diff.read(cx).focus_handle(cx).focus(window, cx);
        }
    }

    /// Shows a sidebar tab and focuses its list.
    fn show_panel(&mut self, panel: Panel, window: &mut Window, cx: &mut Context<Self>) {
        self.panel = panel;
        self.sidebar_visible = true;
        match panel {
            Panel::Changes => self.changes_focus.focus(window, cx),
            Panel::History => {
                if !self.history_loaded {
                    self.load_history(cx);
                }
                self.history_focus.focus(window, cx);
            }
            Panel::Files => {
                if !self.files_loaded {
                    self.load_files(cx);
                }
                self.file_tree.update(cx, |tree, cx| tree.focus(window, cx));
            }
            Panel::Search => self.focus_search(window, cx),
        }
        cx.notify();
    }

    fn render_sidebar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .min_h_0()
            .bg(cx.theme().sidebar)
            .child(
                TabBar::new("sidebar-tabs")
                    .underline()
                    .small()
                    .w_full()
                    .flex_none()
                    .px_1()
                    .selected_index(self.panel.index())
                    .on_click(cx.listener(|this, ix: &usize, window, cx| {
                        let panel = Panel::ALL.get(*ix).copied().unwrap_or(Panel::Changes);
                        this.show_panel(panel, window, cx);
                    }))
                    .child(
                        Tab::new()
                            .prefix(tab_icon(Icon::empty().path("icons/folder.svg"), cx))
                            .label("Files"),
                    )
                    .child(
                        Tab::new()
                            .prefix(tab_icon(Icon::empty().path("icons/file-diff.svg"), cx))
                            .label("Changes")
                            .when(!self.status.entries.is_empty(), |tab| {
                                tab.suffix(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(self.status.entries.len().to_string()),
                                )
                            }),
                    )
                    .child(
                        Tab::new()
                            .prefix(tab_icon(Icon::empty().path("icons/git-graph.svg"), cx))
                            .label("History"),
                    )
                    .child(
                        Tab::new()
                            .prefix(tab_icon(
                                Icon::new(gpui_kit::component::IconName::Search),
                                cx,
                            ))
                            .label("Search"),
                    ),
            )
            .map(|this| match self.panel {
                Panel::Changes => this.child(self.render_changes(window, cx)),
                Panel::History => this.child(self.render_history(window, cx)),
                Panel::Files => this.child(self.render_file_tree(cx)),
                Panel::Search => this.child(self.render_search(window, cx)),
            })
    }

    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let head = self.head_label();
        TitleBar::new().child(
            h_flex()
                .size_full()
                .gap_3()
                .text_sm()
                .child(div().font_medium().child(self.repo.name()))
                .child(
                    Button::new("branch")
                        .ghost()
                        .xsmall()
                        .icon(Icon::empty().path("icons/git-branch.svg"))
                        .label(head)
                        .tooltip_with_action("Switch branch", &SwitchBranch, Some(CHANGES_CONTEXT))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_branch_picker(window, cx)),
                        ),
                )
                .child(div().flex_1())
                .when_some(self.error.clone(), |this, error| {
                    this.child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(cx.theme().red)
                            .child(error),
                    )
                })
                .when(
                    self.diagnostics.errors + self.diagnostics.warnings > 0,
                    |this| {
                        let (errors, warnings) =
                            (self.diagnostics.errors, self.diagnostics.warnings);
                        let muted = cx.theme().muted_foreground;
                        this.child(
                            h_flex()
                                .flex_none()
                                .gap_2()
                                .text_xs()
                                .child(
                                    div()
                                        .text_color(if errors > 0 { cx.theme().red } else { muted })
                                        .child(format!(
                                            "{errors} error{}",
                                            if errors == 1 { "" } else { "s" }
                                        )),
                                )
                                .child(div().text_color(muted).child(format!(
                                    "{warnings} warning{}",
                                    if warnings == 1 { "" } else { "s" }
                                ))),
                        )
                    },
                )
                .child(div().w_1()),
        )
    }

    fn render_changes(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows.clone();
        let selected = self.selected;
        let focused = self.changes_focus.is_focused(window);
        let owner = cx.entity().downgrade();
        let reviewed = self.reviewed.clone();
        let hidden = if self.hide_reviewed {
            self.reviewed.len()
        } else {
            0
        };
        v_flex()
            .flex_1()
            .min_h_0()
            .children(self.render_merge_banner(cx))
            .when(hidden > 0, |this| {
                this.child(
                    div()
                        .flex_none()
                        .px_3()
                        .py_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!(
                            "{hidden} reviewed file{} hidden · h to show",
                            if hidden == 1 { "" } else { "s" }
                        )),
                )
            })
            .child(
                div()
                    .id("changes")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .track_focus(&self.changes_focus)
                    .key_context(CHANGES_CONTEXT)
                    .on_mouse_down(
                        gpui_kit::MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.changes_focus.focus(window, cx)),
                    )
                    .when(self.loaded && rows.is_empty(), |this| {
                        this.child(
                            div()
                                .p_4()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(if hidden > 0 {
                                    "Everything is reviewed"
                                } else {
                                    "Working tree clean"
                                }),
                        )
                    })
                    .child(
                        uniform_list("change-rows", rows.len(), move |range, _, cx| {
                            range
                                .map(|ix| {
                                    render_row(
                                        ix, &rows[ix], selected, focused, &reviewed, &owner, cx,
                                    )
                                })
                                .collect()
                        })
                        .size_full()
                        .track_scroll(&self.changes_scroll),
                    )
                    .child(Scrollbar::vertical(&self.changes_scroll)),
            )
            .children(self.render_notes_strip(cx))
            .child(self.render_commit_box(window, cx))
    }

    fn render_commit_box(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_message = !self.commit_message.read(cx).value().trim().is_empty();
        let can_commit =
            !self.busy && has_message && (self.amend || self.status.staged().next().is_some());
        v_flex()
            .flex_none()
            .p_2()
            .gap_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(self.render_git_bar(window, cx))
            .child(Textarea::new(&self.commit_message).small())
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Checkbox::new("amend")
                            .small()
                            .label("Amend")
                            .checked(self.amend)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.toggle_amend(window, cx)),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("commit")
                            .primary()
                            .small()
                            .label(if self.amend { "Amend" } else { "Commit" })
                            .loading(self.busy)
                            .disabled(!can_commit)
                            .tooltip_with_action(
                                "Commit staged changes",
                                &Commit,
                                Some("Workspace"),
                            )
                            .on_click(cx.listener(|this, _, window, cx| this.commit(window, cx))),
                    ),
            )
    }

    fn render_diff(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let empty = self.diff.read(cx).files().is_empty();
        let selection_bar = self.render_conflict_bar(cx);
        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .children(selection_bar)
            .map(|this| {
                if empty {
                    this.child(
                        v_flex()
                            .size_full()
                            .items_center()
                            .justify_center()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(if !self.loaded {
                                "Reading status…"
                            } else if self.rows.is_empty() {
                                "Nothing to review"
                            } else if self.selected.is_none() {
                                "Select a file"
                            } else {
                                "No textual changes"
                            }),
                    )
                } else {
                    this.child(div().flex_1().min_h_0().child(
                        Diff::new(&self.diff).line_number(true).render_header({
                            let state = self.diff.clone();
                            move |file, _, cx| style::diff_file_header(file, &state, None, cx)
                        }),
                    ))
                }
            })
    }
}

/// Paths whose status means a file appeared, disappeared or moved.
fn tree_shape(status: &Status) -> Vec<String> {
    let moved = |change: Change| {
        matches!(
            change,
            Change::Added | Change::Deleted | Change::Renamed | Change::Copied
        )
    };
    status
        .entries
        .iter()
        .filter(|entry| match entry.status {
            yin_git::FileStatus::Tracked { index, worktree } => moved(index) || moved(worktree),
            yin_git::FileStatus::Untracked | yin_git::FileStatus::Conflicted => true,
        })
        .map(|entry| entry.path.clone())
        .collect()
}

/// An icon in front of a sidebar tab's label.
fn tab_icon(icon: Icon, cx: &App) -> impl IntoElement {
    div()
        .pl_1()
        .text_color(cx.theme().muted_foreground)
        .child(icon.size_3())
}

/// Uses NSFileManager rather than the crate's default of scripting Finder,
/// which needs Automation permission.
fn move_to_trash(path: &std::path::Path) -> anyhow::Result<()> {
    use trash::macos::{DeleteMethod, TrashContextExtMacos as _};
    let mut context = trash::TrashContext::default();
    context.set_delete_method(DeleteMethod::NsFileManager);
    context.delete(path)?;
    Ok(())
}

fn change_color(change: Change, cx: &App) -> Hsla {
    match change {
        Change::Added | Change::Copied => cx.theme().green,
        Change::Deleted => cx.theme().red,
        Change::Renamed => cx.theme().blue,
        _ => cx.theme().yellow,
    }
}

fn render_row(
    ix: usize,
    row: &Row,
    selected: Option<usize>,
    focused: bool,
    reviewed: &std::collections::HashSet<String>,
    owner: &gpui_kit::WeakEntity<Workspace>,
    cx: &App,
) -> gpui_kit::AnyElement {
    match row {
        Row::Header { section, count } => {
            let section = *section;
            let owner = owner.clone();
            h_flex()
                .id(("header", ix))
                .w_full()
                .h_7()
                .px_3()
                .gap_2()
                .text_xs()
                .font_medium()
                .text_color(cx.theme().muted_foreground)
                .child(
                    div()
                        .flex_1()
                        .child(format!("{} · {count}", section.title())),
                )
                .when(
                    !matches!(section, Section::Stashes | Section::Conflicts),
                    |this| {
                        this.child(
                            Button::new(("toggle-section", ix))
                                .ghost()
                                .xsmall()
                                .label(if section.is_staged() {
                                    "Unstage all"
                                } else {
                                    "Stage all"
                                })
                                .on_click(move |_, window, cx| {
                                    owner
                                        .update(cx, |this, cx| {
                                            this.toggle_section(section, window, cx)
                                        })
                                        .ok();
                                }),
                        )
                    },
                )
                .into_any_element()
        }
        Row::Stash(stash) => {
            stash::render_stash_row(ix, stash, selected == Some(ix), focused, owner, cx)
        }
        Row::File { section, entry } => {
            let change = changes::change_for(*section, entry);
            let (directory, filename) = entry
                .path
                .rsplit_once('/')
                .unwrap_or(("", entry.path.as_str()));
            let is_selected = selected == Some(ix);
            let is_reviewed = reviewed.contains(&entry.path);
            let select_owner = owner.clone();
            let toggle_owner = owner.clone();
            ListItem::new(("file", ix))
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
                            Checkbox::new(("stage", ix))
                                .small()
                                .checked(section.is_staged())
                                .on_click(move |_, window, cx| {
                                    toggle_owner
                                        .update(cx, |this, cx| {
                                            this.toggle_staged_at(ix, window, cx)
                                        })
                                        .ok();
                                }),
                        )
                        .child(
                            div()
                                .flex_none()
                                .when(is_reviewed, |this| {
                                    this.text_color(cx.theme().muted_foreground).line_through()
                                })
                                .child(filename.to_owned()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(directory.to_owned()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .w_3()
                                .text_xs()
                                .font_medium()
                                .text_color(change_color(change, cx))
                                .child(match section {
                                    Section::Untracked => "U",
                                    Section::Conflicts => "!",
                                    _ => change.letter(),
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
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A merge stopped for conflicts: start from git's prepared message.
        if let Some(message) = self.merge_message.take()
            && self.commit_message.read(cx).value().trim().is_empty()
        {
            self.commit_message
                .update(cx, |state, cx| state.set_value(message, window, cx));
        }
        let rem = window.rem_size();
        v_flex()
            .id("workspace")
            .key_context("Workspace")
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(|this, _: &Refresh, _, cx| {
                this.tree_shape = None;
                this.refresh(cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, window, cx| {
                this.sidebar_visible = !this.sidebar_visible;
                if !this.sidebar_visible {
                    this.focus_diff(window, cx);
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleDiffMode, _, cx| {
                this.diff_mode = match this.diff_mode {
                    DiffMode::Unified => DiffMode::Split,
                    _ => DiffMode::Unified,
                };
                let mode = this.diff_mode;
                for diff in [&this.diff, &this.all_diff, &this.history_diff] {
                    diff.update(cx, |state, cx| state.set_mode(mode, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| match this.panel {
                Panel::Changes => {
                    let ix = changes::step(&this.rows, this.selected, 1);
                    this.select(ix, cx);
                }
                Panel::History => this.step_commit(1, false, cx),
                Panel::Files => this.step_open_file(1, cx),
                Panel::Search => this.step_search(1, cx),
            }))
            .on_action(
                cx.listener(|this, _: &SelectPrevious, _, cx| match this.panel {
                    Panel::Changes => {
                        let ix = changes::step(&this.rows, this.selected, -1);
                        this.select(ix, cx);
                    }
                    Panel::History => this.step_commit(-1, false, cx),
                    Panel::Files => this.step_open_file(-1, cx),
                    Panel::Search => this.step_search(-1, cx),
                }),
            )
            .on_action(cx.listener(|this, _: &StashPush, window, cx| this.stash_push(window, cx)))
            .on_action(
                cx.listener(|this, _: &StashPop, window, cx| this.stash_pop(None, window, cx)),
            )
            .on_action(cx.listener(|this, _: &ShowChanges, window, cx| {
                this.show_panel(Panel::Changes, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowHistory, window, cx| {
                this.set_history_path(None, window, cx)
            }))
            .on_action(cx.listener(|this, _: &FileHistory, window, cx| {
                let path = this
                    .active_file
                    .and_then(|ix| this.open_files.get(ix))
                    .map(|file| file.path.clone());
                if path.is_some() {
                    this.set_history_path(path, window, cx);
                }
            }))
            .on_action(
                cx.listener(|this, _: &ToggleBlame, window, cx| this.toggle_blame(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &SelectNextExtend, _, cx| this.step_commit(1, true, cx)),
            )
            .on_action(
                cx.listener(|this, _: &SelectPreviousExtend, _, cx| this.step_commit(-1, true, cx)),
            )
            .on_action(cx.listener(|this, _: &CompareBranch, window, cx| {
                this.picking_compare = true;
                this.open_branch_picker(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &AddNote, window, cx| {
                    this.add_note_for_selection(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &CopyReviewPrompt, window, cx| {
                this.copy_review_prompt(window, cx)
            }))
            .on_action(cx.listener(|this, _: &ClearNotes, window, cx| this.clear_notes(window, cx)))
            .on_action(cx.listener(|this, _: &CreatePullRequest, window, cx| {
                this.open_create_pull_request(window, cx)
            }))
            .on_action(cx.listener(|this, _: &ReviewPullRequest, window, cx| {
                this.review_pull_request(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &OpenPullRequest, _, cx| {
                    this.open_pull_request_in_browser(cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &FindInPreview, window, cx| {
                    this.open_preview_find(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &CopyCommitHash, window, cx| {
                this.copy_selected_commit_hash(window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowFiles, window, cx| {
                this.show_panel(Panel::Files, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CloseFile, window, cx| {
                if let Some(ix) = this.active_file {
                    this.close_file(ix, window, cx);
                }
            }))
            .on_action(
                cx.listener(|this, _: &SaveFile, window, cx| this.save_active_file(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &GoToLine, window, cx| this.prompt_go_to_line(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &FindReferences, window, cx| {
                    this.find_references(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &ToggleReviewed, _, cx| this.toggle_reviewed(cx)))
            .on_action(
                cx.listener(|this, _: &CommandPalette, window, cx| this.open_palette(window, cx)),
            )
            .on_action(cx.listener(|this, _: &SwitchWorktree, window, cx| {
                this.open_worktree_picker(window, cx)
            }))
            .on_action(cx.listener(|this, _: &MarkAllReviewed, _, cx| this.mark_all_reviewed(cx)))
            .on_action(
                cx.listener(|this, _: &ToggleHideReviewed, _, cx| this.toggle_hide_reviewed(cx)),
            )
            .on_action(
                cx.listener(|this, _: &TogglePreview, window, cx| this.toggle_preview(window, cx)),
            )
            .on_action(cx.listener(|this, _: &FindFile, window, cx| this.open_finder(window, cx)))
            .on_action(
                cx.listener(|this, _: &OpenSelected, window, cx| match this.panel {
                    Panel::Search => this.open_search_result(window, cx),
                    _ => this.open_selected_tree_item(window, cx),
                }),
            )
            .on_action(cx.listener(|this, _: &ProjectSearch, window, cx| {
                this.show_panel(Panel::Search, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleStaged, window, cx| {
                if let Some(ix) = this.selected {
                    this.toggle_staged_at(ix, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &StageAll, window, cx| {
                this.run(|repo| repo.stage_all(), window, cx);
            }))
            .on_action(cx.listener(|this, _: &UnstageAll, window, cx| {
                this.run(|repo| repo.unstage_all(), window, cx);
            }))
            .on_action(cx.listener(|this, _: &Commit, window, cx| this.commit(window, cx)))
            .on_action(
                cx.listener(|this, _: &Discard, window, cx| this.discard_selected(window, cx)),
            )
            .on_action(cx.listener(|this, _: &SwitchBranch, window, cx| {
                this.picking_compare = false;
                this.open_branch_picker(window, cx)
            }))
            .on_action(cx.listener(|this, _: &StageLines, window, cx| {
                this.stage_selection(false, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &StageHunk, window, cx| {
                    this.stage_selection(true, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &Fetch, window, cx| {
                this.run_remote(RemoteOp::Fetch, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &Pull, window, cx| {
                    this.run_remote(RemoteOp::Pull, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &Push, window, cx| {
                    this.run_remote(RemoteOp::Push, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &FocusCommitMessage, window, cx| {
                this.sidebar_visible = true;
                this.commit_message
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FocusDiff, window, cx| this.focus_diff(window, cx)))
            .on_action(cx.listener(|this, _: &FocusChanges, window, cx| {
                let panel = this.panel;
                this.show_panel(panel, window, cx);
            }))
            .child(self.render_title_bar(cx))
            .child(
                div().flex_1().min_h_0().child(
                    h_resizable("workspace-panes")
                        .with_state(&self.panes)
                        .when(self.sidebar_visible, |this| {
                            this.child(
                                resizable_panel()
                                    .size(self.sidebar_width.map_or(rem * 20., gpui_kit::px))
                                    .size_range(rem * 14. ..rem * 40.)
                                    .child(self.render_sidebar(window, cx)),
                            )
                        })
                        .child(resizable_panel().child(match self.panel {
                            Panel::Changes if self.shows_one_file() => {
                                self.render_diff(cx).into_any_element()
                            }
                            Panel::Changes => self.render_all_diff(window, cx),
                            Panel::History => self.render_commit_detail(window, cx),
                            Panel::Files | Panel::Search => self.render_open_files(cx),
                        })),
                ),
            )
    }
}
