# rvoip `rtc` patches

This directory vendors the published `rtc` 0.20.0-alpha.1 crate so the beta
build is reproducible with four reviewed fixes that are not available
together in one upstream revision.

Source baseline:

- crates.io `rtc` 0.20.0-alpha.1
- upstream source commit `b808b74f712ed379312a114b848ede133880d58a`

Applied fixes:

1. `eisenzopf/rtc@1e5b7d4be6d94850694f2519f4c235d16c871d53`
   opens DataChannels created after the SCTP handshake and preserves DCEP
   partial-reliability parameters. The accompanying `rtc-shared` change is in
   `../rtc-shared`.
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

Remove these path patches only after one immutable upstream release or commit
contains all three behaviors and passes the full rvoip beta gate.
