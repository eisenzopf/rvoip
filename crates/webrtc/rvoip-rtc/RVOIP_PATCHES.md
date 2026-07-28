# rvoip `rtc` patches

This package forks the published `rtc` 0.20.0-alpha.1 crate under the registry
identity `rvoip-rtc` so local and crates.io builds use the same reviewed code.
The Rust library name remains `rtc`. The complete upstream MIT and Apache-2.0
license texts are included as `LICENSE-MIT` and `LICENSE-APACHE`.

Source baseline:

- crates.io `rtc` 0.20.0-alpha.1
- upstream source commit `b808b74f712ed379312a114b848ede133880d58a`

Applied fixes:

1. `eisenzopf/rtc@1e5b7d4be6d94850694f2519f4c235d16c871d53`
   opens DataChannels created after the SCTP handshake and preserves DCEP
   partial-reliability parameters. A 32-bit DCEP reliability value that cannot
   fit the WebRTC API's 16-bit field is reported through rtc-shared's existing
   `OtherDataChannelErr`, keeping one exact upstream `rtc-shared` type across
   every RTC subcrate.
2. `webrtc-rs/rtc@abe3c968a977af3b7c2691136bea7d18b3ed3e84`
   prevents a re-offer from emitting a second media section for a MID already
   matched from the remote description.
3. `EndpointHandler` accepts an undeclared supplemental SSRC when its packet
   carries an exact negotiated MID and payload type, without requiring a RID.
   This preserves the primary coding and gives RFC 4733's separately-clocked
   telephone-event SSRC one deterministic RFC 8843 BUNDLE binding.

The offer fix is applied inside `generate_matched_sdp`, where offer media
sections are owned. The supplemental-SSRC fix is applied at the receive
endpoint's MID/PT demultiplexing boundary. rvoip does not rewrite generated
SDP or add a parallel renegotiation/signaling path.

Original authorship belongs to Rain Liu and the WebRTC.rs contributors. rvoip
changes are documented above and in the retained source history. Remove this
fork only after one immutable upstream release or commit contains all three
behaviors and passes the full rvoip beta gate.
