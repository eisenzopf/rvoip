# Upstream and attribution

This crate is an rvoip-owned package of the embeddable `moq-relay-ietf` fork
used by the rvoip release. The Rust library name remains `moq_relay_ietf`; the
crates.io package name is `rvoip-moq-relay`.

- Upstream project: `moq-rs`
- Upstream repositories: <https://github.com/cloudflare/moq-rs> and
  <https://github.com/englishm/moq-rs>
- Qualified fork: <https://github.com/eisenzopf/moq-rs>
- Imported revision: `ef52ac8656513bb3b07b4b9b80152ac24bb2467e`
- Imported path: `moq-relay-ietf`
- License: MIT OR Apache-2.0
- Copyright: Cloudflare Inc., Luke Curley, Mike English, and contributors, as
  retained in the source SPDX headers

The fork includes the admission, capacity, lifecycle, namespace, diagnostic,
and upstream mTLS behavior used by rvoip. rvoip changed the package identity,
bound it to the paired rvoip MOQT packages, and made the embeddable feature set
opt-in by default. The unrelated standalone admin-server binary and its
outdated Hyper integration are not package targets; rvoip ships and verifies
the embeddable relay/admission runtime. Upstream authorship and source
copyright remain intact.

See `LICENSE-MIT` and `LICENSE-APACHE` for the complete license texts.
