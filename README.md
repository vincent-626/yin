# yin

A native, dark-only code workspace for reviewing changes, running git, and reading code. Built on GPUI.

See [docs/PLAN.md](docs/PLAN.md).

```bash
script/install          # builds yin.app into ~/Applications and a `yin` command
yin ~/Code/some-repo    # or `yin` inside a repository
```

For development: `cargo run -- /path/to/repo`, and optionally
`script/bootstrap-refs` for the reference checkouts agents read.

## Keys

| Where | Key | Does |
|---|---|---|
| Anywhere | `cmd-o` | Open a repository |
| | `cmd-1` / `cmd-2` / `cmd-3` | Changes / History / Files |
| | `cmd-p` | Go to file |
| | `cmd-shift-f` | Search the project |
| | `cmd-w` | Close file tab |
| | `cmd-shift-v` | Markdown preview / source |
| | `cmd-r` | Refresh |
| | `cmd-b` | Toggle sidebar |
| | `cmd-shift-d` | Unified / split diff |
| | `cmd-enter` | Commit staged changes |
| | `cmd-alt-s` / `cmd-alt-u` | Stage all / unstage all |
| | `[` / `]` | Previous / next file, commit, tab or result |
| | `escape` | Back to the sidebar |
| Changes list | `j` / `k` | Move |
| | `space` | Stage / unstage file; mark a conflict resolved |
| | `enter` | Read the diff |
| | `c` | Write the commit message |
| | `cmd-backspace` | Discard file (untracked: to Trash) / drop stash |
| | `b` | Switch or create branch |
| | `f` / `p` / `shift-p` | Fetch / pull / push |
| | `s` / `shift-s` | Stash all / pop stash |
| | `r` / `shift-r` | Mark file / everything reviewed (unmarked again when it changes) |
| | `h` | Hide or show reviewed files |
| | `w` (or `cmd-alt-w`) | Switch worktree |
| | `a` | Review all changes in one diff / one file at a time |
| Diff | `shift-up` / `shift-down` | Select lines |
| | `space` / `s` | Stage (or unstage) selected lines / their hunk |
| History list | `j` / `k`, `enter` | Move, read the diff |
| File tree | arrows, `enter` | Move, open/close folders, open file |
| Search results | `j` / `k`, `enter` | Move, open at the line |
| File view | `cmd-s` | Save |
| | `cmd-f` / `cmd-alt-f` | Find / replace in file |
| | `ctrl-g` | Go to line (`line` or `line:column`) |
| | `f12` / cmd-click | Go to definition |
| | `shift-f12` | Find references |
| | hover | Type and docs (needs the language server installed) |

## Custom keys

Bindings in `~/.config/yin/keymap.json` apply over the defaults. Actions are
named `yin::<Name>`, as in `crates/yin/src/workspace.rs` (`Commit`,
`SelectNext`, `ShowHistory`, …); contexts are `Workspace`, `Changes`,
`History`, `Files`, `SearchResults`, `Diff` and `Input`. Problems show as a
notification at launch.

```json
[
  { "key": "cmd-k", "action": "yin::ShowHistory" },
  { "key": "ctrl-n", "action": "yin::SelectNext", "context": "Changes" }
]
```

Open tabs, the sidebar tab and the diff mode are restored per repository.
