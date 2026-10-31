# Keys

Every action is also in the command palette (`cmd-shift-p`), which shows its
current key.

## Anywhere

| Key | Does |
|---|---|
| `cmd-shift-space`, from any app | Bring yin forward with the repository switcher |
| `cmd-shift-p` | Command palette |
| `cmd-o` | Switch repository: its window if open, else a new one |
| `cmd-shift-o` | Open a folder from the system dialog |
| `cmd-1` / `cmd-2` / `cmd-3` | Files / Changes / History |
| `cmd-p` | Go to file |
| `cmd-shift-f` | Search the project |
| `cmd-enter` | Commit staged changes |
| `cmd-alt-s` / `cmd-alt-u` | Stage all / unstage all |
| `cmd-alt-w` | Switch worktree |
| `cmd-shift-d` | Unified / split diff |
| `cmd-alt-c` | Copy review notes and open PR threads as a prompt |
| `cmd-b` | Show or hide the sidebar |
| `cmd-r` | Refresh |
| `escape` | Back to the sidebar |
| `cmd-q` | Quit (asks first if files have unsaved edits) |

## Changes list

| Key | Does |
|---|---|
| `j` / `k`, `[` / `]` | Move |
| `enter` | Jump to the file in the diff |
| `space` | Stage or unstage the file; mark a conflict resolved |
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
| `space` | Stage the selected lines, or unstage them when they're all staged |
| `s` | The same for their whole hunk |
| `n`, or the + beside a line | Write a review note on the selected lines (`cmd-enter` adds it) |

## History, files and search

| Where | Key | Does |
|---|---|---|
| History | `j` / `k`, `enter` | Move, read the commit |
| History | `shift-j` / `shift-k`, shift-click | Select a run of commits and see their combined diff |
| History | `c` | Compare this branch with another: its commits and whole change |
| History | `cmd-c` | Copy the commit's hash (click the hash to copy it too) |
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
| `cmd-f` in the preview | Find; `enter` / `shift-enter` for next / previous |

## Custom keys

Bindings in `~/.config/yin/keymap.json` apply over the defaults:

```json
[
  { "key": "cmd-k", "action": "yin::ShowHistory" },
  { "key": "ctrl-n", "action": "yin::SelectNext", "context": "Changes" }
]
```

The `Global` context is the system-wide switcher key, which works whichever
app is in front; `"key": "none"` turns it off. macOS lets two apps register
the same combination, so pick one nothing else uses:

```json
[{ "key": "ctrl-alt-y", "action": "yin::SwitchRepository", "context": "Global" }]
```

Actions are named `yin::<Name>`; the names are listed in
[`crates/yin/src/workspace.rs`](../crates/yin/src/workspace.rs) (`Commit`,
`SelectNext`, `ShowHistory`, …). Contexts are `Workspace`, `Changes`,
`History`, `Files`, `SearchResults`, `Diff` and `Input`. Mistakes in the file
show as a notification at launch.
