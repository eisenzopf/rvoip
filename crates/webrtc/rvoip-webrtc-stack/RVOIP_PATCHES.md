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

## Peer connection driver: expired deadlines and a closed command channel

Upstream's driver loop handled an already-expired protocol deadline by calling
`handle_timeout` synchronously and `continue`-ing, so a deadline that stayed in
the past (for example a DTLS retransmit timer after a call ended) spun a Tokio
worker without ever polling the command channel; a queued `Close` was never
serviced. It also ignored `None` from the command channel, so once every
sender was gone `recv()` resolved immediately on each pass and the loop never
ended.

rvoip's driver sends an expired deadline through the same timer branch of the
`select!` as any other deadline (a zero-length sleep), so the command channel
and sockets are polled and the task yields on every pass, and it returns when
the command channel closes. Private regressions in `peer_connection::driver`
cover a repeatedly expired deadline with a queued `Close`, abort of a driver
whose deadline stays expired, and a closed command channel.

## `rvoip-rtc` 0.21.1 baseline

`rvoip-rtc` now tracks upstream `rtc` 0.21.1 rather than 0.20.0-alpha.1, and
this crate is adapted to that API without otherwise moving its own `webrtc`
0.20.0-alpha.1 baseline:

- `RTCPeerConnection` is no longer generic over its interceptor chain, so the
  interceptor type parameter is gone from `PeerConnectionBuilder` and the
  internal peer-connection, transceiver, sender, receiver and data-channel
  wrappers. `PeerConnectionBuilder::with_interceptor_registry` takes the
  non-generic `Registry` and returns the same builder type.
- The driver and wrappers pass `Instant::now()` to the core calls that now take
  the current time (`build`, descriptions, RTP/RTCP writes, data-channel sends,
  the STUN client and the sample packetizer) and read `TaggedRTCMessage` from
  `poll_read`.
- Events and messages the core marks `#[non_exhaustive]` that this crate does
  not handle are logged at debug level and ignored.
- `create_data_channel` wakes the driver, as upstream `webrtc` 0.21 does, so
  the DCEP OPEN of a channel dialed on an established association is written
  at once instead of at the next protocol timeout.
- `DataChannel::id` is now the core's connection-local handle, which differs
  between the two peers. `DataChannel::stream_id` (a provided trait method)
  returns the SCTP stream id both peers share.
- `SettingEngineBuilder` is re-exported from `webrtc::peer_connection`, and
  tests and examples build `SettingEngine` through it. A relay-only ICE policy
  now needs a TURN server, so the policy test configures one.
