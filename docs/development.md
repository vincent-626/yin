# Development

## Building

The toolchain is pinned in `rust-toolchain.toml`; rustup installs it. No Xcode
is needed.

```bash
cargo run -- /path/to/repo     # debug build, dependencies optimized
cargo test --workspace          # unit, git and headless UI tests
script/install                  # release build into ~/Applications + `yin`
```

## Layout

| Crate | What it holds |
|---|---|
| `yin` | The app: window, workspace and one module per feature |
| `yin_git` | Git through the CLI: status, diffs, partial patches, branches, remotes, history, stash, conflicts |
| `yin_fs` | Working tree scan, folder tree, fuzzy matching, project search |
| `yin_lsp` | A minimal language server client |

[PLAN.md](PLAN.md) has the scope, the phases it was built in, decisions and
known issues. [CLAUDE.md](../CLAUDE.md) is the guide for coding agents working
on yin.

## Seeing changes in the real app

These drive the app on macOS and need Screen Recording and Accessibility
permission for the terminal:

| Script | Does |
|---|---|
| `script/demo-repo <dir>` | A throwaway repo with every kind of change and a bare remote |
| `script/screenshot [out] [repo]` | Launches yin and captures its window |
| `script/drive '<System Events command>' …` | Sends keystrokes |
| `script/mouse <x> <y> [click \| drag <x2> <y2>]` | Moves, clicks or drags |
| `script/window-id` | yin's window number, for `screencapture -l` |

Drive keystrokes only in throwaway repositories: single keys like `s` stash and
`cmd-backspace` discards.

## References

`script/bootstrap-refs` clones Zed and gpui-component into `ref/` (ignored by
git) to read when working with GPUI.

## Releasing

CI runs formatting, clippy and the tests on macOS for code pushes and pull
requests. To release, bump `version` in `crates/yin/Cargo.toml`, commit, and
push a matching tag:

```bash
git tag v0.2.0 && git push origin v0.2.0
```

The release workflow builds `yin.app`, and attaches `yin-macos-arm64.zip` and
its checksum to a GitHub release, which `script/install-release` installs.
Releases are ad-hoc signed; notarizing them would need an Apple Developer ID
and `notarytool` in the workflow.

`script/make-icon` redraws the app icon (`assets/icon.png`, `assets/yin.icns`).
