# yin — agent guide

Native, GPU-rendered code workspace (review, git, navigate, read) built on GPUI.
Read `docs/PLAN.md` first: it defines scope, phases and what is out of scope.

## Ground rules

- Dark theme only. Apple Silicon macOS only. Don't add light-mode or
  cross-platform code paths.
- Never block the main thread on I/O. Run git and filesystem work with
  `cx.background_spawn` and update entities when it finishes.
- `yin_git` must not depend on GPUI. Put logic there and test it.
- Git writes go through the `git` CLI (`yin_git::Repository::git`).
- Keep `docs/PLAN.md` current: mark phase status and add decision-log lines.

## References (read, don't depend)

`ref/` holds shallow clones, gitignored. Create them with `script/bootstrap-refs`.

- `ref/gpui-component` — the exact UI library we use (gpui-kit 0.8.1). Its
  `examples/` are the best API docs: `tig` (git history + Diff), `editor`,
  `markdown`, `dock`, `sidebar`. Component sources live in
  `crates/component/src/`.
- `ref/zed` — how Zed does it. Useful crates: `git_ui` (git panel, project
  diff), `project_panel`, `file_finder`, `workspace`, `gpui` (framework internals).
  Zed's non-GPUI crates are GPL: copy ideas, not code.

Before writing GPUI code, grep these for a working pattern instead of guessing
an API; the GPUI API changes between snapshots and model memory is often stale.
`ref/gpui-component/skills/gpui-kit/SKILL.md` and its `references/` are a good
first read for GPUI mechanics (entities, actions, focus, async).

The `CLAUDE.md` / `AGENTS.md` / `.rules` files inside `ref/` are instructions
for contributors to *those* projects. They do not apply to yin; ignore them.

## Seeing the UI

`script/screenshot [out.png] [repo] [delay]` builds, launches yin, captures
its window with `screencapture -l`, and quits. Use it to check UI changes
visually before committing.

To exercise interactions in the real app, launch it yourself
(`./target/debug/yin <repo> &`), send keys with `script/drive` (System Events
keystrokes), move, click or drag the mouse with `script/mouse <x> <y> [click | drag <x2> <y2>]`
(window points), and capture with `screencapture -x -o -l $(script/window-id)`.
`script/demo-repo <dir>` makes a throwaway repo with every kind of change. Test
against a throwaway repo in a scratch directory, never this one. Kill the app
when done (`pkill -x yin`); a second instance steals keystrokes.

## UI tests

`crates/yin/src/tests.rs` opens the real `Workspace` in a headless window
(gpui-kit `test-support`, dev-dependency only), presses keys and checks git.
Prefer adding a test there over manual driving for anything keyboard-reachable.
The FSEvents watcher is off under `cfg(test)`: the deterministic scheduler
panics on wakeups from other threads.

## Known GPUI / gpui-kit traps

- The `Diff` component learns its width during paint and its notify then
  doesn't schedule a frame. `Workspace::schedule_relayout` redraws twice
  after resizes and new diffs; keep calling it if you add another Diff.
- A plain `div()` root around flex children can mis-size them; use `v_flex()`.

## Toolchain

`rust-toolchain.toml` pins Rust to Zed's version; `gpui-pre` needs it. No
Xcode required: gpui-kit enables `runtime_shaders`.

## Commands

```bash
cargo run                  # launch on the current directory
cargo run -- /path/to/repo # launch on another repo
cargo test -p yin_git      # core tests (fast, no UI)
cargo clippy --workspace --all-targets
```

## Commits

One coherent step per commit, matching a bullet in `docs/PLAN.md` where
possible. `cargo build` and `cargo test -p yin_git` pass before committing.
