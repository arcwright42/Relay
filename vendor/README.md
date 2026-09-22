# Reviewed dependency patches

## gpui-pre 0.3.6 — Inspector pointer routing

- Source: the published [gpui-pre 0.3.6 crate](https://crates.io/crates/gpui-pre/0.3.6).
- Original crate SHA-256: `a0437c0b83e636a92bd1a39fa1d05fb632ae671289537497b35871ffbe231b84`.
- Upstream Zed revision: `bcf6582ce3500df93a8a39366640173e6786cea6`.
- License: Apache-2.0, retained in `gpui-pre/LICENSE-APACHE`.
- All published files are retained except Cargo's cache marker `.cargo-ok` and the dependency's own `Cargo.lock`. Relay uses the root lockfile.
- Relay modifications are limited to `src/window.rs`, recorded in [the patch](gpui-pre-inspector.patch).

While the Inspector is picking, upstream intercepts every mouse event before widget listeners run. This also swallows clicks on the Inspector's own close and pick buttons. Relay limits picking interception to the application content region; the Inspector pane receives ordinary pointer events. The boundary follows the same `30rem` width used to lay out the pane.

The patch is selected by the root `[patch.crates-io]`, with no changes to application dependency versions or the user's Cargo cache. The vendored package is excluded from Relay workspace members. Its dependencies remain locked in the root `Cargo.lock`.

Regression coverage: `crates/relay-ui/tests/inspector.rs` dispatches real GPUI pointer events to verify closing during picking and after selection, reopening, suppression of application clicks while selecting, and restoration of normal input after closing. It runs as part of `cargo xtask verify`.

When upgrading GPUI Kit, check whether its matching GPUI version fixes this routing. If it does, remove this patch and vendor directory, update the lockfile, and retain the regression tests. Do not update vendored code independently of the matching Kit release.
