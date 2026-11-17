# rvoip `rtc` patches

This package forks the published `rtc` 0.21.1 crate under the registry
identity `rvoip-rtc` so local and crates.io builds use the same reviewed code.
The Rust library name remains `rtc`. The complete upstream MIT and Apache-2.0
license texts are included as `LICENSE-MIT` and `LICENSE-APACHE`.

Source baseline:

- crates.io `rtc` 0.21.1
- upstream source commit `2a732df55ca4bcf26b80914fa0f570cfaf74cfcb`

The fork was rebased from `rtc` 0.20.0-alpha.1 (upstream source commit
`b808b74f712ed379312a114b848ede133880d58a`) onto 0.21.1. Its `rtc-*`
dependencies are pinned to the exact 0.21.1 releases that `rtc` 0.21.1 was
published against.

Applied fixes:

1. DCEP partial-reliability parameters (from
   `eisenzopf/rtc@1e5b7d4be6d94850694f2519f4c235d16c871d53`). A DataChannel
   accepted from the peer keeps the `maxRetransmits` or `maxPacketLifeTime`
   its `DATA_CHANNEL_OPEN` announced instead of reporting a reliable channel.
   A 32-bit DCEP reliability value that cannot fit the WebRTC API's 16-bit
   field is reported through rtc-shared's existing `OtherDataChannelErr`,
   keeping one exact upstream `rtc-shared` type across every RTC subcrate.
   Applied in `RTCDataChannelInternal::accept`.
2. `EndpointHandler` accepts an undeclared supplemental SSRC when its packet
   carries an exact negotiated MID and payload type, without requiring a RID.
   This preserves the primary coding and gives RFC 4733's separately-clocked
   telephone-event SSRC one deterministic RFC 8843 BUNDLE binding. Upstream
   0.21 moved stream establishment from the endpoint handler into the
   interceptor handler (`InterceptorHandler::ensure_remote_stream_bound`), so
   the fix now lives there: the MID is read even when no RID extension is
   negotiated, an RTP packet without RID/RRID goes to the undeclared-SSRC
   binding, and that binding appends a coding instead of replacing the
   receiver's declared codings.
3. The BridgeFu exact-Chromium working-tree candidate was based on
   `eisenzopf/rtc@1e5b7d4be6d94850694f2519f4c235d16c871d53`
   (working-tree `patch-id 478b7da63ea6d195f446a9abce4c56e62129a86e`).
   The reviewed six-file RTC subset was extracted from historical rvoip commit
   `ace18056` (`patch-id 04f06567b162464eaf4185bfa3f5d037bec603a7`).
   Unrelated `rvoip-webrtc` DTMF changes are not part of it. The integration
   distinguishes codec-specific audio bindings from RID simulcast, advertises
   only the primary audio SSRC, never emits an empty `a=simulcast` attribute,
   selects supplemental payload types by complete codec identity, groups
   declared SSRCs onto one receiver track, and admits an un-signaled
   supplemental SSRC only through an authoritative MID/payload binding or a
   uniquely negotiated audio payload type. The authoritative-MID path remains
   media-kind neutral, and the established sole-video/no-extension fallback is
   preserved.
4. `RTCRtpSender::write_rtp` binds each outbound packet to the codec of the
   encoding that owns its SSRC, as `rtc` 0.20 did, and keeps an explicitly
   requested, negotiated RFC 4733 telephone-event payload type on that SSRC.
   Upstream 0.21 instead sends the caller's payload type unchanged and rejects
   one the sender did not negotiate; rvoip keeps the binding so a payload type
   the remote numbered differently is still sent with the negotiated value.
5. `add_track`, `add_transceiver_from_track` and `add_transceiver_from_kind`
   keep the `rtc` 0.20 sender semantics rvoip's media path is built on.
   Upstream 0.21 narrows the transceiver's codec preferences to the codecs
   named by the track's encodings and truncates every non-RID track to one
   encoding. rvoip still validates the track's codecs but does not narrow the
   preferences, so offers and answers carry every codec the `MediaEngine`
   registered (alternative audio codecs, RFC 4733 telephone-event, further
   video codecs), and it keeps an audio track's codec-distinct supplemental
   encodings (patch 3). An application that wants a narrowed offer calls
   `RTCRtpTransceiver::set_codec_preferences`; upstream's
   `rtx_codec_for_an_unoffered_primary_is_not_offered` test does so.

The supplemental-SSRC fix is applied at the receive path's MID/PT
demultiplexing boundary. rvoip does not rewrite generated SDP or add a
parallel renegotiation/signaling path.

Fixes carried by earlier revisions of this fork that upstream 0.21.1 now
contains, and which are therefore no longer patched:

- DataChannels created after the SCTP handshake are dialed immediately
  (`RTCPeerConnection::create_data_channel`), the rest of
  `eisenzopf/rtc@1e5b7d4be6d94850694f2519f4c235d16c871d53`.
- `webrtc-rs/rtc@abe3c968a977af3b7c2691136bea7d18b3ed3e84`: a re-offer does
  not emit a second media section for a MID already matched from the remote
  description.
- An answer keeps the offer's codec preference order (webrtc-rs/rtc#213). The
  rvoip regression test `tests/answer_codec_preference_order.rs` is kept.
- The fork no longer installs a process-global rustls crypto provider when a
  peer connection is built: 0.21 selects the provider per peer connection
  (`SettingEngine::set_crypto_provider`) and hands DTLS an explicit rustls
  provider, so no global installation is needed.

Original authorship belongs to Rain Liu and the WebRTC.rs contributors. rvoip
changes are documented above and in the retained source history. Remove this
fork only after one immutable upstream release or commit contains all
documented behaviors and passes the full rvoip beta gate.
