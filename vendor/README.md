# Vendored code

## gpui-component

`gpui-component/` is [gpui-component](https://github.com/longbridge/gpui-component)
0.8.1, as published on crates.io, under the Apache License 2.0
(`gpui-component/LICENSE-APACHE`). The workspace's `[patch.crates-io]` builds
yin against this copy instead of the published crate.

yin changes it where the published crate can't do what yin needs. Every file
yin changed starts with a comment saying so, and each change is listed here:

(none yet)

To move to a newer gpui-component: copy the new release's crate source here,
then apply the changes above again (`git log -p -- vendor/gpui-component`
shows each one).
