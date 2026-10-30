# yin

A native, dark-only code workspace for reviewing changes, running git, and reading code. Built on GPUI.

See [docs/PLAN.md](docs/PLAN.md).

```bash
script/bootstrap-refs   # optional: reference checkouts for agents
cargo run -- /path/to/repo
```

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
| Diff | `shift-up` / `shift-down` | Select lines |
| | `space` / `s` | Stage (or unstage) selected lines / their hunk |
| History list | `j` / `k`, `enter` | Move, read the diff |
| File tree | arrows, `enter` | Move, open/close folders, open file |
| Search results | `j` / `k`, `enter` | Move, open at the line |
| File view | `cmd-f` | Find in file |
