# Third-party source notices

The rvoip workspace contains attributed forks where a reviewed behavior is not
available from one suitable upstream release. Package names use the `rvoip-`
prefix so crates.io consumers receive the same code that the workspace tests.

## MOQT / moq-rs

`rvoip-moq-transport`, `rvoip-moq-native`, and `rvoip-moq-relay` are derived
from `moq-rs` at revision
`ef52ac8656513bb3b07b4b9b80152ac24bb2467e`. Upstream projects and authors
include Cloudflare Inc., Luke Curley, Mike English, and other contributors.
The source is licensed MIT OR Apache-2.0. Each package contains the complete
license texts, retained SPDX copyright headers, and an `UPSTREAM.md` provenance
record.

Upstream: <https://github.com/cloudflare/moq-rs> and
<https://github.com/englishm/moq-rs>. Qualified fork:
<https://github.com/eisenzopf/moq-rs>.

## WebRTC.rs

`rvoip-rtc` derives from WebRTC.rs `rtc` 0.20.0-alpha.1 at commit
`b808b74f712ed379312a114b848ede133880d58a`, with the reviewed changes listed
in its `RVOIP_PATCHES.md`.
`rvoip-webrtc-stack` derives from WebRTC.rs `webrtc` 0.20.0-alpha.1 at commit
`b899593a5c525e88098ce9f5326fe29b4478832d` and binds that attributed RTC
fork. Original authors include Rain Liu and the WebRTC.rs contributors.

These sources are licensed MIT OR Apache-2.0. Each package contains the
complete license texts and a provenance record.

Upstream: <https://github.com/webrtc-rs/rtc> and
<https://github.com/webrtc-rs/webrtc>. Qualified RTC fork:
<https://github.com/eisenzopf/rtc>.
