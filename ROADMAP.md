# rvoip Roadmap

**Updated:** 2026-08-11  
**Purpose:** portfolio-level rollup of active and future work across the rvoip
workspace. Detailed component plans remain authoritative for acceptance criteria
and implementation history.

## Status model

- **Active** — accepted work with an existing detailed tracker.
- **Future** — desired direction; not scheduled and not a support claim.
- **Research** — requires standards, interoperability, dependency, or product
  validation before implementation is committed.

## Active engineering tracks

| Track | Status | Detailed tracker |
|---|---|---|
| Open-internet media hardening | Active | [`docs/PRODUCTION_HARDENING_ROADMAP.md`](docs/PRODUCTION_HARDENING_ROADMAP.md) — DTLS fragmentation, crypto review, CI/hygiene follow-through |
| Cross-product architecture gaps | Active | [`docs/GAP_PLAN.md`](docs/GAP_PLAN.md) — deferred v1.x/v2 work and protocol adapters |
| UCTP completion | Active | [`crates/uctp/rvoip-uctp/UCTP_GAP_PLAN.md`](crates/uctp/rvoip-uctp/UCTP_GAP_PLAN.md) — browser bidi-stream interoperability and per-session SIP codec override |
| SIP standards promotion | Active | [`docs/sip/SIP_RFC_COMPLIANCE.md`](docs/sip/SIP_RFC_COMPLIANCE.md#roadmap-rollup) — partial/type-only behaviors to promote with executable evidence |
| SIP transport reliability | Active | [`crates/sip/sip-transport/TODO.md`](crates/sip/sip-transport/TODO.md) — failover, recovery, backpressure, load/stress, and interoperability |

## Future protocols and products

| Initiative | Status | Intended integration |
|---|---|---|
| SIP-over-QUIC | Future | Add a QUIC transport to `rvoip-sip-transport`; keep the SIP transaction/dialog layers transport-neutral. |
| RTP-over-QUIC (RoQ) | Future | Add a codec-agnostic `rvoip-roq` adapter for RTP/RTCP over QUIC DATAGRAMs and, where useful, QUIC streams. Preserve UCTP's distinct datagram framing and bridge at the media-adapter boundary. |
| **Lyra V2 over UCTP, RTP, SIP, and RoQ** | Research | Add an optional Lyra V2 codec backend, an experimental RTP payload mapping, UCTP capability negotiation, SIP SDP integration, and a Lyra-over-RoQ end-to-end proof. See the workstream below. |
| Media over QUIC (MoQ) | Future | Extend the existing MoQ crates toward broadcast-scale fan-out and UCTP/WebRTC bridging. |
| Multi-party SFU/MCU products | Future | Build integrated products on the shipped media and subscription-routing primitives. |
| AAuth production graduation | Future | Graduate the experimental standards work after security and deployment evidence mature. |
| Deeper AI participants | Future | Add multi-agent orchestration beyond the current pluggable provider harness. |
| QUIC multipath/NAT traversal | Research | Re-evaluate `noq` or equivalent when direct peer-to-peer and mobile multipath requirements justify a `quinn`-compatible change. |

## Lyra V2 + RTP-over-QUIC workstream

### Goal

Demonstrate a bidirectional voice session that encodes 20 ms PCM frames with
Lyra V2, packetizes them as RTP, and carries the same RTP stream over either
ordinary UDP or RoQ. The proof must work through both a UCTP Session and a SIP
call controlled by `rvoip-sip`.

This is initially an **experimental, controlled-endpoint profile**. Lyra does
not currently have an IETF-standard RTP payload format or a broadly deployed
IANA/SDP registration, and RoQ is not yet a broadly deployed production
transport. The roadmap must not imply third-party interoperability until a
stable specification and independent test evidence exist.

### Component ownership

1. **`rvoip-codec-core` — Lyra V2 codec backend**
   - Add an opt-in feature and isolate the Google Lyra/TFLite native dependency
     from default builds.
   - Support mono 20 ms frames, 3.2/6/9.2 kbps, and the Lyra V2-supported
     external sample rates.
   - Expose encode/decode, bitrate selection, packet-loss concealment behavior,
     and capability metadata through the existing codec abstractions.

2. **`rvoip-rtp-core` — experimental Lyra RTP profile**
   - Add packetizer/depacketizer support using a dynamic payload type.
   - Define the controlled-profile SDP shape, initially following the proven
     experimental convention `lyra/<clock-rate>/1` with a `bitrate` `fmtp`
     parameter.
   - Specify timestamp increments, frame aggregation rules, marker behavior,
     loss handling, and maximum packet size. Keep the profile explicitly
     namespaced/experimental until standardized.

3. **`rvoip-uctp` — Lyra capability and bridge support**
   - Advertise Lyra V2 clock rates, bitrates, frame duration, and experimental
     RTP profile in `CapabilityDescriptor` negotiation.
   - Carry Lyra RTP packets through the existing UCTP media datagram framing;
     do not create a second codec-specific UCTP wire format.
   - Add codec selection, mismatch refusal, and optional transcoding fallback
     for UCTP-to-SIP bridges.

4. **`rvoip-roq` — RTP/RTCP over QUIC**
   - Implement RoQ flow-ID mapping over QUIC DATAGRAMs first; evaluate stream
     modes separately so reliable delivery cannot accidentally add
     head-of-line latency to conversational audio.
   - Remain codec-agnostic, with Lyra V2 as the first low-bitrate audio proof
     and Opus/G.711 as interoperability controls.
   - Define RTCP/congestion-control ownership and whether QUIC transport
     feedback can safely replace any RTCP feedback in each deployment mode.

5. **`rvoip-sip` — negotiation and end-to-end product integration**
   - Offer/answer the experimental Lyra RTP profile in SDP only when enabled.
   - Complete the existing per-session codec-override gap so re-INVITEs and
     UCTP/SIP bridges can select Lyra without changing global codec policy.
   - Support SIP signaling with a media leg using Lyra RTP over UDP and a
     bridged media leg using the same RTP packets over RoQ.
   - Treat RoQ as a media transport selected by the media adapter, not as a new
     SIP message encoding. SIP-over-QUIC remains a separate roadmap item.

### Proof and qualification gates

- Codec golden vectors and encode/decode round trips for every supported
  bitrate and enabled clock rate.
- RTP sequence/timestamp, loss, reordering, aggregation, and SDP negotiation
  tests.
- UCTP Lyra-to-Lyra loopback and UCTP-to-SIP bridge tests.
- `rvoip-sip` calls using Lyra/RTP over UDP, then the same RTP flow tunneled over
  QUIC DATAGRAMs through `rvoip-roq`.
- Comparison runs against Opus and G.711 under clean, lossy, reordered, and
  mobile-path-migration conditions.
- Measure mouth-to-ear latency, CPU, memory, packet-loss behavior, and actual
  wire bitrate at 20, 40, and 100 ms packetization intervals. Report codec,
  RTP/RTCP, QUIC, and IP/UDP overhead separately.
- Security review of QUIC-only transport encryption versus retaining SRTP for
  end-to-end protection across a terminating RoQ gateway.
- Independent endpoint interoperability before removing the experimental
  label or making a compatibility claim.

## Maintenance

- Keep this file as the root portfolio rollup; do not copy detailed phase
  histories into it.
- Add new work here when it crosses crate/product boundaries. Keep crate-local
  TODOs and acceptance details in the linked component tracker.
- A published crate, feature flag, or experimental demo is not by itself a
  production-readiness or interoperability claim.
