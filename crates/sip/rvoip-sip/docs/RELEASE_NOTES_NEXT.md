# rvoip 0.3.10 Release Notes

These notes describe the coordinated 45-crate `0.3.10` release. It was
published from the exact source commit accepted by the protected release
qualification and publication workflows. The signed qualification artifact
and GitHub release identify that immutable commit, workflow run, complete gate
inventory, measured performance, and publication result. This evidence-only
follow-up copies those generated reports into the repository without changing
the released tag.

## Headline

`0.3.10` is a patch release that hardens the `0.3.9` feature set and adds
Jambonz OSS as a mandatory external SIP interoperability peer. It corrects
REFER/NOTIFY ordering under concurrent transfer progress, preserves an
optional `Referred-By` header across a Jambonz-mediated transfer, closes
security-diagnostic leaks, and strengthens release metadata and feature-bundle
gates. It does not expand the supported deployment boundary beyond the
documented SIP, SDP, RTP, codec, and facade profiles.

## Jambonz interoperability

- Jambonz is tested as an independent SBC/B2BUA, registrar, and RTPengine
  media anchor using the same PBX runner and the same `Endpoint`, `StreamPeer`,
  and `CallbackPeer` APIs used for Asterisk and FreeSWITCH.
- The release profile pins the latest reviewed open-source component line,
  currently Jambonz OSS `0.9.9`: `sbc-inbound` commit
  `b7b707cc2e2a1025623076f16446ea61bae429e0` and `sbc-outbound` commit
  `fec25d5d1539cdcb80ef8e8b8fc0bc090319dd27`. Source archives and every
  container are digest-verified, and qualification fails if those component
  pins are no longer the selected upstream heads.
- The mandatory UDP/plain-RTP matrix covers authenticated registration,
  separate PCMU and PCMA calls with bidirectional audio, provisional and final
  call signaling, hold/resume, RFC 4733 DTMF, CANCEL/487, rejection,
  REFER/NOTIFY blind transfer, replacement INVITE, BYE from either side, and
  resource cleanup.
- G.729, AMR, TLS/SRTP, the RVoIP-as-B2BUA scenario, WebRTC, PSTN,
  application verbs, recording, high availability, and load are explicit
  exclusions from this Jambonz profile. Codec and transport support elsewhere
  in RVoIP is not reduced by those peer-specific exclusions.
- The local Colima rehearsal passed every applicable Jambonz matrix cell. That
  rehearsal is diagnostic evidence only. Protected publication independently
  requires the exact release commit to record the complete matrix as PASS.

## Transfer correctness

- RFC 3515 blind transfer now serializes the implicit subscription's initial
  `100 Trying`, progress, and terminal NOTIFY requests on the exact REFER
  lifecycle. A later status cannot overtake an earlier NOTIFY transaction,
  and stale, duplicate, or regressive statuses are suppressed.
- `Refer-To` remains mandatory for REFER. RFC 3892 `Referred-By` remains
  optional; when a peer supplies it, RVoIP preserves the typed header unchanged
  into the referenced request. Jambonz qualification verifies that behavior
  without incorrectly making `Referred-By` a universal requirement.
- `CallbackPeerBuilder::on_refer_accepted` reports successful local acceptance
  of an inbound REFER. It is intentionally distinct from
  `on_transfer_accepted`, which continues to report a remote peer accepting a
  REFER originated by the application.
- Attended transfer and RFC 3891 call-replacement semantics remain outside the
  bounded release claim.

## Security and release hardening

- Sensitive authentication and key material is redacted from diagnostics and
  logging paths exercised by the release gates.
- Certificate validation, path handling, GitHub Actions cache use, and
  CodeQL release policy are fail-closed. Publication requires current analysis
  and zero unreviewed open alerts; reviewed fixture-only findings must carry an
  individual, auditable disposition.
- Facade feature-bundle tests run with default features disabled and inspect
  the resolved dependency graph. The carrier and full bundles continue to
  include G.711, G.729, AMR-NB, and AMR-WB as documented; Opus remains
  explicit in the browser, AI, and native-full bundles.
- Active version-bearing documentation is checked against the workspace
  version. The release gate also verifies that the Jambonz version, scope,
  exclusions, and qualification status remain aligned across the public
  README, SIP README, interop plan, topology profile, compatibility matrix,
  RFC matrix, changelog, and these notes.

## Performance evaluation

The protected `0.3.10` qualification executed three clean canonical 2,000-CPS
passes, the full performance and resiliency matrix, the 160-CPS high-density
full-media burst, a one-hour
30-call monolithic soak, a one-hour 500-call split soak, teardown/churn tests,
and regression comparison. It publishes structured JSON and Markdown metrics
plus a SHA-256 index of all current-run performance artifacts. July results
remain historical baselines; they cannot qualify this release.

General-user 10,000 CPS full-media capability is not claimed. The supported
envelope remains bounded by the new 2,000-CPS real-media evidence, exact host
configuration, workloads, and soak durations recorded for this release.

The signed qualification artifact is the authority for the measured host
shape, source commit, workload configuration, ASR and error counts,
setup-latency percentiles, CPU/RSS behavior, media delivery, cleanup, and
artifact hashes. The GitHub release links that artifact and its workflow run;
the repository's generated qualification history mirrors it after publication.

## Compatibility

The changes are additive for public application APIs. Existing
`on_transfer_accepted` behavior is unchanged; applications may adopt the new
local inbound-REFER hook when they need that distinct lifecycle point. There
is no Telnyx- or Jambonz-specific runtime dependency in RVoIP and no provider
REST API in the SIP stack.

The `0.3.9` tag, crates, and qualification evidence remain immutable release
history; none of that evidence is reused to qualify `0.3.10`.

## Qualification record

The protected Release Qualification workflow ran from a clean `main` commit
and recorded every selected gate exactly once as PASS, including the full
45-crate matrix, feature bundles, CodeQL policy, Asterisk, FreeSWITCH, Jambonz,
Kamailio, OpenSIPS, SIPp, strict-UA, security, performance, resiliency, soak,
regression, source-fence, and cleanup gates. It produces a signed aggregate
bound to the exact source SHA and immutable artifact hashes.

The protected Release Publish workflow accepted that aggregate after verifying
that its source SHA was still `main`, its version was exactly `0.3.10`, its
evidence was fresh and complete, and the dry-run and live publication inputs
resolved to the same commit. The resulting GitHub release is the durable link
to the exact qualification run and measured reports; the protected `v0.3.10`
tag identifies the source without a self-referential documentation commit.
