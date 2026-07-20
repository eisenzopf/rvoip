# Changelog

## Unreleased — Bridgefu 1.0 foundation

The following reusable work is present in the coordinated working tree and is
not yet a published rvoip release.

### Added

- Complete authenticated-principal propagation and ownership checks across
  SIP, WebRTC, UCTP, routes, and operational events.
- Transport-neutral `DataMessage`, arbitrary WebRTC DataChannels, SIP MESSAGE,
  typed initial SIP headers, DTMF, and correlated transfer outcomes.
- Single-consumer `MediaGraph` with directional routes, codec-group
  transcoding, bounded fanout, snapshots, drops/evictions, and metrics.
- Dormant prepare/bind/activate lifecycles for SIP, WebRTC, and Amazon Connect,
  including owned cancellation, terminal events, and bounded drain.
- SIP outbound activation receipts now linearize after the exact session is
  active. Established teardown waits for the peer's successful final BYE
  response while still reclaiming local state on timeout or rejection.
- UCTP 0.2 complete-RTP routing, authenticated raw QUIC/WebTransport sessions,
  virtual publishers, direct-listener limits, and exact cleanup.
- `rvoip-moq` draft-19/MSF-01/LOC-03 publisher, subscriber, origin, relay,
  authorization, compatibility, reconnect, health, and drain abstractions.
- Configurable symmetric RTP, advertised SIP/RTP addresses, RFC 3581 `rport`,
  WebRTC ICE server/NAT policy, and per-exchange WHIP/WHEP versus WS gathering.

### Breaking protocol changes

- UCTP media datagrams now carry a complete RTP packet after the UCTP header.
- Wire-incompatible MOQT draft changes are semver-breaking at the
  `rvoip-moq` compatibility boundary.

The private WebRTC/RTC TURN candidate and the dynamic moq-rs publisher-lease
candidate remain outside the consumed dependency graph until project-owner
review. No upstream submission is authorized by this changelog.
