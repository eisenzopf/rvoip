# Changelog

## 0.2.0 — 2026-07-11

This is a breaking alpha-protocol release.

- Media datagrams are now exactly the eight-byte UCTP header followed by one
  complete RTP packet. Checked `RtpDatagram` pack/unpack APIs reject codec-
  payload-only bodies.
- `stream_local_id` is allocated once per physical peer, is never zero, and is
  never reused during that peer lifetime.
- Negotiated Streams are bound by the substrate before `stream.opened`; the
  synthetic invite-time audio Stream has been removed.
- Added the shared `PeerMediaRouter`, atomic reservation/commit, exact route
  indexes, diagnostics, cancellation, and teardown.
- Added authenticated `SessionBindingResolver`, `PeerResourceBindings`, and
  canonical subscription routing. Cross-peer fanout now requires explicit
  authorized Session resolution.
- Publisher teardown is Connection-conditioned, so stale cleanup cannot remove
  a same-named replacement.
- Added `UctpCompatibility`, advertised compatibility facts, full byte/PCAP
  vectors, TLS key-log opt-in helpers, and real wire subscribe/unsubscribe
  conformance tests.

See `docs/BRIDGEFU_FOUNDATIONS_MIGRATION.md` for migration instructions.
