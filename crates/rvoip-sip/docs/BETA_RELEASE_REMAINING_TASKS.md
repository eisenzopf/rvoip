# rvoip-sip Beta Release Remaining Tasks

Date: 2026-05-26

This document tracks what remains before `rvoip-sip` can be called beta release
feature-complete under the requirements in the beta docs. It is intentionally
conservative: an item is complete only when there is linked release evidence,
an explicit non-claim, or an accepted exclusion.

## Current Status

`rvoip-sip` has a strong beta-candidate gate result, but the release is not yet
fully attested against every beta checklist item.

Latest complete no-skip gate:

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
| Release hygiene | Commit the current beta-gate, Config/profile, docs, and report-packaging changes. | Clean git revision for the final attestation run. |
| Release hygiene | Re-run the final full beta gate from a clean commit. | New `summary.md` with `0` failures, `0` skips, and `git_status: clean`. |
| Soak | Run the documented 24-hour release-candidate soak. | `perf_soak_30min.json` or successor soak artifact showing `duration_secs=86400`, RSS gate pass, retained objects `0`, and no stuck receivers/runners. |
| Security | Run dependency advisory audit. | `cargo audit` output, or documented equivalent, archived in the beta report with no unaccepted advisories. |
| Security | Run parser fuzz smoke coverage. | Archived fuzz-smoke logs for SIP message parsing, URI parsing, header parsing, and SDP parsing. |
| Security | Close the `SECURITY_POSTURE.md` required checks. | Evidence links for digest auth, TLS client/server, trace redaction, SDES-SRTP limits, dependency audit, fuzz smoke, and unsupported security-mode errors/non-claims. |
| RFC compliance | Convert `RFC_COMPLIANCE_MATRIX.md` from a requirement matrix into an evidence-linked matrix. | Each claimed RFC row links to unit tests, generated validation, RFC 4475 logs, SIPp, PBX, baresip, or an explicit non-claim. |
| Compatibility | Convert `COMPATIBILITY_MATRIX.md` supported/interop-tested rows into evidence-linked rows. | Evidence links for each beta-supported API surface, method, transport, media feature, and performance profile. |
| Performance | Populate `BETA_PERFORMANCE_REPORT.md` with actual results. | Tables and artifact links for endpoint, PBX media server, signaling-only server, SIPp, RTP steady state, soak, RSS, CPU, latency, config, and environment. |
| Checklist | Update `BETA_RELEASE_CHECKLIST.md` from unchecked placeholders to evidence-backed checked items. | Every checked item links to report evidence; every unchecked item is either a blocker or a documented non-claim. |
| Release notes | Finalize `RELEASE_NOTES_NEXT.md` with verified claims only. | Release notes reference the final clean beta report and contain no broad production-ready, WebRTC, DTLS-SRTP, ICE, TURN, WSS outbound, carrier SBC, or general 10,000 CPS full-media claim. |
| API packaging | Confirm MSRV and semver policy are documented. | Public docs state the beta MSRV and semver policy. |
| API packaging | Confirm feature flags and supported combinations are documented. | Public docs list beta feature flags, test status, and unsupported combinations. |

## Evidence Already Available

Use these artifacts when closing checklist and matrix rows:

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
- Perf JSON:
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
| 24-hour production soak for production release | Required before production; for beta release, the docs currently require it as release-candidate evidence and it remains a blocker unless deliberately waived in the checklist. |

## Definition of Beta Release Feature Complete

`rvoip-sip` is beta release feature-complete when all of the following are true:

- The final beta report is generated from a clean commit.
- `BETA_RUN_LOCAL_PBX=1 crates/rvoip-sip/scripts/beta_gate.sh --full --require-external`
  passes with `0` failures and `0` skips.
- The documented 24-hour soak passes or the checklist is explicitly changed to
  accept the 30-minute soak as the beta bar.
- Dependency audit output is archived and has no unaccepted advisories.
- Parser fuzz smoke output is archived for SIP message, URI, header, and SDP
  parsing.
- `BETA_RELEASE_CHECKLIST.md` is fully checked or every unchecked item is an
  explicit beta non-claim.
- `BETA_PERFORMANCE_REPORT.md` contains real values and links to raw artifacts.
- `RFC_COMPLIANCE_MATRIX.md`, `COMPATIBILITY_MATRIX.md`, and
  `SECURITY_POSTURE.md` link supported claims to evidence.
- Release notes contain only verified claims from the final report.

