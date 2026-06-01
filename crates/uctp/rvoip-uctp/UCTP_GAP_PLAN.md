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
| §3.2 | **WebTransport browser bidi-stream interop** | SPKI pinning + WT session readiness work under headless Chromium across `web-transport-quinn` 0.5–0.11, and the full wire path works Rust↔Rust. **Blocked:** the Chromium→Rust *bidirectional-stream* envelope round-trip doesn't complete (`accept_bi` hangs/returns empty across all four versions tried). Upstream-dependency block — track `web-transport-quinn` 0.12+, or reshape the handshake to unidirectional-stream-per-direction + datagrams (better browser interop). The browser-smoke spec asserts the SPKI/readiness deliverable and logs the bidi gap rather than failing. |
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
