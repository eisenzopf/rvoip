# rvoip 0.3.2 Release Notes

Date: 2026-07-26

These notes describe the unified `0.3.2` workspace candidate. Behavioral and
performance claims remain bounded by the current clean beta report,
compatibility and RFC matrices, interoperability evidence, security posture,
and performance report.

## Headline

All 44 publishable crates move together to `0.3.2`. The SIP product remains the
release-gated beta surface. WebRTC, UCTP, Media over QUIC, identity, and
optional extensions retain their documented developer-preview or experimental
status where they are outside the SIP attestation.

## Added

- Authenticated-principal propagation and ownership checks now span SIP,
  WebRTC, UCTP, routes, and operational events.
- Transport-neutral data messaging covers arbitrary WebRTC DataChannels, SIP
  MESSAGE, typed initial SIP headers, DTMF, and correlated transfer outcomes.
- `MediaGraph` provides directional routes, codec-group transcoding, bounded
  fanout, snapshots, and drop/eviction metrics under a single-consumer model.
- SIP, WebRTC, and Amazon Connect support prepare/bind/activate lifecycles with
  owned cancellation, terminal events, and bounded drain.
- SIP outbound activation receipts linearize after the exact session becomes
  active; established teardown waits for the peer's final BYE response while
  retaining timeout/rejection cleanup.
- UCTP carries complete RTP packets and supports authenticated raw QUIC and
  WebTransport sessions, virtual publishers, direct-listener limits, and exact
  cleanup.
- `rvoip-moq` implements the documented draft-19/MSF-01/LOC-03 publisher,
  subscriber, origin, relay, authorization, reconnect, health, and drain
  abstractions.
- Symmetric RTP, advertised SIP/RTP addresses, RFC 3581 `rport`, WebRTC ICE/NAT
  policy, and per-exchange WHIP/WHEP versus WebSocket gathering are
  configurable.
- Developer-preview `rvoip-vapi` supplies a bidirectional WebSocket agent
  adapter through the facade's opt-in `vapi` feature and the `full` profile.
- `rvoip::app` adds voice-only SIP/WebRTC admission, transport-neutral accepted
  call events, startup-safe event retention, explicit SIP/RTP advertisement,
  and example 14's shared Vapi agent server.

## Compatibility Notes

- UCTP media datagrams now contain a complete RTP packet after the UCTP header.
- MOQT draft changes are wire-incompatible at the `rvoip-moq` compatibility
  boundary.
- Exhaustive matches over `rvoip_core_traits::connection::Transport` must add
  the `Vapi` variant.
- Exhaustive matches over `rvoip::app::AppEvent` must add the
  `InboundCallAccepted` variant.

These are intentional pre-1.0 compatibility changes. The private WebRTC/RTC
TURN candidate and dynamic moq-rs publisher-lease candidate remain outside the
consumed dependency graph.

## Beta-Scope Claims

- SIP APIs remain centered on `Endpoint`, `StreamPeer`, `CallbackPeer`,
  `UnifiedCoordinator`, and `SessionHandle`.
- Beta media support and interoperability claims are limited to the codecs,
  transports, peers, topology, and workloads recorded by the promoted report.
- General full-media performance claims remain capped at the documented 2,000
  CPS beta profile and require three source-identical canonical runs.
- Higher-CPS tuned results must retain their hardware, topology, workload, and
  configuration caveats.
- The full release gate includes workspace and downstream tests, documentation,
  API compatibility, Asterisk, FreeSWITCH, SIPp, baresip strict-UA, dependency
  audit, parser fuzz smoke, performance matrices, burst tests, and soaks.

## Must Not Claim Yet

- Broad production readiness.
- Carrier SBC certification.
- Browser/WebRTC support within the SIP beta qualification.
- DTLS-SRTP, ICE, or TURN support within the SIP beta qualification.
- Untested codec or topology support.
- General-user 10,000 CPS full-media capability.

## Evidence and Promotion

The [current release report](BETA_RELEASE_REPORT.md),
[complete gate report](BETA_GATE_REPORT.md), and
[performance report](BETA_PERFORMANCE_REPORT.md) are the authority for the
promoted candidate. Report promotion is a deterministic post-run derivation;
the source attestation remains bound to the clean tested commit.
