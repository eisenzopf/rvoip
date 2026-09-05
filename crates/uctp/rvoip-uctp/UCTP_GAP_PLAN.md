# UCTP — Gap Plan (outstanding work)

The UCTP v0 spike, the v0.x production-hardening track, and multi-party routing
all landed — see [`UCTP_IMPLEMENTATION_PLAN.md`](UCTP_IMPLEMENTATION_PLAN.md)
§11–§13 for the authoritative as-built record. The 2026-05-25 v1 punch list
closed 4 of its 5 items (DTMF auto-route, coordinator auto-verify gate, outbound
trickle-ICE pump, and the §4.2 `renegotiate_media` driver across QUIC/WT/WS/SIP).

This doc was trimmed (2026-06-01) to track **only what remains**. The full
section-by-section history is in git (`git log --follow` this file) and in
`UCTP_IMPLEMENTATION_PLAN.md`.

## Outstanding (carry-forward to v1.x)

| # | Item | Status / next step |
|---|---|---|
| §3.2 | **WebTransport browser interop** | **Closed for the supported profile:** UCTP uses one finite unidirectional stream per control envelope and WebTransport datagrams for RTP. The required Chromium gate proves TLS/SPKI admission, auth, RFC 8785/Ed25519-signed controls, Connection offer/answer/readiness, sustained bidirectional RTP, DTMF and quality events, teardown, and capacity-releasing reconnect. Long-lived bidirectional control streams are not part of the public profile. |
| §4.2 | **SIP per-session codec override** | `renegotiate_media` re-INVITE currently uses the SIP layer's configured `offered_codecs`, not the orchestrator-supplied list. Add `UnifiedCoordinator::set_offered_codecs_for_session(session, Vec<u8>)` (a thin wrapper over the existing `MediaAdapter::set_offered_codecs`) so the orchestrator can pass codec preferences through the SIP SDP generator. |

## Out-of-scope (§6 — tracked, not scheduled)

- CRC32 envelope checksums
- `stream.active-speaker` emission
- `recording.vcon-fetch` round-trip
- WebTransport-over-HTTP/3-datagram

## References

- Authoritative design + as-built record: [`UCTP_IMPLEMENTATION_PLAN.md`](UCTP_IMPLEMENTATION_PLAN.md)
- Wire spec: [`CONVERSATION_PROTOCOL.md`](../../../docs/CONVERSATION_PROTOCOL.md) · Architecture: [`INTERFACE_DESIGN.md`](../../../docs/INTERFACE_DESIGN.md)
- WS↔WebRTC media bridge: [`../rvoip-websocket/src/media_bridge.rs`](../rvoip-websocket/src/media_bridge.rs)
