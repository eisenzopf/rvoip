# rvoip-sip Next Release Notes Draft

Date: 2026-07-20

These notes are a draft for the beta line. A development checkpoint is not a
release attestation. Keep release claims only when they are backed by the
current clean beta report, compatibility matrix, RFC matrix, interop results,
security posture, and performance report.

## Headline

`rvoip-sip` is moving from alpha toward beta as a Rust-native SIP application
layer for bounded client, server, PBX, and gateway scenarios.

## `SessionState` Copy-on-Write Migration

The next release moves infrequently used `SessionState` fields into a shared,
copy-on-write cold block. This is an intentional pre-1.0 source break and must
ship as a minor release (for example, `0.3.0`), not as a `0.2.5` patch.

Normal constructor calls such as `SessionState::new(...)` and ordinary field
reads and writes keep their existing spelling through `Deref`/`DerefMut`.
Code that pattern-destructures a moved cold field, uses one in a struct-update
expression, or passes it to `offset_of!(SessionState, ...)` must change. Read
the field normally, or clone the state and assign the field afterward; do not
depend on `SessionState` field offsets.

The measured inline layout fell from `1,984` bytes to `576` bytes (`1,408`
bytes, about 71%, removed from each hot clone). The 2026-07-20 qualification
work also demonstrated materially improved throughput, latency, and peak RSS,
but it did not complete the release gate because cleanup and soak failures
remain. The layout result and current performance measurements are engineering
evidence, not final release-performance qualification.

## Exact Outbound Request Event Contract

The next beta source line also extends the public `DialogToSessionEvent`
contract used between dialog-core and session-core:

- `AuthRequired` now carries the exact challenged `transaction_id` and
  `request_uri`. Digest authentication signs that Request-URI, so consumers
  must use these fields instead of reconstructing either value from mutable
  session or dialog metadata.
- `OutboundRequestCompleted` and `OutboundRequestOutcome` report the exact
  terminal result of tracked outbound INFO, REFER, NOTIFY, and UPDATE attempts:
  final response, timeout, or transport failure.
- A 491 response to a re-INVITE remains `ReinviteGlare` and enters the
  re-INVITE retry state machine. A 491 response to UPDATE is an exact
  `OutboundRequestCompleted` result, allowing UPDATE-owned retained state to
  be released without reusing the re-INVITE retry path.

This is an intentional pre-1.0 source break: code constructing or
destructuring `AuthRequired` must account for the new fields, and exhaustive
matches over `DialogToSessionEvent` must account for the new variant. It is
therefore part of the next minor beta revision (the planned `0.3.0` line), not
a patch release. Missing new `AuthRequired` fields remain accepted when
deserializing older serialized events through their defaults; transaction and
Request-URI values remain private signaling metadata and are redacted from the
custom debug representation.

## Pending Credential-Zeroization Design

Owner-level zeroization still needs one end-to-end design covering staged
options and every dialog, request, and header copy of authentication material.
That design must preserve public move/struct-update APIs or intentionally
version their source break. The experimental partial `Drop` patch was rejected
and is not release or security evidence.

## Beta-Scope Claims

- Public API surfaces are centered on `Endpoint`, `StreamPeer`,
  `CallbackPeer`, `UnifiedCoordinator`, and `SessionHandle`.
- Beta media support is limited to PCMU, PCMA, telephone-event DTMF, optional
  comfort noise, RTP, and tested SDES-SRTP profiles.
- Interop evidence covers local Asterisk, local FreeSWITCH, SIPp standalone
  load scenarios, and baresip strict-UA behavior in the current reference
  report.
- General full-media performance claims remain capped at the documented 2,000
  CPS beta profile. The current measured point is provisional until three
  consecutive canonical runs and the complete beta performance gate pass from
  the same clean source fingerprint.
- Higher CPS results are advanced tuned profiles and must include tuning,
  hardware, topology, and caveats.
- SIP trace output redacts authorization data, authentication challenges,
  cookies, identity headers, token-like headers, and SDP keying attributes.
- The release gate includes local tests, interop/performance modes, dependency
  audit, and parser fuzz smoke targets.

## Current Qualification Snapshot — Not a Release Attestation

The latest qualification bundle was generated on 2026-07-20 from base
revision `85b932e4` with source-tree fingerprint
`20f57cedfc2c6691e2f872b6aa505345cac690d34b6f4aa288bbe4f5abb41461`.
The tree was dirty because the candidate changes were staged but not committed,
so this evidence cannot serve as a clean-commit release attestation.

Passing evidence from that bundle includes:

- Interoperability completed with `0` failures and `0` skips. Asterisk and
  FreeSWITCH passed their PBX matrices with real bidirectional spectral audio;
  the SIPp ladder passed through 2,000 CPS; and baresip strict-UA passed.
- Dependency audit and all ten parser fuzz-smoke targets passed, subject to the
  explicitly documented accepted transitive advisories.
- The canonical 2,000-CPS point completed 65,000 of 65,000 calls at 1,857.11
  achieved CPS, ASR/NER `1.0`, setup p50/p95/p99
  `0.647/0.926/4.960 ms`, full-cycle p99 `7.881 ms`, zero call errors, and
  1,375.19 MB peak RSS.
- Twenty-three performance and resiliency stages passed before the monolithic
  soak stopped the remaining sequence.

The candidate is still blocked by:

- Six retained `transaction_dialog_route_hash` entries after the canonical
  run. Cleanup convergence therefore failed, and the reported cleanup-endpoint
  RSS rate was 29.94 MB/hour against the 10 MB/hour gate.
- The 30-minute real-media soak completed 5,012 of 5,016 calls and processed
  42,873,129 received audio frames, but failed with three media-setup errors,
  one teardown error, one live session/audio receiver after the 120-second
  drain, and 27 retained objects.
- Two BYE-dispatch failures observed in the high-rate signaling-only run and
  short sustained-call setup/tail failures that require resolution or an
  explicitly reviewed qualification decision.
- The burst matrix and split one-hour soak did not run after the monolithic
  soak failure. Three consecutive clean canonical runs have not been archived.

The earlier 2026-05-26 report at
`crates/sip/rvoip-sip/beta-report/20260526T221457Z/summary.md` is historical
evidence for revision `865430d4`; it does not attest the current candidate.

## Must Not Claim Yet

- Broad production readiness.
- Carrier SBC certification.
- Browser/WebRTC support.
- DTLS-SRTP, ICE, or TURN support.
- Opus, G.722, or G.729 full-media support.
- WSS outbound support.
- PUBLISH end-to-end application support.
- General-user 10,000 CPS full-media capability.
- A clean, leak-free beta release-candidate qualification for the current
  source tree.

## Evidence Required Before Release

- Commit the reviewed candidate and run every gate from that clean revision.
- Archive three consecutive canonical 2,000-CPS passes with the same source
  fingerprint and all absolute/regression checks green.
- Resolve the route-hash and live-media cleanup defects, then pass the complete
  monolithic soak, burst matrix, and split caller/receiver soak sequence.
- Re-run and archive local, security, interoperability, PBX, SIPp, and strict-UA
  evidence without source changes between phases.
- Update `COMPATIBILITY_MATRIX.md`, `RFC_COMPLIANCE_MATRIX.md`,
  `BETA_PERFORMANCE_REPORT.md`, and `SECURITY_POSTURE.md` with links to the
  resulting immutable evidence bundle.
- Require `scripts/beta_gate.sh --full --require-external` to pass with no
  failures or hidden skips before describing the candidate as beta-ready.
