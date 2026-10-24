# Vendored format dependencies

Source: kolto-labs/kotor-formats commit
`f223b4f573e3b72a25d2ec5d606e101ddbb875ad`.

The upstream repository remains private. This snapshot permits public kq
builds without credentials. The upstream MIT license is retained in LICENSE.

Local adjustments: standalone package manifests, rustfmt normalization in
NCS decode/tests, and removal of upstream workflow/release automation.
These crates are excluded from the kq workspace; they remain normal path
dependencies. No blanket lint suppression is applied.

To update, replace this snapshot with a reviewed upstream commit, retain
this provenance and license, and run the workspace tests, clippy, formatting,
and Rust 1.82 build with the checked-in lockfile.
