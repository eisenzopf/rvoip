---
section: Breaking changes
breaking: true
---
`rvoip-rtc` (library name `rtc`) is rebased from upstream `rtc` 0.20.0-alpha.1
onto `rtc` 0.21.1, and its `rtc-*` dependencies move to the exact 0.21.1
releases. The rvoip patches it still needs are kept and the ones upstream now
ships are dropped (see the crate's `RVOIP_PATCHES.md`). Its public API is now
upstream 0.21's, which is not source compatible with 0.20: `RTCPeerConnection`
is no longer generic over its interceptor chain, `RTCPeerConnectionBuilder::build`
and the calls that act at a point in time (`set_local_description`,
`set_remote_description`, `RTCRtpSender::write_rtp`, `write_rtcp`, data-channel
`send`/`send_text`, `get_stats`) take an `Instant`, `poll_read` yields a
`TaggedRTCMessage`, `RTCDataChannelId` is a connection-local `usize` handle
rather than the SCTP stream id, settings are built with `SettingEngineBuilder`,
and cryptography goes through a per-connection `RTCCryptoProvider`. The
`openssl`, `vendored-openssl` and `pem` features are removed; `crypto-ring`
(the default) and `crypto-aws-lc-rs` replace them. `rvoip-webrtc-stack`'s
`PeerConnectionBuilder` loses its interceptor type parameter, and its
`DataChannel::id` is now that per-connection handle; the new
`DataChannel::stream_id` returns the SCTP stream id both peers share.

Two behaviours change for every user, including code that only uses
`rvoip-webrtc`: a relay-only ICE transport policy without a TURN server is
rejected when the peer connection is built, and a data channel opens only on a
session whose offer carried it (create it before the offer, or renegotiate),
because SCTP is no longer started for a media-only session.

To migrate code that uses `rvoip-rtc` directly, follow upstream's
[crypto-provider migration guide](https://github.com/webrtc-rs/rtc/blob/master/docs/crypto-provider-migration.md):
drop the interceptor type parameter, pass `Instant::now()` where a call now
asks for the time, destructure `TaggedRTCMessage { message, .. }` from
`poll_read`, replace `SettingEngine::set_*` calls with
`SettingEngineBuilder::new().with_*(..).build()`, store data-channel ids as
`RTCDataChannelId`, and enable `crypto-aws-lc-rs` instead of the removed
`openssl` features if you need a non-`ring` backend. For
`rvoip-webrtc-stack`, write `PeerConnectionBuilder<A>` instead of
`PeerConnectionBuilder<A, I>` (`with_interceptor_registry` now takes a
`Registry`), and compare `DataChannel::stream_id` rather than `id` across
peers. Configure a TURN server with `IceTransportPolicy::Relay`.
