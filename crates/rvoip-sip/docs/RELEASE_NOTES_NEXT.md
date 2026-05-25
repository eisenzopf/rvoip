# rvoip-sip Next Release Notes Draft

Date: 2026-05-25

These notes are a draft for the beta/production-candidate line. Only keep
claims that are backed by the compatibility matrix, RFC matrix, interop
results, and performance report.

## Headline

`rvoip-sip` is moving from alpha toward beta as a Rust-native SIP application
layer for bounded client, server, PBX, and gateway scenarios.

## Beta-Scope Claims

- Public API surfaces are centered on `Endpoint`, `StreamPeer`,
  `CallbackPeer`, `UnifiedCoordinator`, and `SessionHandle`.
- Beta media support is limited to PCMU, PCMA, telephone-event DTMF, optional
  comfort noise, RTP, and tested SDES-SRTP profiles.
- General full-media performance claims are capped at the documented 2,000 CPS
  beta profile until release evidence proves the exact setup.
- Higher CPS results are advanced tuned profiles and must include tuning,
  hardware, and topology caveats.
- WebRTC-grade media is post-beta unless completed and separately tested.

## Must Not Claim Yet

- Broad production readiness.
- Carrier SBC certification.
- Browser/WebRTC support.
- DTLS-SRTP, ICE, or TURN support.
- Opus, G.722, or G.729 full-media support.
- WSS outbound support.
- PUBLISH end-to-end application support.
- General-user 10,000 CPS full-media capability.

## Release Evidence Required

- `COMPATIBILITY_MATRIX.md` complete for claimed features.
- `RFC_COMPLIANCE_MATRIX.md` complete for claimed RFCs.
- `INTEROP_CI_PLAN.md` scenarios run and archived.
- `BETA_PERFORMANCE_REPORT.md` populated with raw-result links.
- `SECURITY_POSTURE.md` release checks complete.
- `scripts/beta_gate.sh --local` passes.
- `scripts/beta_gate.sh --full` either passes with external artifacts or every
  skip is converted into an explicit non-claim in these notes.
