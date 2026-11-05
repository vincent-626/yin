# Vendored code

## gpui-component

`gpui-component/` is [gpui-component](https://github.com/longbridge/gpui-component)
0.8.1, as published on crates.io, under the Apache License 2.0
(`gpui-component/LICENSE-APACHE`). The workspace's `[patch.crates-io]` builds
yin against this copy instead of the published crate.

yin changes it where the published crate can't do what yin needs. Every file
yin changed starts with a comment saying so, and each change is listed here:

- **Pinned file headers** (`src/diff/mod.rs`, `src/diff/state.rs`): while a
  file's lines scroll under the top of the diff, its header stays pinned
  there, and the next file's header pushes it out. The header's row is
  split out as `file_header_bar` so the pinned copy reuses it.
- **The + button for notes** (`src/diff/mod.rs`, `code_line`): drawn over the
  line number, as on GitHub, instead of over the start of the code.
- **Find bar keeps typed letters** (`src/input/overlay.rs`, `sync`): typing
  moved the search anchor, which the panel took for a new `cmd-f` and so
  selected its query again, and the next letter replaced it. Only
  `activation_revision`, which every `cmd-f` advances, now counts.

To move to a newer gpui-component: copy the new release's crate source here,
then apply the changes above again (`git log -p -- vendor/gpui-component`
shows each one).
