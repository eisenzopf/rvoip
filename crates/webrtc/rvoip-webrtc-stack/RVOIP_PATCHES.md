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

The source implementation is unchanged. Packaging changes bind its `rtc`
dependency to the attributed `rvoip-rtc` package so crates.io consumers run
the same reviewed RTC code as the rvoip workspace. See `LICENSE-MIT` and
`LICENSE-APACHE`.
