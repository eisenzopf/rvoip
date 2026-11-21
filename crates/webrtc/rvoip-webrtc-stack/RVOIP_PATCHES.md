# rvoip WebRTC stack packaging

This crate packages the published `webrtc` 0.20.0-alpha.1 source under the
registry identity `rvoip-webrtc-stack`. Its Rust library name remains
`webrtc`, so rvoip's public adapter code does not change.

Source baseline:

- Upstream project: <https://github.com/webrtc-rs/webrtc>
- crates.io package: `webrtc` 0.20.0-alpha.1
- upstream source commit: `b899593a5c525e88098ce9f5326fe29b4478832d`
- license: MIT OR Apache-2.0
- original authors: Rain Liu and the WebRTC.rs contributors

Packaging changes bind its `rtc` dependency to the attributed `rvoip-rtc`
package so crates.io consumers run the same reviewed RTC code as the rvoip
workspace. See `LICENSE-MIT` and `LICENSE-APACHE`.

## rvoip 0.3.5 runtime policy

rvoip qualifies Tokio as this async integration crate's sole runtime. The
experimental Smol adapter, its public feature and helper API, and its optional
dependencies were removed. Tokio is mandatory; the `runtime-tokio` feature
name remains as a source-compatible no-op for existing Tokio manifests.

The underlying `rvoip-rtc` protocol crate remains Sans-I/O and can still be
driven by applications using other executors.
