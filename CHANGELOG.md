# Changelog

What changed in each release. Unreleased changes collect at the top until the
next version.

## Unreleased

## 0.4.2 — 2026-10-31

- Review notes follow their code as an agent edits: a note moves with its
  lines, and when they change or disappear it's marked outdated, shown at the
  top of its file with the code it was about, and flagged in the prompt. Notes
  written on commits in History show wherever their code appears there.

## 0.4.1 — 2026-10-31

- A changelog, used for the release notes.
- Fixed: the sidebar came back wider after `cmd-b` hid and showed it.
- The stack above History shows each layer's commit count dimmed, apart from
  the branch name.

## 0.4.0 — 2026-10-31

- **Commit ranges.** Shift-click or shift-↑↓ in History selects a run of
  commits and shows what they changed together.
- **Branch comparison.** `c` in History (or "Compare with branch…" in the git
  bar's menu) lists only this branch's commits since another, and its whole
  change from where they split.
- **Stacks.** When the branch sits on other branches, the stack shows above the
  History list; each layer opens its own change.
- **Review notes.** Press `n` on selected diff lines, or the + beside a line, to
  leave a note. Notes show inline, stay on your Mac, and "Copy as prompt"
  (`cmd-alt-c`) hands them all to an agent at once.
- **Pull requests** through the GitHub CLI: the branch's PR and its open
  threads in the git bar, its review threads inline in its diff, and a dialog
  that pushes and opens a new one. A stacked branch targets the branch below.
  yin never posts comments.
- **Find in the markdown preview** with `cmd-f`.
- Fixed: `yin.app/Contents/MacOS/yin <repo>`, run directly, opened no window.

## 0.3.0 — 2026-10-31

- **One app, one window per repository.** The `yin` command and the Dock hand
  folders to the running app; opening a repository again brings its window
  forward, and closing the last window keeps yin running.
- **Repository switcher** (`cmd-o`), and `cmd-shift-space` from any app: recent
  repositories and the ones in your code folders.
- Lighter while an agent writes: refreshes wait for 300 ms of quiet and skip
  work nothing shows. CPU during steady writing went from about 32% to 13%.

## 0.2.1 — 2026-10-31

- The status bar is gone; errors and diagnostics counts sit in the title bar.
- The first window fits the screen instead of opening at a fixed size.

## 0.2.0 — 2026-10-31

- **One review diff.** The Changes tab always shows every change in one
  scrolling diff; picking a file jumps to it, and lines or hunks stage (or
  unstage) right there.
- **Git bar** above the commit box with the branch, fetch, pull, push and a
  menu holding every other git action.
- Files tab first; sidebar tabs with icons.
- Quieter commit hashes, copyable commit messages and hashes.

## 0.1.1 — 2026-10-30

- Third-party licence notices ship with the app.

## 0.1.0 — 2026-10-30

First release.

- Changes, staged and unstaged, with a live syntax-highlighted diff; stage
  files, hunks or lines; commit and amend.
- Fetch, pull and push; branches, stashes, merge conflicts, rebases.
- History, file history and blame; worktree picker.
- File tree, fuzzy file finder, project search, rendered markdown, light
  editing.
- Go to definition, references, hover and diagnostics through language servers.
- "Since last review" marks, a command palette, and keys you can rebind.
