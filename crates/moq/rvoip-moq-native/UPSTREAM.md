# Upstream and attribution

This crate is an rvoip-owned package of the `moq-native-ietf` fork used by the
rvoip release. The Rust library name remains `moq_native_ietf`; the crates.io
package name is `rvoip-moq-native` so the exact fork is part of the published
dependency graph.

- Upstream project: `moq-rs`
- Upstream repositories: <https://github.com/cloudflare/moq-rs> and
  <https://github.com/englishm/moq-rs>
- Qualified fork: <https://github.com/eisenzopf/moq-rs>
- Imported revision: `ef52ac8656513bb3b07b4b9b80152ac24bb2467e`
- Imported path: `moq-native-ietf`
- License: MIT OR Apache-2.0
- Copyright: Cloudflare Inc., Luke Curley, Mike English, and contributors, as
  retained in the source SPDX headers

rvoip changed the package identity and bound its transport dependency to the
paired `rvoip-moq-transport` package. Upstream authorship and source copyright
remain intact.

See `LICENSE-MIT` and `LICENSE-APACHE` for the complete license texts.
