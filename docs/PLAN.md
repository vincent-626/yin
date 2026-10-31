# yin — plan

yin is a personal, native code workspace for one user on Apple Silicon macOS. It
replaces Zed for the work that remains when coding agents do most of the
editing: **reviewing changes, running git, navigating and reading code, and
reading markdown.** Dark only (always yin, never yang).

This document is the source of truth for scope and order. Update the status
column as phases land; record decisions in the log at the bottom.

## Goals

1. **Review is the core loop.** Opening yin on a repo shows what changed, as a
   fast, syntax-highlighted diff, with stage/unstage one keystroke away.
2. **Git without a terminal** for the daily set: status, stage/unstage (file,
   hunk, line), commit, amend, branch create/switch, fetch, pull, push, log.
3. **Navigate and read**: file tree, fuzzy file finder, highlighted read-only
   code view, project search, rendered markdown.
4. **Light editing**: fix a typo, tweak a line, save. Not a full editor.
5. **Feels like Zed**: instant startup, 120 fps scrolling, keyboard-first,
   no spinners for local operations.

## Non-goals

- Light theme, Windows, Linux, Intel Macs.
- Running code: no terminal, debugger, tasks or test runner.
- Extensions, plugins, settings UI, collaboration, AI chat.
- Being a general editor: no multi-buffer refactors, vim mode, snippets.

## Principles

