# rvoip 0.3.9 Release Candidate Notes

Date: 2026-09-05

These notes describe the coordinated 45-crate `0.3.9` release candidate.
Publication requires a fresh protected `remote-release` qualification bound to
the exact release commit and current gate catalog. Earlier qualification
evidence does not qualify this release.

## Headline

`0.3.9` is the carrier-grade honesty release. SIP media behavior that existed
only as disconnected primitives is now reachable through production paths and
the public facade: playout and loss concealment, measured MOS and RTCP XR,
trusted carrier identity, ICE, DTLS-SRTP, per-session codec renegotiation,
remote endpoint registration, browser media, and lossless RTP observation.

The release also introduces six deployment-oriented facade bundles. G.711 is
the portable baseline; G.729, AMR-NB, and AMR-WB are first-class pure-Rust
carrier codecs; Opus remains first-class and explicit in native browser/AI
bundles because its current backend links `libopus`.

## Carrier SIP and remote endpoints

- The carrier profile enables deadline-driven playout, bounded reorder,
  G.711 packet-loss concealment, clock-drift handling, measured quality, and
  RTCP XR reporting. Unknown quality is no longer reported as a perfect MOS.
- Trusted-trunk policy gates inbound P-Asserted-Identity and an allowlist of
  private carrier headers. Typed PAI/PPI origination survives Digest retries
  without allowing arbitrary custom headers to impersonate those fields.
- Response Contacts honor the dialog transport, and observed-source routing
  applies consistently to BYE-with-reason, INFO, and NOTIFY.
- The production remote-endpoint profile uses authenticated RFC 5626 outbound
  registrations on exact TLS/WSS flows, rejects incomplete registrations,
  retains opaque flow capabilities, supports ordered failover, and removes
  stale routes on close, expiry, unregister, replacement, or restart.
- Awaitable connection and media readiness replaces application polling while
  preserving exact lifecycle generation, cancellation, and deadline outcomes.

## Secure media and NAT traversal

- SIP can negotiate `UDP/TLS/RTP/SAVP` with DTLS 1.2, SHA-256 fingerprints,
  RFC 8842 setup roles, RFC 7983 shared-socket demultiplexing, and secure-only
  latching before keys are installed. SDES remains available for compatible
  deployments.
- The sans-I/O RFC 8445 ICE agent and RFC 8489 STUN codec support full/lite
  roles, role conflicts, nomination, restarts, authenticated checks, and
  server-reflexive address discovery. SIP scope is one component with no TURN
  or trickle ICE in this release.
- RTP/RTCP teardown emits a standards-compliant compound Receiver Report plus
  BYE rather than unnegotiated reduced-size RTCP.

## Media, codecs, and application control

- Per-session re-INVITE codec changes commit only after the final negotiated
  answer. Rejection, timeout, replacement, or lost observation leaves the
  stable generation unchanged and retryable.
- Checked RTP packetization and observation preserve marker, CSRC, extension,
  padding, sequence, timestamp, SSRC, and negotiated payload identity when the
  caller requests a packet-preserving boundary.
- N-way mix-minus conferencing handles G.711 carrier and Opus browser members
  on one canonical mono mix bus, converts channel layouts at each boundary,
  and advances stereo RTP timestamps by sample frames. Supervisor monitoring
  can hear without contributing.
- Recording sink factories isolate concurrent recordings, and Vapi barge-in
  flushes both adapter audio and downstream graph queues with drop accounting.
- Authoritative ingress exposes admission tickets, a single-consumer durable
  operational stream, fail-closed health, and bounded drain behavior before
  adapters begin accepting work.
- Production AAuth delegation uses least-privilege scope intersection;
  signature freshness rejects far-future timestamps and replay handling is
  consume-once at the configured authority boundary.

## Deployment bundles

- `bundle-sip-endpoint`: provider-neutral SIP with G.711.
- `bundle-carrier-sip`: SIP, DTLS-SRTP, STIR/SHAKEN, G.711, G.729, AMR-NB,
  and AMR-WB.
- `bundle-browser-gateway`: high-level SIP/WebRTC/UCTP gateway with G.711 and
  Opus.
- `bundle-ai-conversation`: browser/AI conversation surface with G.711 and
  Opus.
- `bundle-full-pure-rust`: every facade surface and pure-Rust codec.
- `bundle-full-native`: every facade surface and all mainline codecs,
  including Opus.

The bundle catalog is manifest-derived, documented in `docs/FEATURE_BUNDLES.md`,
tested with default features disabled, and checked at the resolved dependency
graph. Release gates separately exercise codec-free, G.711, G.729, AMR,
Opus, and all-codec configurations.

## Compatibility and bounded claims

The changes are additive at the facade. Direct `rvoip-core` consumers that
previously acquired Opus through feature unification must now enable `opus` or
`all-codecs` explicitly; this prevents a purported pure-Rust graph from
silently linking a native codec. Applications should select a `bundle-*`
feature with `default-features = false` when deployment shape matters.

The remote endpoint profile is not declared fully qualified until the
protected run records its live two-UA NAT/TLS/SDES evidence. Browser/WebRTC,
ICE, DTLS-SRTP, codec, proxy, performance, and soak claims are bounded to the
peers, versions, machine shapes, workloads, and thresholds captured by the
signed qualification aggregate. TURN, SIP trickle ICE, TLS-SRTP, campaigns,
and unmeasured carrier networks are not implied.

## Qualification

The release candidate must first pass the normal PR Gate, then a complete
`remote-preflight`, followed by `remote-release` with `first_candidate=true`.
The full profile runs the structured 108-gate coverage ledger on hosted and
ephemeral GCP workers and includes a continuous one-hour soak. Publication
accepts only the signed aggregate for the exact clean `main` candidate.
