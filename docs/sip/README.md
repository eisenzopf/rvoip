# SIP documentation

Documentation for the `rvoip` SIP stack (`rvoip-sip`, `rvoip-sip-dialog`,
`rvoip-sip-core`, `rvoip-sip-transport`, `rvoip-sip-proxy`,
`rvoip-sip-registrar`).

## Contents

- **[SIP_RFC_COMPLIANCE.md](SIP_RFC_COMPLIANCE.md)** — the comprehensive SIP RFC
  compliance matrix: every SIP and SIP-adjacent RFC by number, with title,
  description, implementation status, and the **test/interop evidence** that
  attests to each claim. Use it to know what is supported today and for roadmap
  planning.

## Related (crate-local) docs

- [`crates/sip/rvoip-sip/docs/RFC_COMPLIANCE_MATRIX.md`](../../crates/sip/rvoip-sip/docs/RFC_COMPLIANCE_MATRIX.md)
  — authoritative **beta-claim** record (what release notes may claim).
- [`crates/sip/rvoip-sip/docs/SECURITY_POSTURE.md`](../../crates/sip/rvoip-sip/docs/SECURITY_POSTURE.md)
  — security non-claims (DTLS-SRTP, ICE, TURN).
- [`crates/sip/rvoip-sip/beta-report/`](../../crates/sip/rvoip-sip/beta-report/)
  — generated beta-gate reports (interop matrices, SIPp, baresip, perf) that
  back the attestations.

## Attestation basis

The compliance matrix is grounded in beta-gate report `20260616T014649Z`
(revision `2bd8c570`, all gates PASS). Reproduce with
`crates/sip/rvoip-sip/scripts/beta_gate.sh`.
