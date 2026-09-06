# rvoip 0.3.10 release hardening plan

Status: in progress, 2026-09-05.

## Purpose

0.3.10 is a patch release that corrects the release-quality gaps discovered
after 0.3.9 without introducing a new compatibility boundary. The 0.3.9 tag,
crates, release, and qualification evidence remain immutable. This release
must be qualified from a fresh exact candidate after the changes below land.

## Exit criteria

0.3.10 may be published only when all of the following are true:

1. Every active package README and installation example names the workspace
   release version. Historical release records keep their original versions.
2. Release preparation discovers and updates every declared active metadata
   file, and CI fails when a current-version marker or package dependency
   example drifts.
3. Every open CodeQL alert on the candidate is either fixed or individually
   adjudicated with a durable, reviewable reason. New critical/high alerts
   block merge and publication.
4. Cryptographic key material, credentials, authorization values, and tokens
   are never printed or included in `Debug`/`Display` output. Redaction tests
   cover the public diagnostic boundaries.
5. The Jambonz interoperability profile below passes against pinned upstream
   source and container identities.
6. The ordinary PR gate, all feature-bundle gates, the full 45-crate matrix,
   CodeQL, and a fresh `remote-release` qualification pass on the exact
   candidate.
7. The coordinated 45-crate dry run and protected publication operate on the
   same qualified commit. Generated evidence is committed in a follow-up
   evidence-only pull request.
8. A new performance evaluation runs against the exact 0.3.10 candidate. It
   includes three fresh canonical 2,000-CPS passes, the full performance and
   resiliency matrix, high-density media burst, monolithic and split one-hour
   soaks, regression comparison, and a machine-readable current-result index.
   July artifacts remain historical baselines and cannot substitute for this
   release's measurements.

## Workstream A: release metadata integrity

- Correct stale 0.3.8 current-release statements and dependency snippets in
  active crate documentation. Do not rewrite historical assessments,
  exceptions, or prior-release measurements.
- Expand `ACTIVE_RELEASE_METADATA_FILES` to cover every package README and
  active release-policy document that carries the current train number.
- Add a repository policy check that reads the workspace version and verifies:
  - every declared active file contains the current version;
  - dependency examples for publishable workspace crates use that version;
  - the manifest contains no missing or duplicate path; and
  - historical files are explicit rather than silently excluded.
- Test preparation from 0.3.9 to 0.3.10 and a deliberately stale README.

## Workstream B: CodeQL and secret diagnostics

- Remove all production logging of DTLS pre-master secrets, master secrets,
  derived keying material, bearer/API tokens, and credential-bearing objects.
- Review each non-test alert as a dataflow problem rather than dismissing it by
  filename. Fix the path, cleartext transmission, certificate, and workflow
  cache findings or prove the boundary safe.
- Classify test/example constants separately. Test credentials, RFC known
  answers, packet fixtures, and deterministic nonces may be adjudicated only
  when they cannot be selected by production code and the value is visibly a
  fixture. Realistic-looking secrets are replaced with explicit fixture
  constructors where practical.
- Record every adjudication against its stable GitHub alert number and retain
  an exact-candidate machine-readable policy receipt with the release
  evidence. The checked-in verifier rejects a stale analysis or any alert
  that remains open; GitHub's per-alert audit history is the durable ledger.
- Add CodeQL merge protection for critical/high security alerts after the
  existing baseline is resolved. A successful scanner execution is not a
  substitute for a zero-unreviewed-alert policy.

## Workstream C: Jambonz interoperability

### What “Jambonz interoperability” means for 0.3.10

The release claim is a SIP-trunk interoperability claim between RVoIP and the
open-source Jambonz SBC path. It is not a claim that RVoIP implements the
Jambonz application verb API or replaces any Jambonz component.

The mandatory deterministic profile uses the latest stable open-source
Jambonz release available when qualification begins. For the initial 0.3.10
implementation that is the 0.9.9 component line. Jambonz Commercial 10.2.0 is
a distinct licensed product line and is not silently substituted for the
open-source release. A separately credentialed commercial run may supplement
the open-source gate but does not replace it.

Each qualification resolves the selected latest release once, then uses:

- an exact pinned `jambonz/sbc-outbound` source revision whose package version
  matches the selected release, plus a freshness assertion across the paired
  inbound/outbound open-source component line;
- pinned Drachtio, RTPengine, MySQL, Redis, InfluxDB, and customer-auth
  container manifest digests;
- an ephemeral isolated Docker network and database;
- the same digest-authenticated registered RVoIP endpoints used by the shared
  PBX matrix; and
- the same RVoIP caller, callee, and transfer-target processes used by the
  Asterisk and FreeSWITCH interoperability matrix.

Jambonz is implemented as a provider in
`crates/sip/rvoip-sip/examples/pbx`, not as a separate protocol test suite.
Each two-party cell places RVoIP on both sides of the Jambonz signaling and
anchored-media boundary, exercising RVoIP in both UAC/caller and UAS/callee
roles through the same orchestration and evidence format used for Asterisk and
FreeSWITCH.

The required scenario matrix is:

| Area | Required evidence |
|---|---|
| Admission | Authenticated registered users accepted; bad credentials rejected |
| Dialog | INVITE, 100/180, 200, ACK, and route/contact correctness |
| SDP/media | PCMU and PCMA negotiation; RTP in both directions; continuous audio assertion |
| DTMF | RFC 4733 event crosses the anchored media path |
| Cancellation | CANCEL after provisional response produces 487 and settles both legs |
| Teardown | BYE initiated from each side and no surviving dialog/media allocation |
| Mid-dialog | re-INVITE hold/resume with restored bidirectional media |
| Transfer | RFC 3515 blind REFER; local acceptance completion; Referred-By preservation when supplied; ordered 100/180/terminal NOTIFY; replacement INVITE and teardown |
| Evidence | redacted cell transcripts, media assertions, Jambonz commit, image digests, matrix, and teardown inventory |

