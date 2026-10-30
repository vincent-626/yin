# Keys

Every action is also in the command palette (`cmd-shift-p`), which shows its
current key.

## Anywhere

| Key | Does |
|---|---|
| `cmd-shift-p` | Command palette |
| `cmd-o` | Open a repository |
| `cmd-1` / `cmd-2` / `cmd-3` | Changes / History / Files |
| `cmd-p` | Go to file |
| `cmd-shift-f` | Search the project |
| `cmd-enter` | Commit staged changes |
| `cmd-alt-s` / `cmd-alt-u` | Stage all / unstage all |
| `cmd-alt-w` | Switch worktree |
| `cmd-shift-d` | Unified / split diff |
| `cmd-b` | Show or hide the sidebar |
| `cmd-r` | Refresh |
| `escape` | Back to the sidebar |
| `cmd-q` | Quit (asks first if files have unsaved edits) |

## Changes list

| Key | Does |
|---|---|
| `j` / `k`, `[` / `]` | Move |
| `enter` | Read the diff |
| `space` | Stage or unstage the file; mark a conflict resolved |
| `a` | Every change in one diff / one file at a time |
| `r` / `shift-r` | Mark the file / everything reviewed |
| `h` | Hide or show reviewed files |
| `c` | Write the commit message |
| `cmd-backspace` | Discard the file's changes (untracked files go to the Trash), or drop a stash |
| `b` | Switch or create a branch |
| `w` | Switch worktree |
| `f` / `p` / `shift-p` | Fetch / pull / push |
| `s` / `shift-s` | Stash everything / pop a stash |

## Diff

| Key | Does |
|---|---|
| `shift-up` / `shift-down`, or drag line numbers | Select lines |
| `space` | Stage (or in Staged, unstage) the selected lines |
| `s` | Stage (or unstage) their whole hunk |

## History, files and search

| Where | Key | Does |
|---|---|---|
| History | `j` / `k`, `enter` | Move, read the commit |
| File tree | arrows, `enter` | Move, open and close folders, open a file |
| Search results | `j` / `k`, `enter` | Move, open at the line |

## Open files

| Key | Does |
|---|---|
| `cmd-s` | Save |
| `cmd-w` | Close the tab |
| `cmd-f` / `cmd-alt-f` | Find / replace |
| `ctrl-g` | Go to line (`line` or `line:column`) |
| `f12`, or cmd-click | Go to definition |
| `shift-f12` | Find references |
| hover | Type and documentation |
| `cmd-alt-b` | Blame (click a commit to open it) |
| `cmd-alt-h` | History of this file |
| `cmd-shift-v` | Markdown preview / source |

## Custom keys

Bindings in `~/.config/yin/keymap.json` apply over the defaults:

```json
[
  { "key": "cmd-k", "action": "yin::ShowHistory" },
  { "key": "ctrl-n", "action": "yin::SelectNext", "context": "Changes" }
]
```

Actions are named `yin::<Name>`; the names are listed in
[`crates/yin/src/workspace.rs`](../crates/yin/src/workspace.rs) (`Commit`,
`SelectNext`, `ShowHistory`, …). Contexts are `Workspace`, `Changes`,
`History`, `Files`, `SearchResults`, `Diff` and `Input`. Mistakes in the file
show as a notification at launch.