- **Native, Rust, GPU-rendered.** GPUI (Zed's UI framework) via gpui-kit.
- **Correct before clever.** Git writes go through the `git` CLI so hooks,
  signing, credential helpers and SSH config behave exactly as in a terminal.
  Reads start on the CLI too; move a read to `gix` only when a measurement
  says the CLI is the bottleneck.
- **UI never blocks on I/O.** Every git call and file read runs on the
  background executor; views render the last known state.
- **The UI-free core is tested.** `yin_git` (and later `yin_fs`) have no GPUI
  dependency and are covered by tests against throwaway repos.
- **Borrow, don't fork.** Read Zed and gpui-component in `ref/` for patterns;
  never depend on Zed's GPL crates.

## Architecture

```
crates/
  yin/          app binary: window, workspace (one module per feature), keymap
  yin_git/      repository model: status, diff, partial patches, branches,
                remotes, log, stash, conflicts
  yin_fs/       worktree scan, folder tree, fuzzy matching, project search
  (later)
  yin_lsp/      minimal LSP client for go-to-definition and hover
```

Workspace layout (Zed-like):

```
┌──────────────────────────── title bar: repo · branch · ahead/behind ───────┐
│ left dock          │ center                                                 │
│  [Changes] [Files] │  tabs: Changes diff | file views | markdown previews   │
│  staged / unstaged │                                                        │
│  untracked         │                                                        │
│  commit box        │                                                        │
├────────────────────┴────────────────────────────────────────────────────────┤
│ status bar: branch · sync state · last git op result                        │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Key dependencies

| Need | Choice | Why |
|---|---|---|
| UI | `gpui-kit` =0.8.1 (GPUI `gpui-pre` 0.3.10 + gpui-component) | Zed's renderer; ships Diff, Input/code editor, Tree, Dock, Resizable, Title bar |
| Shaders | `runtime_shaders` (enabled by gpui-kit) | No full Xcode needed |
| Diff display | gpui-component `Diff` | Virtualized, split/unified, tree-sitter highlighting, gutter markers |
| Highlighting | tree-sitter via gpui-kit features | Same grammars as Zed |
| Git | `git` CLI (porcelain v2, `-z`) | Exact parity with terminal git |
| File walk / search | `ignore`, `grep-searcher` | ripgrep's engines |
| Watcher | `notify` (FSEvents) | Debounced refresh of status and tree |
| Fuzzy finder | `nucleo` | Helix's matcher, fast on large repos |
| Markdown | gpui-component `TextView` markdown | Already themed and virtualized |

### Performance budgets (M1, release build)

| Operation | Budget |
|---|---|
| Launch to first frame with repo state | < 300 ms |
| `git status` refresh on a 50k-file repo | < 150 ms, off the main thread |
| Open a 5k-line diff | < 100 ms to first paint |
| Scroll in diff / file view | no dropped frames |
| Fuzzy finder keystroke on 50k paths | < 16 ms |

Zed's own repo (`ref/zed`) is the standing benchmark repo.

## Phases

Each bullet is roughly one commit. A phase ends with something usable.

### Phase 0 — Foundations · status: done
- Plan, agent guide (`CLAUDE.md`), reference checkouts (`script/bootstrap-refs`).
- Cargo workspace, gpui-kit pinned, dark-only window with title bar.

### Phase 1 — Review loop · status: done
- [x] `yin_git`: open repo, parse `git status --porcelain=v2 -z` (branch, ahead/behind,
  staged/unstaged/untracked/conflicted), per-file patches, stage/unstage/commit. Tests.
- [x] Changes panel: grouped file list (Conflicts / Staged / Changes / Untracked),
  checkboxes, keyboard navigation (`j`/`k`, `space`, `enter`, `[`/`]`).
- [x] Diff view: gpui-component `Diff` fed from `git diff`, language from extension,
  untracked files shown as additions, unified/split toggle (`cmd-shift-d`).
- [x] Commit box: message input, commit, amend toggle, `cmd-enter`; hook failures
  shown as notifications.
- [x] Auto-refresh: FSEvents watcher on worktree and git dirs (gitignore-aware,
  debounced), plus refresh on window focus.
- [x] Discard (`cmd-backspace`) behind a confirm dialog; untracked files go to
  the Trash.
- [x] Open a repo: `cmd-o` / File menu folder picker, remembered last repo,
  launch order argument → cwd → last repo → welcome screen.
- [x] Headless UI tests (`gpui-kit` test-support) for the stage → commit flow.

### Phase 2 — Git operations · status: done
- [x] Hunk and line staging (partial patch, `git apply --cached [--reverse]`):
  select lines in the diff, `space` for lines, `s` for the hunk.
- [x] Branches: fuzzy picker (`b`, or click the branch name), switch, create from
  HEAD, check out remote branches as tracking branches.
- [x] Remotes: fetch (`f`), pull fast-forward only (`p`), push (`shift-p`, sets the
  upstream on first push); title-bar buttons with counts; never blocks the UI.
- [x] History tab (`cmd-2`): commit list with refs, commit message and full diff.
- [x] Stash: push (`s`), pop (`shift-s` or row button), drop, view as a diff.
- [x] Conflicts: working file with Accept current / incoming / both, Mark resolved,
  merge banner with Abort.

### Phase 3 — Navigate and read · status: done
- [x] `yin_fs`: gitignore-aware parallel scan, folder tree, rescanned on refresh
  (38 ms for Zed's 4.4k files).
- [x] Files tab (`cmd-3`): tree with git status colours on files and folders.
- [x] Tabs in the main pane; read-only, highlighted, line-numbered file view with
  in-file search; open files reload when changed on disk.
- [x] Fuzzy file finder (`cmd-p`) with nucleo; empty query lists open and changed files.
- [x] Markdown rendered by default, with a Source / Preview toggle (`cmd-shift-v`).
- [x] Project search (`cmd-shift-f`): smart-case literal or regex, grouped
  results, opens at the line.

### Phase 4 — Light editing · status: done
- [x] Files are editable in the gpui-component code editor; opening focuses it.
- [x] Save (`cmd-s`), ● on unsaved tabs, markdown preview follows edits.
- [x] Disk changes reload clean files; files with unsaved edits keep them and
  show a Reload banner.
- [x] Closing a tab, switching repository, quitting and closing the window ask
  before discarding unsaved edits.
- [x] Find in file (`cmd-f`, built into the editor), go to line (`ctrl-g`).
- Known: the first character typed right after `cmd-f` is sometimes dropped
  (gpui-component find bar).

### Phase 5 — Code intelligence · status: done
- [x] `yin_lsp`: JSON-RPC over stdio, answers server requests, finds servers via
  the login shell's PATH. Tested against rust-analyzer.
- [x] One server per language, started when its first file opens
  (rust-analyzer, gopls, typescript-language-server, pyright, clangd); open files
  synced as they are edited.
- [x] Go to definition (`f12`, cmd-click) opening other files in tabs; files
  outside the repo open read-only.
- [x] Hover (gpui-component's popover).
- [x] Find references (`shift-f12`), listed in the Search tab.

### Phase 6 — Agent-era review and polish · status: done
- [x] "Since last review": `r` marks a file reviewed by content hash, `h` hides
  reviewed files; an agent's later edit brings the file back. Saved per repo.
- [x] Worktree picker (`w`): every worktree with its branch and change count;
  picking one switches the window.
- [x] Session restore per repository (open tabs, active tab, sidebar tab and
  visibility, diff mode), saved on every way out. Panel sizes are not kept yet.
- [x] Keymap file `~/.config/yin/keymap.json`; problems shown at launch.
- [x] `script/bundle` builds `yin.app`; `script/install` copies it to
  `~/Applications` and adds a `yin [path]` command in `~/.local/bin`.

## After the plan · status: done
- [x] All-changes review view (`a`): every change against HEAD in one diff.
- [x] Command palette (`cmd-shift-p`).
- [x] Language server diagnostics: squiggles, red files and folders in the tree,
  error and warning counts in the title bar (the status bar is gone).
- [x] Sidebar width and window frame remembered.
- [x] Diverged pull offers a rebase; rejected push offers force-with-lease;
  rebase banner with Continue and Abort; merge commits start from git's message.
- [x] Blame (`cmd-alt-b`) and file history (`cmd-alt-h`).
- [x] Popovers get their own surface colour (the dark theme reused the editor's).
- [x] One review diff: the Changes pane always shows every change against
  HEAD; picking a file scrolls to it, and lines or hunks stage (or unstage, when
  already staged) right there. Conflicts and stashes still show on their own.
- [x] One app, one window per repository: the `yin` command and the Dock
  hand folders to the running app (`open -a`), opening a repository again
  brings its window forward, and windows stay closed without quitting. A
  repository switcher (`cmd-o`, and `cmd-shift-space` system-wide) lists
  recent repositories and the ones under the usual code folders.
- [x] Git bar above the commit box: branch, fetch, pull, push and a menu with
  every other git action. Sidebar tabs restyled (underline, icon and label).
- [x] Commit ranges: shift-select commits in History for their combined diff.
- [x] Branch comparison (`c` in History): the branch's commits since another,
  and its whole change from the merge base.
- [x] Stacks: the branches HEAD is stacked between, above the History list;
  each layer opens its own change. New pull requests target the layer below.
- [x] Review notes on diff lines (`n`, or the + button), kept per repository;
  "Copy as prompt" (`cmd-alt-c`) hands them, with open PR threads, to an agent.
- [x] Pull requests through `gh`: number and open threads in the git bar,
  review threads inline when comparing with the PR's base, and a dialog that
  pushes and opens one. Read-only otherwise: yin posts no comments.

## Known issues (upstream, gpui-component 0.8.1)
- The first character typed right after `cmd-f` is lost. The find bar re-selects
  its query when the search session and panel disagree (`input/overlay.rs`,
  `query_echo`), which races the first keystroke. Forcing redraws doesn't help;
  needs a fix in the component.
- Hover cards draw each markdown code block's language label over the block's
  first line.
- Diff rows keep a stale width after the window grows (worked around in yin by
  `schedule_relayout`).

## Risks

- **GPUI API churn.** Pinned to gpui-kit =0.8.1; upgrade on purpose, one commit.
- **gpui-kit editor limits** (large files, editing ergonomics). Fallback:
  study Zed's `editor` crate and build a narrower read-only view on GPUI text.
- **CLI process cost** on large repos. Measure first; move hot reads to `gix`.

## Decision log

- 2026-10-30 — Native GPUI over web stack, for performance and the Zed feel.
- 2026-10-30 — gpui-kit 0.8.1 (crates.io) instead of depending on Zed's repo:
  stable published snapshot, Apache-2.0, includes the Diff component.
- 2026-10-30 — git CLI for all git operations to start; `gix` only where measured.
- 2026-10-30 — Dark only, Apple Silicon macOS only.
- 2026-10-30 — Rust pinned to 1.98.1 (Zed's toolchain); gpui-pre needs it.
- 2026-10-30 — Untracked files are discarded to the Trash (NSFileManager),
  never deleted outright.
- 2026-10-30 — Workaround for gpui-component Diff keeping a stale width after
  the window grows (`schedule_relayout`); worth reporting upstream.
- 2026-10-30 — Line staging places a replacement where its selected deletion
  was; unselected deletions after it move behind the additions (git-gui's rule).
- 2026-10-30 — Pull is fast-forward only; diverged branches are an error for now.
- 2026-10-31 — Staging from the HEAD diff maps the picked lines through the
  staged and unstaged diffs (`Repository::stage_from_head`): anything unstaged
  stages, otherwise it unstages, as Zed toggles hunks. Replaces the one-file mode.
- 2026-10-31 — One process with many windows, as Zed and JetBrains do, not
  VS Code's process per window: yin runs no third-party code, and language
  servers are already separate processes. The global hotkey is Carbon's
  `RegisterEventHotKey`, which needs no Accessibility permission.
- 2026-10-31 — Refreshes wait for 300 ms of quiet, build the single-file diff
  only for conflicts and stashes, and rescan the tree only when files appear,
  disappear or move. The Diff component compiles each language's highlight
  query on every update; that's upstream, so yin updates it less often. An
  agent writing steadily went from about 32% CPU to 13%.
- 2026-10-31 — GitHub through the `gh` CLI rather than the API directly: it
  owns sign-in and tokens. Lookups run when the branch changes or after two
  minutes, never per file save, and hide quietly without gh.
- 2026-10-31 — Notes are written in a box floating over the diff, not inline:
  the diff binds single letters (space, s, n) that would eat typing.
- 2026-10-30 — Verify UI in the real app: `script/screenshot`, `script/drive`
  (System Events keystrokes) and `screencapture -l`.
