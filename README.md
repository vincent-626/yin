# yin

A fast, native code workspace for reviewing changes, running git and reading
code — built for the way coding agents work.

When an agent writes most of the code, an editor's job shrinks to reviewing what
changed, committing it, and reading around it. yin does that and little else:
it opens on what changed, follows the working tree live while the agent edits,
and keeps git one keystroke away. It is written in Rust on
[GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui), Zed's
GPU-accelerated UI framework, so it starts instantly and scrolls smoothly.

It is dark only (always yin, never yang) and runs on Apple Silicon Macs.

## Features

**Review**
- The working tree's changes, staged, unstaged and untracked, with a
  syntax-highlighted diff that updates as files change on disk.
- Every change in one scrolling diff; picking a file jumps to it.
- Stage or unstage whole files, hunks or individual lines, right in that diff.
- Mark files reviewed and hide them; a file comes back the moment an agent
  touches it again.

**Git**
- Commit and amend; hook output shows when a hook fails.
- Fetch, pull and push. A diverged pull offers to rebase; a rejected push offers
  a force push with lease.
- Switch or create branches, stash, and resolve merge conflicts in the diff.
- History, per-file history, blame, and a picker for worktrees.

**Read and navigate**
- File tree coloured by git status, fuzzy file finder, project search.
- Rendered markdown, light editing with save, find and go to line.
- Go to definition, find references, hover and diagnostics through language
  servers (rust-analyzer, gopls, typescript-language-server, pyright, clangd),
  started automatically when installed.

**Everywhere**
- Keyboard first, with a command palette (`cmd-shift-p`) listing every action
  and its key. Keys are customisable.
- Each repository reopens with its tabs and layout.

## Install

On an Apple Silicon Mac with macOS 13 or later and git:

```bash
curl -fsSL https://raw.githubusercontent.com/vincent-626/yin/main/script/install-release | bash
```

This installs `yin.app` into `~/Applications` and a `yin` command into
`~/.local/bin`. Then open a repository:

```bash
yin ~/Code/some-repo     # or just `yin` inside one
```

The app is not notarized, so the installer verifies the download's checksum
and clears the quarantine flag that would otherwise stop macOS opening it. To
build from source instead, see [docs/development.md](docs/development.md).

Language features need the server for each language on your `PATH`; yin uses
them when it finds them and says so when it doesn't.

## Using it

yin opens on the **Changes** tab. Move with `j`/`k`, stage with `space`, press
`c` to write a message and `cmd-enter` to commit. Every git action is in the
bar above the commit box. `cmd-1` to `cmd-3` switch between Files, Changes and History;
`cmd-p` finds a file and `cmd-shift-f` searches the project.

Everything else is in the command palette (`cmd-shift-p`) and in
[docs/keys.md](docs/keys.md), along with how to rebind keys.

## Status

yin is a personal tool, shared as it is. It covers what its author needs every
day and deliberately leaves out the rest: there is no terminal, debugger,
extension system, light theme, or support for Intel Macs, Linux or Windows.

Known issues come from the UI component library yin builds on and are listed in
[docs/PLAN.md](docs/PLAN.md#known-issues-upstream-gpui-component-081), which
also records how yin was planned and built.

## Built with

- [GPUI](https://github.com/zed-industries/zed) from Zed, via
  [gpui-kit and gpui-component](https://github.com/longbridge/gpui-component)
- The `git` command line for every git operation, so hooks, signing and
  credentials behave as they do in a terminal
- [ignore](https://crates.io/crates/ignore) and
  [grep-searcher](https://crates.io/crates/grep-searcher) from ripgrep,
  [nucleo](https://github.com/helix-editor/nucleo) from Helix,
  [tree-sitter](https://tree-sitter.github.io) and
  [Lucide](https://lucide.dev) icons

## License

[MIT](LICENSE)
