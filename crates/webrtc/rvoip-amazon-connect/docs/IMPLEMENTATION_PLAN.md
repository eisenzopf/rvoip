# rvoip-amazon-connect — implementation plan & roadmap

## Goal

Deliver a call arriving over any rvoip transport (SIP, WebRTC, UCTP/QUIC) to a
live Amazon Connect agent, translating inbound SIP custom headers into Connect
contact attributes that drive an agent screen pop. Motivating path: PSTN → Vapi
→ blind transfer (SIP INVITE + `X-` headers) → rvoip → Amazon Connect.

## Status (v1 — shipped)

| Area | State |
|---|---|
| `Transport::AmazonConnect` (core-traits) | ✅ added |
| SIP-header → attribute translation (`mapping.rs`) | ✅ + unit tests |
| Vendored Chime `SignalingProtocol.proto` + prost codegen | ✅ |
| Chime signaling client (JOIN/JOIN_ACK/SUBSCRIBE/ACK/PING/LEAVE) | ✅ (offline-tested) |
| Control plane (`StartWebRTCContact`) behind `ConnectContactStarter` trait | ✅ |
| `AwsConnectStarter` (`aws-control` feature) | ✅ compiles against `aws-sdk-connect` |
| `AmazonConnectAdapter` implementing `ConnectionAdapter` | ✅ |
| Reuse of rvoip-webrtc peer/media (no fork) | ✅ |
| Batteries-included `ConnectScreenPopServer` (`server` feature) | ✅ SIP UAS + transcoding bridge |
| Example 13 (boots the server) + adapter mock-control test | ✅ |

## Batteries-included server (`server` feature)

`ConnectScreenPopServer` is the turnkey entry point — one `ScreenPopServerConfig`
(SIP `Config` + `ConnectConfig` + `ConnectContactStarter`) and it:
1. runs a SIP UAS via `UnifiedCoordinator::next_incoming_call` (full headers),
2. extracts custom `Other` headers from the parsed INVITE (`raw_header_value`,
   preserving case + clean values),
3. translates them via `AttributeMapping`,
4. answers SIP, places the Connect contact, and bridges the two `MediaStream`s
   with `bridge.rs` (the orchestrator's transcoder recipe over raw streams:
   `codec_to_pt` + per-direction `Transcoder` + `spawn_pump`).

A teardown watcher (a second broadcast `events()` subscription) ends the Connect
contact + aborts the bridge when the SIP leg sends `CallEnded`/`CallFailed`/
`CallCancelled`, so a BYE never leaks the Connect contact.

Design note: rvoip has **no single global config** — per-crate config structs
are the norm (`rvoip_sip::Config`, `WebRtcConfig`, etc.), optionally composed by
the `rvoip` app facade. `ScreenPopServerConfig` follows that convention: it
composes the SIP config + Connect config rather than inventing a new global one.
The server bridges streams **directly** (not through `Orchestrator`) because the
orchestrator's `bridge_connections` requires both legs in its connection table
and its `ConnectionInbound` event drops SIP headers — the direct path keeps
header access first-class and everything self-contained in the connector.

## Architecture decisions

- **Control plane is a trait** (`ConnectContactStarter`). The crate + unit tests
  build with zero AWS deps; the real `aws-sdk-connect` impl is behind
  `aws-control`. This also isolates the AWS `aws-lc-rs` crypto provider from the
  workspace's `ring` rustls provider (they must not coexist in one process).
- **TURN from JOIN_ACK**, not a separate `TurnControlUrl` POST — matches the
  modern Chime SDK join flow. Hence the two-step `join()` → `subscribe()` API:
  the peer connection is built between them so JOIN_ACK TURN creds seed its ICE.
- **Full-gather (trickle off)** so the SUBSCRIBE frame carries a complete SDP
  offer inline, which Chime's signaling expects.
- **Media reuse**: `RvoipPeerConnection` + `from_tracks_with_dtmf_events` +
  pumps come from rvoip-webrtc unchanged.

## Chime signaling wire format — VALIDATED against live Amazon Connect (2026-06-17)

The `connect-probe` binary established a full connection to a live instance
(StartWebRTCContact → JOIN/JOIN_ACK+TURN → SUBSCRIBE → SDP answer → ICE+DTLS
Connected). The wire details below were corrected against the Amazon Chime SDK
source (`DefaultSignalingClient` / `SignalingClientConnectionRequest`) and
confirmed live. All isolated to `src/signaling/chime.rs`:

1. **Signaling URL** = `<SignalingUrl>?X-Chime-Control-Protocol-Version=3&X-Amzn-Chime-Send-Close-On-Error=1`.
   The join token is **NOT** a query param.
2. **Auth** = the attendee join token rides in the WebSocket subprotocol header:
   `Sec-WebSocket-Protocol: _aws_wt_session, <joinToken>`.
3. **Binary framing** = every WS binary message is `[0x05 frame-type byte] +
   protobuf SdkSignalFrame` (Chime's `FRAME_TYPE_RTC`). Prepend on send, strip on
   receive.
4. **JOIN** = `protocol_version=2`, minimal client_details — accepted as-is.
5. **TURN** = taken from `JOIN_ACK.turn_credentials` (not a `TurnControlUrl` POST).
6. **Audio-only SUBSCRIBE** = `duplex = RX` (NOT `DUPLEX` — that triggers a
   server-side "failed to initialize video session" 400), plus one
   `AmazonChimeExpressAudio` send-stream descriptor carrying our `attendee_id`,
   the full SDP offer in `sdp_offer`, and `audio_host` = `MediaPlacement.AudioHostUrl`.

Known-benign warnings during the probe: rtc-dtls logs `server_name` defaulting to
"localhost" (DTLS still completes), and a "remote-track channel full" warning on
immediate teardown (no consumer attached in the probe). Neither blocks the
connection; verify they stay benign once real media is bridged.

## Future scope

- Video / screen-share (`AllowedCapabilities` plumbed at the API, not negotiated).
- Multiple agents / WHEP-style fan-out.
- Connect → other-transport origination (outbound).
- Surface Chime `SdkAudioStatusFrame` / metrics into `AdapterEvent::Quality`.
