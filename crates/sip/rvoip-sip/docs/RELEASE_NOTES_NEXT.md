# rvoip-sip Next Release Notes Draft

Date: 2026-07-25

These notes describe the unified `0.3.0` workspace release. A development
checkpoint is not a release attestation. Keep behavioral claims only when they
are backed by the current clean beta report, compatibility matrix, RFC matrix,
interop results, security posture, and performance report.

## Headline

All 38 publishable crates now ship at one `0.3.0` version. The SIP product
retains its release-gated beta scope as a Rust-native application layer for
bounded client, server, PBX, and gateway scenarios. Optional non-SIP surfaces
remain experimental where documented.

## Unified Workspace Publication

- Every publishable crate inherits `[workspace.package].version`.
- Every internal registry dependency targets the same `0.3.0` release.
- Publication follows one Cargo-derived dependency graph rather than separate
  alpha and beta trains.
- The `0.3.0` version migration changes manifests, package resolution, and
  publication metadata; it does not rewrite the verified `0.2.5` beta
  evidence or claim that the version-only commit reran PBX, performance, or
  soak gates.

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
bytes, about 71%, removed from each hot clone). The later clean
`20260724T231400Z` qualification completed the release gate; use the generated
current reports for release-performance claims rather than the earlier
development checkpoint.

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
  CPS beta profile. Three consecutive canonical runs and the complete beta
  performance gate passed from the same clean source fingerprint.
- Higher CPS results are advanced tuned profiles and must include tuning,
  hardware, topology, and caveats.
- SIP trace output redacts authorization data, authentication challenges,
  cookies, identity headers, token-like headers, and SDP keying attributes.
- The release gate includes local tests, interop/performance modes, dependency
  audit, and parser fuzz smoke targets.

## Current Qualification Snapshot

Run `20260724T231400Z` qualifies tested commit
`8d44fb3574e40f62526aa68f19833e95274cd06b` as a beta release candidate:
108 required gates passed, with zero failures and zero skips, from a clean and
unchanged source tree. The result includes workspace unit/integration/doctests,
security, Asterisk and FreeSWITCH matrices, SIPp, baresip strict-UA, the full
performance matrix, three canonical 2K passes, high-density full-delivery
media burst, monolithic and split soaks, and final source fences.

See the [current release report](BETA_RELEASE_REPORT.md), the
[complete 108-gate report](BETA_GATE_REPORT.md), and the
[current performance report](BETA_PERFORMANCE_REPORT.md). These are post-run
reporting derivations: the candidate remains the tested commit, and later
documentation-only commits were not exercised by the run.

## Must Not Claim Yet

- Broad production readiness.
- Carrier SBC certification.
- Browser/WebRTC support.
- DTLS-SRTP, ICE, or TURN support.
- Opus, G.722, or G.729 full-media support.
- WSS outbound support.
- PUBLISH end-to-end application support.
- General-user 10,000 CPS full-media capability.

## Release Promotion Notes

- Preserve candidate identity `8d44fb35`; do not imply that reporting-only
  commits were included in the executed candidate.
- Verify the immutable report attestation and packaged v1 source attestation
  before publishing.
- Keep claims bounded by the compatibility, security, topology, RFC, and
  performance non-claims in the current documentation.
