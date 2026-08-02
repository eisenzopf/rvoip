# rvoip 0.3.6 Release Candidate Notes

Date: 2026-07-30

These notes describe the coordinated 44-crate `0.3.6` release candidate.
Publication requires a fresh strict full-beta qualification bound to the exact
clean release source. No `0.3.4` carry-forward evidence qualifies this release.

## Headline

`0.3.6` is a security, media-correctness, and SIP-lifecycle release. It makes
incomplete security features fail closed, repairs RTP/RTCP and SRTP/SRTCP
state, replaces simulated codec behavior, completes transactional SIP
renegotiation, closes issues #46 and #50, and makes Tokio the sole WebRTC
runtime.

## Security and media correctness

- Placeholder AEAD AES-GCM SRTP profiles retain their public identities but
  cannot be configured, advertised, selected, or negotiated. DTLS construction
  returns a typed unsupported-feature error instead of panicking.
- RTP padding and RFC 8285 one-byte/two-byte header extensions now serialize
  and parse with strict length, padding, reserved-ID, and alignment handling.
- RTP loss and jitter tracking handles sequence rollover, duplicates, gaps, and
  reordered packets without corrupting the timing reference. RTCP LSR uses the
  complete middle 32 bits of the NTP timestamp, and compound RTCP ingress
  preserves well-formed unknown packet types while rejecting malformed input.
- SRTP maintains independent local outbound and remote inbound keys and
  per-SSRC rollover/replay state. Incoming packets are authenticated before
  replay state commits. SRTCP uses monotonic per-SSRC indexes and derives each
  IV from the real SSRC and index. RFC vectors, rollover/reordering, replay,
  multi-SSRC, failed-authentication, and pinned libSRTP interoperability are
  covered.
- Low-level SDES answers generate fresh directional key material. AES-256 SDES
  accepts safely unpadded key material in compatible mode, keeps strict mode
  canonical, validates decoded lengths, and emits secret-safe negotiation and
  authentication diagnostics. This closes issue #46.

## SIP signaling and lifecycle

- Signaling-only and media-enabled hold/resume share transactional stable-state
  handling. Repeated hold and resume operations are idempotent.
- Re-INVITE, UPDATE, delayed-offer INVITE/200/ACK, authentication retry,
  retransmission, rollback, and media application paths use exact transaction
  and CSeq ownership. Invalid or incomplete negotiation cannot corrupt the
  established dialog.
- Failed 2xx SDP negotiation is ACKed, terminates exactly once, and produces an
  application-visible negotiation failure without exposing key material.
- `EndpointBuilder`, `StreamPeer`, and `StreamPeerBuilder` now accept
  `SipNatConfig` and `SymmetricRtpPolicy`, allowing high-level applications to
  tune symmetric-RTP rebinding for real NAT deployments. This closes issue
  #50.
- BYE, redirect, admission, and retry cleanup retains one terminal fact,
  exact-generation ownership, bounded retention, and keyed/no-scan hot paths.
  Registrar and SIP crates pass the strict release lint policy.

## Codecs and WebRTC

- Audio/video codec-name classification is ASCII case-insensitive. Opus uses
  the real codec backend through the canonical codec-core implementation;
  `opus-sim` remains only as a deprecated alias to that backend.
- Feature-disabled builds do not advertise Opus. G.722 low-level RTP payload
  support remains available, but codec factories and negotiation report it as
  unsupported until a complete codec exists.
- The standalone WebRTC stack is Tokio-only; Smol and async-std runtime support
  and dependencies are removed. CI exercises default, all-feature, and
  no-default-feature configurations and scans the complete all-features graph
  for forbidden alternate runtime dependencies.
- The confirmed Chromium SDP regression is corrected for codec-specific audio
  selection, primary SSRC handling, and empty simulcast output.

## Architecture and compatibility

- SIP signaling changes preserve the sharded, exact-key, generation-protected,
  bounded-retention architecture in
  [`SIGNALING_PERFORMANCE_ARCHITECTURE.md`](SIGNALING_PERFORMANCE_ARCHITECTURE.md).
  They do not introduce per-transaction sleeper tasks, unbounded histories, or
  global hot-path scans.
- Public compatibility is compared with the documented `0.3.4` baseline.
  Typed unsupported errors replace behavior that previously panicked or
  implied unavailable security/codec support.
- AES-GCM, end-to-end SIP DTLS-SRTP, MIKEY, ZRTP, and G.722 codec negotiation
  remain explicit non-claims. The new ICE/NAT architecture and unrelated
  WebRTC additions from PR #35 are not part of this repair release.
- General-user 10,000 CPS full-media capability is not claimed. The strict SIP
  beta envelope remains bounded by its recorded 2,000-CPS real-media profile,
  exact host configuration, peer matrix, workloads, and soak durations.
- Browser/WebRTC edge qualification remains separate from the SIP beta claim;
  the exact Chromium repair tests do not broaden that claim to untested
  browsers, ICE/TURN deployments, or network topologies.

## Qualification

The release candidate must pass the one-command full beta gate from a clean,
committed `0.3.6` source tree. Required evidence includes three fresh canonical
2,000-CPS runs; workspace, public-API, security, parser, PBX, SIPp, strict-UA,
Kamailio, and OpenSIPS gates; full-media performance and resiliency matrices;
and both one-hour monolithic and split soaks. The generated report package and
its source fingerprint are verified before crates.io publication.

Historical `0.3.2` exception and `0.3.4` carry-forward attestations remain
unchanged release history. They are not presented as current `0.3.6` evidence.
