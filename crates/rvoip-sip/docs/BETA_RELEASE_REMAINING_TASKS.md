# rvoip-sip Beta Release Remaining Tasks

Date: 2026-05-26

This document tracks what remains before `rvoip-sip` can be called beta release
feature-complete under the active beta docs. An item is complete only when
there is linked release evidence, an explicit non-claim, or an accepted
exclusion.

## Current Status

The beta gap-closing plan has been implemented in repository files:

- `scripts/beta_gate.sh --security` now runs dependency advisory audit and
  parser fuzz smoke targets.
- Parser fuzz targets exist for SIP message parsing, URI parsing, header
  parsing, and SDP parsing.
- SIP trace redaction now covers authorization data, authentication challenges,
  cookies, identity headers, token-like headers, and SDP keying attributes.
- `SECURITY_POSTURE.md`, `RFC_COMPLIANCE_MATRIX.md`,
  `COMPATIBILITY_MATRIX.md`, `BETA_PERFORMANCE_REPORT.md`,
  `BETA_RELEASE_CHECKLIST.md`, and `RELEASE_NOTES_NEXT.md` are converted from
  placeholder status toward evidence-linked release docs.
- `README.md` documents MSRV, semver policy, and feature flag support.
- `rvoip-sip` crate metadata declares the workspace Rust version.
- The dependency audit remediation has been applied: hard RustSec blockers for
  Hickory DNS, rustls-webpki/rustls, `rsa`, `time`, and DTLS/rcgen transitive
  paths are cleared under the Rust 1.88 baseline.
- Current short security gate passed with Rust 1.88:
  `target/beta-gate/20260526T194243Z/summary.md`.

The release is still not fully attested because the final clean report,
security artifacts, and 24-hour soak have not been produced in this change.

Latest complete no-skip reference gate:

- Command: `BETA_RUN_LOCAL_PBX=1 crates/rvoip-sip/scripts/beta_gate.sh --full --require-external`
- Summary: `crates/rvoip-sip/beta-report/20260526T032035Z/summary.md`
- Result: `0` failures, `0` skips
- Git revision: `d6e8beaa`
- Git status at run time: `dirty`
- Report directory: `crates/rvoip-sip/beta-report/20260526T032035Z`

Completed evidence from that run:

- Local Asterisk PBX matrix passed.
- Local FreeSWITCH PBX matrix passed.
- PBX matrix contains `192` passing rows.
- SIPp standalone matrix passed.
- baresip strict-UA matrix passed.
- Endpoint, PBX media server, and signaling-only server performance profiles
  passed.
- 30-minute soak passed: `35,010 / 35,010` calls, ASR `1.0`.
- Soak retained objects after drain: `0`.
- Soak active Bob audio receivers after drain: `0`.
- Soak RSS gate: `0.75 MB/hr` post-drain against the Config default
  `10 MB/hr` threshold.
- Peak RSS during the 30-minute soak: `312.7 MB`.

## Remaining Blocking Tasks

These tasks block beta release feature completeness.

| Area | Remaining task | Required evidence |
|------|----------------|-------------------|
| Release hygiene | Commit the current beta-gate, fuzz, redaction, metadata, docs, and report-packaging changes. | Clean git revision for the final attestation run. |
| Release hygiene | Re-run the final full beta gate from a clean commit. | New `summary.md` with `0` failures, `0` skips, and `git_status: clean`. |
| Security | Carry the audit-clean security run into the final clean report. | `security/cargo-audit.txt` and `security/cargo-audit.json` archived with no vulnerabilities or unaccepted advisories. Current Rust 1.88 short audit passes; remaining advisory output is limited to allowed/documented warnings (`async-std`, `audiopus_sys`, `paste`, `rustls-pemfile`, `yaml-rust`, `lru`). |
| Security | Run parser fuzz smoke coverage from the final gate. | `security/fuzz/sip_message.log`, `security/fuzz/uri.log`, `security/fuzz/header.log`, and `security/fuzz/sdp.log` archived with no crashes. Short smoke passed in `target/beta-gate/20260526T194243Z`. |
| Soak | Run the documented 24-hour release-candidate soak, or deliberately waive it for beta. | Soak artifact showing `duration_secs=86400`, RSS gate pass, retained objects `0`, and no stuck receivers/runners; or a checklist change accepting 30-minute evidence. |
| Release notes | Finalize `RELEASE_NOTES_NEXT.md` against the clean report. | Release notes reference the final clean beta report and contain no broad readiness, WebRTC, DTLS-SRTP, ICE, TURN, WSS outbound, carrier SBC, or general 10,000 CPS full-media claim. |

