# rvoip-webrtc — Gap Plan (outstanding work)

As of v0.1.26+D the crate meets its **drop-in 1:1 WebRTC client/server library**
goal. Core ICE / DTLS-SRTP / SDP / RTP plumbing, WHIP/WHEP (RFC 9725), trickle
ICE, the codec set (Opus / G.711 / VP8 / VP9 / H.264-CB), RTCP feedback, typed
stats + Prometheus, in-process TLS, DTLS-fingerprint identity binding, the real
client surface (`client-cpal` / `client-video-vp8` / `client-video-h264`), and
the SIP↔WebRTC media bridge all landed (G1–G12 + D1–D4).

This doc was trimmed (2026-06-01) to track **only what remains**. What landed is
in [`../CHANGELOG.md`](../CHANGELOG.md); the full prior plans are in
[`archived/`](archived/) (`GAP_IMPLEMENTATION_PLAN`, `HARDENING_PLAN`,
`IMPLEMENTATION_PLAN`) and in git history.

## Outstanding 🟡 gaps (Should-level; the crate nominally owns these)

| Item | Status / next step | Ref |
|---|---|---|
| **WS signaling schema/version negotiation** | First-party WS JSON signaling is solid, but there's no schema or version negotiation — `serde`-parse only, no JSON Schema/versioning. | — |
| **TURN TCP / TLS** | URL parsing is supported but not exercised by any in-tree test (UDP TURN is covered). | RFC 6062, 7065 |
| **H.264 baseline profile** | Only `profile-level-id=42e01f` (constrained-baseline, Safari path) is offered; some SBCs require baseline `42001f`. | — |
| **Per-pair RTT in the snapshot** | RTT is available via webrtc-rs `get_stats` and in `CandidatePairStats`, but not surfaced on the top-level `WebRtcStatsSnapshot`. | — |
| **Structured per-connection event log** | Adapter events emit, but there's no per-connection ICE/DTLS/codec timeline. | — |
| **Per-route CORS allow-list** | Global `cors_origins` works; per-route allow-list deferred (G12). | — |

## Out-of-scope by design (⚪)

This is a **1:1 adapter**, not an SFU/MCU. Intentionally not implemented:

- SFU / MCU multi-party fan-out; Simulcast layer selection; SVC
- RED for Opus (RFC 2198), Comfort Noise (RFC 3389), G.722, AV1, H.265/HEVC
- RTCP XR (RFC 3611); W3C WebRTC Identity (`setIdentityProvider`)
- Hardware codec offload; insertable streams / E2EE; session resume after a signaling blip

> DTMF (RFC 4733) is **done** (D1) — the PT-101 wire path ships and `tests/dtmf_wire.rs` runs un-ignored; it is no longer a gap.
