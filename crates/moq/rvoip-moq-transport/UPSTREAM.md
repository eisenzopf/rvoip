# Upstream and attribution

This crate is an rvoip-owned package of the `moq-transport` fork used by the
rvoip release. The Rust library name remains `moq_transport`; the crates.io
package name is `rvoip-moq-transport` so downstream builds cannot silently
substitute a different upstream revision.

- Upstream project: `moq-rs`
- Upstream repositories: <https://github.com/cloudflare/moq-rs> and
  <https://github.com/englishm/moq-rs>
- Qualified fork: <https://github.com/eisenzopf/moq-rs>
- Imported revision: `ef52ac8656513bb3b07b4b9b80152ac24bb2467e`
- Imported path: `moq-transport`
- License: MIT OR Apache-2.0
- Copyright: Cloudflare Inc., Luke Curley, Mike English, and contributors, as
  retained in the source SPDX headers

The imported revision contains the draft-19 transport, bounded request and
retention behavior, namespace discovery, authorization parsing, and lifecycle
fixes exercised by rvoip. rvoip changed the package identity and dependency
plumbing; upstream authorship and source copyright remain intact.

See `LICENSE-MIT` and `LICENSE-APACHE` for the complete license texts.