## Evidence Already Available

Use these artifacts when closing final checklist and matrix rows:

- Gate summary:
  `crates/rvoip-sip/beta-report/20260526T032035Z/summary.md`
- Environment:
  `crates/rvoip-sip/beta-report/20260526T032035Z/environment/environment.md`
- PBX matrix:
  `crates/rvoip-sip/beta-report/20260526T032035Z/pbx/matrix.tsv`
- PBX summary:
  `crates/rvoip-sip/beta-report/20260526T032035Z/pbx/summary.md`
- SIPp run summary:
  `crates/rvoip-sip/beta-report/20260526T032035Z/sipp/run_summary.md`
- SIPp analysis:
  `crates/rvoip-sip/beta-report/20260526T032035Z/sipp/analysis.md`
- baresip strict-UA summary:
  `crates/rvoip-sip/beta-report/20260526T032035Z/strict-ua/summary.md`
- Performance JSON:
  `crates/rvoip-sip/beta-report/20260526T032035Z/perf-results/`

Gate rows already covered by the latest report:

- `format check`
- `rvoip-sip all-target check`
- `claimed lower-crate check`
- `supporting SIP crate tests`
- `rvoip-sip unit tests`
- `rvoip-sip integration tests`
- `rvoip-sip doctests`
- `rvoip-sip examples compile`
- `rvoip-sip rustdoc`
- `sip-core RFC 4475 torture tests`
- `sip-core generated message validation`
- `sip dialog generated validation`
- `local Asterisk PBX matrix`
- `local FreeSWITCH PBX matrix`
- `SIPp standalone matrix`
- `baresip strict-UA matrix`
- `Kamailio/OpenSIPS proxy de-scope audit`
- `perf call setup CPS (endpoint)`
- `perf call setup CPS (pbx-media-server)`
- `perf call setup CPS (signaling-only-server-high-performance)`
- `perf registration throughput`
- `perf concurrent active calls`
- `perf RTP steady state`
- `perf backpressure step`
- `perf transport recovery`
- `perf session churn leak`
- `perf soak candidate`

## Required Non-Claims

These are not blockers if release notes and public docs do not claim them.

| Area | Beta stance |
|------|-------------|
| Kamailio/OpenSIPS/RTPengine topology | Investigation track only. Do not claim as a supported beta deployment shape. |
| Carrier SBC certification | Out of scope for beta. Library support is not certification. |
| Browser/WebRTC edge | Post-beta. |
| ICE/TURN NAT traversal | Post-beta. STUN remains limited address discovery where documented. |
| DTLS-SRTP | Post-beta. |
| WSS outbound | Not supported for beta. |
| PUBLISH end-to-end application support | Post-beta/parser-only until wired and tested. |
| Full media with Opus, G.722, or G.729 | Post-beta unless separately completed and tested. |
| General-user 10,000 CPS full-media claim | Not a beta claim. Results above 2,000 CPS must be labeled tuned or experimental with explicit configuration and caveats. |

## Definition of Beta Release Feature Complete

`rvoip-sip` is beta release feature-complete when all of the following are true:

- The final beta report is generated from a clean commit.
- `BETA_RUN_LOCAL_PBX=1 crates/rvoip-sip/scripts/beta_gate.sh --full --require-external`
  passes with `0` failures and `0` skips.
- Dependency audit output is archived and has no unaccepted advisories.
- Parser fuzz smoke output is archived for SIP message, URI, header, and SDP
  parsing.
- The documented 24-hour soak passes, or the checklist explicitly accepts the
  30-minute soak as the beta bar.
- `BETA_RELEASE_CHECKLIST.md` is fully checked or every unchecked item is an
  explicit beta non-claim.
- Release notes contain only verified claims from the final clean report.