The 0.3.10 claim deliberately excludes TLS/SRTP, WebRTC, Jambonz speech/AI
verbs, hosted jambonz.cloud, carrier PSTN connectivity, HA/failover, recording,
and load capacity. Those require separate profiles. A licensed Jambonz-mini
rehearsal may supplement the release but cannot replace the deterministic
profile or silently broaden its claim.

### Harness and release integration

- Add `jambonz` to the existing `rvoip-sip` PBX/SBC harness as a distinct,
  mandatory B2BUA peer at
  the same release-policy level as Asterisk and FreeSWITCH; do not treat it
  as an Asterisk-compatible alias, an optional proxy lab, or a parallel
  Jambonz-only scenario implementation.
- Add a freshness check that compares the pinned open-source component line
  with the latest stable upstream line. A newer stable version fails closed
  and requires a reviewed pin update before qualification; no gate pulls a
  floating `latest` tag while calls are running.
- Add isolated `jambonz-up`, readiness, evidence, and `jambonz-down` lifecycle
  actions. Cleanup must run after failure and prove that every container,
  network, and generated credential is gone.
- Pin upstream source and every image by immutable digest. A moved tag or
  inaccessible artifact fails the gate.
- Add dedicated release gate IDs rather than silently expanding the meaning of
  the existing Asterisk/FreeSWITCH matrix.
- Exercise blind transfer through Endpoint, StreamPeer, and CallbackBuilder.
  CallbackBuilder uses its local inbound-REFER acceptance hook to sequence the
  replacement request; `on_transfer_accepted` retains its distinct meaning for
  acceptance of an outbound REFER by a remote peer.
- Put Jambonz on an appropriately sized ephemeral interop worker and update
  quota/cost documentation. The worker receives no release or carrier secret.
  The pinned legacy Jambonz dependencies require an amd64 engine: the release
  profile runs on the existing x86 GCP interop worker, while Apple Silicon
  developers must use an x86_64 Colima profile with its gRPC UDP forwarder
  rather than treating architecture or SSH-forwarding failures as SIP
  failures. A live UDP probe enforces that prerequisite. The Colima topology
  explicitly publishes the SIP listener and bounded media range on loopback
  and advertises the Colima host gateway for return SIP and RTP; the Linux
  release topology continues to use its private bridge.

## Workstream D: release governance and evidence clarity

- Make CodeQL critical/high results a required merge signal after baseline
  resolution.
- Require the release report and plan to say “qualified for the recorded
  profile” instead of an unbounded “fully qualified.”
- Explain the implementation PR, preparation PR, dry-run publication, live
  publication, and evidence PR as separate immutable stages.
- Label or close stale pull requests so failed future-work branches cannot be
  mistaken for release failures. Historical Actions runs remain visible and
  are not rewritten.
- Document that release tags are protected annotated tags and that evidence is
  artifact-attested. Cryptographically signed tags are a separate supply-chain
  enhancement until explicitly required.

## Workstream E: current performance evaluation

- Run three chronological clean 2,000-CPS PBX/media-server passes on the exact
  candidate and require a byte-identical executable, source fingerprint, full
  audio delivery, at least 99.9% ASR, zero non-timeout errors, and complete
  post-drain cleanup.
- Run the existing call-setup, registration, concurrency, RTP, backpressure,
  recovery, mid-call signaling, TLS/SRTP overhead, PDD, long-call, registrar,
  mixed, B2BUA, AI-agent, contact-center, SIPp-parity, churn, teardown, burst,
  and soak gates as fresh 0.3.10 measurements.
- Generate a current-candidate performance evaluation in JSON and Markdown,
  plus a SHA-256 index of every packaged performance artifact. Require the
  160-CPS high-density media burst, 99.5% minimum ASR, 15 MB/hour RSS-slope
  ceiling, 3,600-second 30-call monolithic soak, 3,600-second 500-call split
  soak, and regression audit to pass without threshold reinterpretation.
- Record the worker shape, source commit, environment identity, effective
  workload, setup-latency percentiles, completion/error counts, CPU/RSS,
  delivered-media counts, drain state, and artifact hashes in the protected
  qualification bundle. The release notes must link the resulting run and
  summarize the measured values before publication.

## Verification sequence

1. Run formatting, release-tooling unit tests, metadata-drift tests, redaction
   tests, and targeted security tests locally.
2. Run the Jambonz profile alone and preserve its teardown receipt.
3. Open the implementation pull request and require PR Gate plus CodeQL.
4. Merge normally; do not use an administrative bypass.
5. Run remote preflight for the changed worker topology.
6. Run a fresh complete `remote-release` qualification with
   `first_candidate=true`. Evidence reuse from 0.3.9 is not accepted for this
   hardening release, and the new current-candidate performance evaluation is
   mandatory.
7. Prepare and merge the coordinated 0.3.10 version PR if it was not already
   the exact qualified candidate, then rerun qualification on the final SHA.
8. Run protected publication dry-run, then live publication.
9. Verify all 45 crates and docs on crates.io, create the protected tag and
   GitHub release, and merge the generated evidence-only PR.

## Non-goals

- No public API break is planned; any required break stops this patch release
  and triggers an explicit 0.4.0 decision.
- No Telnyx-specific behavior enters RVoIP.
- No Jambonz REST/application-verb client enters the RVoIP runtime.
- No historical release artifact or report is rewritten.
