# rvoip-sip Next Release Notes Draft

Date: 2026-05-26

These notes are a draft for the beta line. Keep only claims that are backed by
the final clean beta report, compatibility matrix, RFC matrix, interop results,
security posture, and performance report.

## Headline

`rvoip-sip` is moving from alpha toward beta as a Rust-native SIP application
layer for bounded client, server, PBX, and gateway scenarios.

## Beta-Scope Claims

- Public API surfaces are centered on `Endpoint`, `StreamPeer`,
  `CallbackPeer`, `UnifiedCoordinator`, and `SessionHandle`.
- Beta media support is limited to PCMU, PCMA, telephone-event DTMF, optional
  comfort noise, RTP, and tested SDES-SRTP profiles.
- Interop evidence covers local Asterisk, local FreeSWITCH, SIPp standalone
  load scenarios, and baresip strict-UA behavior in the current reference
  report.
- General full-media performance claims are capped at the documented 2,000 CPS
  beta profile backed by the final clean report.
- Higher CPS results are advanced tuned profiles and must include tuning,
  hardware, topology, and caveats.
- SIP trace output redacts authorization data, authentication challenges,
  cookies, identity headers, token-like headers, and SDP keying attributes.
- The release gate includes local tests, interop/performance modes, dependency
  audit, and parser fuzz smoke targets.

## Current Reference Evidence

- Reference report:
  `crates/rvoip-sip/beta-report/20260526T221457Z/summary.md`
- Result: `0` failures, `0` skips
- Git revision: `865430d4`
- Git status at run time: `clean`
- Rust/Cargo: `1.88.0`

This is the final clean beta release-candidate attestation.

## Must Not Claim Yet

- Broad production readiness.
- Carrier SBC certification.
- Browser/WebRTC support.
- DTLS-SRTP, ICE, or TURN support.
- Opus, G.722, or G.729 full-media support.
- WSS outbound support.
- PUBLISH end-to-end application support.
- General-user 10,000 CPS full-media capability.

## Release Evidence Archived

- `COMPATIBILITY_MATRIX.md` complete for claimed features.
- `RFC_COMPLIANCE_MATRIX.md` complete for claimed RFCs.
- `INTEROP_CI_PLAN.md` scenarios run and archived.
- `BETA_PERFORMANCE_REPORT.md` populated with raw-result links.
- `SECURITY_POSTURE.md` release checks complete.
- `scripts/beta_gate.sh --local` passes.
- `scripts/beta_gate.sh --security` passes and archives dependency audit plus
  parser fuzz smoke logs.
- `scripts/beta_gate.sh --full --require-external` passes from a clean commit.
- The checklist explicitly accepts the 30-minute soak as the beta release bar.
