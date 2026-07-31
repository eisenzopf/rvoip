# rvoip Unified Remote Release Gate Automation Plan

Status: Proposed for owner review  
Target migration: rvoip 0.3.5  
Prepared: 2026-07-31  
Scope: the complete 44-crate rvoip workspace and every gate required for a
coordinated full release

## Executive decision

The recommended release system uses GitHub Actions as the control plane and
ephemeral Google Cloud Compute Engine VMs as remote workers.

The release process will:

1. Qualify the entire 44-crate workspace, not only rvoip-sip.
2. Give every gate a permanent number, description, command, dependency set,
   runner contract, and independent attestation.
3. Run the mandatory workspace and package gates freshly for every candidate
   commit.
4. Run those mandatory gates in parallel shards instead of one monolithic
   process.
5. Reuse expensive external, browser, interoperability, performance, and soak
   evidence only when exact input and dependency analysis proves it remains
   valid.
6. Resume an interrupted qualification from its durable remote attestations.
7. Provision clean Google Cloud VMs only while work exists and delete them
   after one job.
8. Use an n2-standard-32 Ubuntu x64 VM with 32 vCPU and 128 GB RAM plus a
   1,200 GB SSD work disk for performance, canonical 2K, and soak work.
9. Keep crates.io publication in a separate protected workflow after every
   selected gate is independently VALID_PASS.

This replaces the existing all-day local monolith without weakening the
coordinated release requirement.

## The retest policy

All 44 publishable crates move to one version. That does require fresh
release-surface verification for every crate on the final candidate commit. It
does not mean that a mechanical version change must automatically rerun every
hour-long soak or every external peer matrix.

The recommended conservative rule is:

### Always fresh after any candidate code change

- clean source and candidate identity;
- the complete 44-crate metadata and dependency graph;
- formatting and strict lint gates;
- default workspace tests;
- workspace target and integration tests;
- workspace doctests;
- required all-feature and feature-matrix tests;
- per-crate compile/test result for all 44 crates;
- Tokio-only and forbidden-runtime checks;
- release tooling tests;
- package manifest validation for all 44 crates;
- cargo package or equivalent registry-resolution validation for all 44
  crates; and
- final aggregate/source verification.

These gates are intentionally rerun for every candidate commit. They are
parallelized remotely, so this conservative safety rule does not recreate the
local monolith.

### Reused only when exact inputs are unchanged

- Asterisk and FreeSWITCH matrices;
- SIPp and strict-UA matrices;
- Kamailio and OpenSIPS stateful proxy matrices;
- Chromium and other browser interoperability;
- libSRTP and other external-library interoperability;
- TURN/public-NAT qualification;
- MOQT network qualification;
- high-density performance profiles;
- canonical 2K evidence;
- long-duration performance and leak tests; and
- one-hour monolithic and split soaks.

Any source, feature, harness, peer, runtime, topology, threshold, or dependency
change relevant to one of these gates invalidates it and schedules it again.

### Version-only release preparation

A mechanical workspace version bump and exact internal dependency version
update trigger all fresh workspace/package gates. Expensive runtime gates may
remain valid only when:

- the changed paths are limited to an allowlisted release-metadata delta;
- semantic source and runtime configuration digests are identical;
- a dedicated version-surface gate verifies every location that exposes
  CARGO_PKG_VERSION or another package version at runtime;
- package manifests and lockfile resolution are valid for all 44 crates; and
- no feature, dependency source, dependency version other than the coordinated
  internal release number, build script, generated code, protocol identity, or
  runtime behavior changed.

An unknown or ambiguous delta invalidates the affected gate closure or fails
closed. It is never silently treated as version-only.

## Why this is not a weaker release

A full release remains an all-workspace release:

- every crate is freshly compiled and tested;
- every crate is freshly packaged and registry-checked;
- all release-wide policy gates are fresh;
- every selected specialty gate is either freshly run or proven byte-for-byte
  input-equivalent;
- the final aggregate binds all evidence to the final release candidate; and
- one missing, stale, expired, failed, or conflicting gate blocks publication.

The optimization comes from remote parallelism and exact evidence reuse, not
from skipping coverage.

## Current workspace

The release authority currently requires exactly 44 publishable packages.

| Number | Package | Family |
| --- | --- | --- |
| C01 | rvoip | facade |
| C02 | rvoip-client | facade/client |
| C03 | rvoip-core | foundation |
| C04 | rvoip-core-traits | foundation |
| C05 | rvoip-infra-common | foundation |
| C06 | rvoip-media-core | media |
| C07 | rvoip-codec-core | media |
| C08 | rvoip-rtp-core | media/security |
| C09 | rvoip-audio-device | media/device |
| C10 | rvoip-sip | SIP |
| C11 | rvoip-sip-core | SIP |
| C12 | rvoip-sip-transport | SIP |
| C13 | rvoip-sip-dialog | SIP |
| C14 | rvoip-sip-proxy | SIP |
| C15 | rvoip-sip-registrar | SIP |
| C16 | rvoip-rtc | WebRTC/RTC |
| C17 | rvoip-webrtc-stack | WebRTC/RTC |
| C18 | rvoip-webrtc | WebRTC/RTC |
| C19 | rvoip-amazon-connect | WebRTC/integration |
| C20 | rvoip-uctp | UCTP |
| C21 | rvoip-quic | UCTP |
| C22 | rvoip-webtransport | UCTP |
| C23 | rvoip-websocket | UCTP |
| C24 | rvoip-moq-transport | MOQT |
| C25 | rvoip-moq-native | MOQT |
| C26 | rvoip-moq-relay | MOQT |
| C27 | rvoip-moq | MOQT |
| C28 | rvoip-auth-core | identity |
| C29 | rvoip-users-core | identity |
| C30 | rvoip-identity | identity |
| C31 | rvoip-vcon | extension |
| C32 | rvoip-vcon-postgres | extension/live service |
| C33 | rvoip-harness | extension |
| C34 | rvoip-vapi | extension/network |
| C35 | rvoip-stir-shaken | extension/security |
| C36 | rvoip-keycloak | extension/live service |
| C37 | rvoip-redis | extension/live service |
| C38 | rvoip-audit | extension |
| C39 | rvoip-oidc | extension/security |
| C40 | rvoip-ldap | extension/live service |
| C41 | rvoip-scim | extension/network |
| C42 | rvoip-saml | extension/security |
| C43 | rvoip-webauthn | extension/security |
| C44 | rvoip-ims-aka | extension/security |

The catalog generator verifies this list against cargo metadata and
scripts/release.py. An added, removed, renamed, or newly non-publishable crate
requires a reviewed catalog update.

## Gate model

### Stable release gate ranges

| Range | Scope |
| --- | --- |
| RG001-RG099 | candidate, source, policy, and environment |
| RG100-RG199 | workspace-wide build, lint, test, documentation, and API |
| RG200-RG299 | foundation, media, codec, RTP, SRTP, and security specialties |
| RG300-RG399 | SIP product and SIP external interoperability |
| RG400-RG499 | RTC, WebRTC, UCTP, WebSocket, WebTransport, QUIC, and MOQT |
| RG500-RG599 | identity, extensions, databases, providers, and live services |
| RG600-RG699 | browser, cross-product, external-library, and network interop |
| RG700-RG799 | performance, resiliency, leak, load, and soak |
| RG800-RG899 | package graph, registry dry run, checksums, and publication |
| RG900-RG999 | evidence collection, reporting, approval, and release receipt |

Existing rvoip-sip beta gate IDs remain stable aliases during migration. The
root release catalog imports them rather than creating a second contradictory
SIP policy.

### Per-crate mandatory gates

Every C01-C44 package has these stable sub-gates:

| Suffix | Meaning |
| --- | --- |
| .1 | manifest, metadata, dependency, feature, and target validation |
| .2 | default compile and test |
| .3 | required feature matrix or all-feature compile/test |
| .4 | documentation and public API validation where applicable |
| .5 | package file manifest, archive, and registry-resolution validation |

For example:

- C08.2 is the fresh default rvoip-rtp-core test gate.
- C08.3 is its required SRTP/feature matrix.
- C08.5 is its final package/archive gate.
- C32.3 is the rvoip-vcon-postgres live-service feature gate.

Each sub-gate emits its own result even when several sub-gates share one worker
job or compilation cache.

### Minimum release-wide gates

The initial root catalog must include at least:

#### Source and policy

- RG001 candidate commit and clean source;
- RG002 44-crate inventory;
- RG003 dependency DAG and unified version;
- RG004 gate policy validation;
- RG005 toolchain and target identity;
- RG006 exact lockfile and registry-source audit;
- RG007 runtime architecture policy;
- RG008 final source and aggregate reconciliation.

#### Workspace-wide

- RG101 formatting;
- RG102 release-tooling tests;
- RG103 default workspace library tests;
- RG104 workspace binary/example/integration tests;
- RG105 workspace doctests;
- RG106 workspace all-target check;
- RG107 required all-feature compile/test;
- RG108 strict Clippy;
- RG109 cargo-deny/license/source policy;
- RG110 public API compatibility;
- RG111 examples and downstream consumer matrix;
- RG112 Tokio-only dependency/runtime gate;
- RG113 no Smol or alternate-runtime dependency gate;
- RG114 minimum supported Rust version check;
- RG115 current stable Rust check;
- RG116 documentation build;
- RG117 generated-code and schema consistency;
- RG118 version-surface validation.

#### Foundation, media, and security

- RG201 foundation crate matrix;
- RG202 media-core and codec feature matrix;
- RG203 Opus feature matrix;
- RG204 G.722 unsupported-behavior contract;
- RG205 RTP/RTCP correctness;
- RG206 SRTP/SRTCP known-answer vectors;
- RG207 libSRTP external interoperability;
- RG208 SDES directional-key behavior;
- RG209 security fail-closed profiles and DTLS unsupported behavior;
- RG210 dependency advisory audit;
- RG211 independent parser fuzz targets.

#### SIP

- RG301 complete rvoip-sip policy-selected unit/integration/doctest catalog;
- RG302 signaling-only hold/resume;
- RG303 SIP renegotiation call flows;
- RG304 registrar behavior;
- RG305 proxy behavior;
- RG306 Asterisk matrix;
- RG307 FreeSWITCH matrix;
- RG308 SIPp matrix;
- RG309 strict-UA matrix;
- RG310 Kamailio/OpenSIPS stateful proxy matrix;
- RG311 SIP cleanup and lifecycle invariants;
- RG312 SIP architecture performance invariants.

#### RTC, WebRTC, UCTP, and MOQT

- RG401 RTC unit and integration matrix;
- RG402 WebRTC Tokio default/no-default/all-feature matrix;
- RG403 Chromium interoperability;
- RG404 WebRTC malformed SDP regression;
- RG405 UCTP matrix;
- RG406 QUIC matrix;
- RG407 WebTransport matrix;
- RG408 WebSocket media/TLS matrix;
- RG409 Amazon Connect boundary;
- RG410 MOQT transport/native/relay matrix;
- RG411 MOQT publisher lease and revocation behavior;
- RG412 MOQT external network qualification.

#### Identity and extensions

- RG501 identity and auth-core matrix;
- RG502 users-core matrix;
- RG503 OIDC, SAML, SCIM, WebAuthn, and IMS-AKA matrix;
- RG504 STIR/SHAKEN matrix;
- RG505 Redis live-service matrix;
- RG506 PostgreSQL live-service matrix;
- RG507 Keycloak live-service matrix;
- RG508 LDAP live-service matrix;
- RG509 vCon model/signing/store matrix;
- RG510 Vapi and harness matrix;
- RG511 audit extension matrix.

#### Cross-product and browser

- RG601 facade and client consumer smoke;
- RG602 standalone example suite;
- RG603 browser smoke;
- RG604 exact built-SDK Chromium destinations where selected;
- RG605 TURN-only and public-NAT scenarios where selected;
- RG606 cross-family integration graph.

#### Performance and resiliency

- RG701 canonical 2K three-pass evidence;
- RG702 call-setup profile matrix;
- RG703 registration throughput;
- RG704 concurrent calls;
- RG705 RTP steady state;
- RG706 backpressure and overload;
- RG707 transport recovery;
- RG708 mid-call signaling under media;
- RG709 TLS and SRTP overhead;
- RG710 registrar scale;
- RG711 mixed workload;
- RG712 B2BUA and contact-center workload;
- RG713 media churn;
- RG714 session churn and leak detection;
- RG715 mass teardown;
- RG716 high-density media burst;
- RG717 monolithic one-hour soak;
- RG718 split high-concurrency one-hour soak;
- RG719 regression baseline verification;
- RG720 performance evidence reconciliation.

#### Packaging and publication

- RG801 release audit;
- RG802 coordinated prepare dry run;
- RG803 all 44 package manifests;
- RG804 all 44 package archives;
- RG805 crates.io registry-only dependency graph simulation;
- RG806 publication topological order;
- RG807 crates.io publish dry run;
- RG808 checksum inventory;
- RG809 partial-publication resume simulation;
- RG810 no-tag-before-complete-publication rule.

#### Reporting

- RG901 per-gate schema validation;
- RG902 provenance and artifact hash verification;
- RG903 selected-gate completeness;
- RG904 source/reuse reconciliation;
- RG905 release report generation;
- RG906 release verifier integration;
- RG907 publication approval receipt.

The implementation PR expands this minimum into the exact leaf catalog,
including all existing SIP beta entries. Policy tests assert the exact selected
inventory for a full release.

## Google Cloud architecture

### Recommended machine classes

| Worker class | Default VM | Memory | Work disk | Use |
| --- | --- | ---: | ---: | --- |
| controller-light | GitHub-hosted Ubuntu | managed | managed | plan, formatting, report, small checks |
| cargo-heavy | n2-standard-8 | 32 GB | 250 GB pd-balanced | workspace and per-crate shards |
| interop | n2-standard-8 | 32 GB | 250 GB pd-ssd | PBX, proxy, browser, live-service lanes |
| performance | n2-standard-32 | 128 GB | 1,200 GB pd-ssd | canonical, load, performance, and soaks |

Google documents n2-standard-32 as 32 vCPU and 128 GB RAM:
https://docs.cloud.google.com/compute/docs/general-purpose-machines

The performance VM uses the STANDARD provisioning model, not Spot. Cargo-heavy
workers may use Spot after retry behavior is proven because an interruption is
classified as INFRA_ERROR. Interop and live-service jobs use STANDARD by
default.

Machine classes and disks are policy values recorded in every attestation.
Changing them invalidates environment-sensitive evidence.

### Dedicated Google Cloud project

Use a dedicated project for release testing. It contains only:

- a private release-runner subnet;
- Cloud NAT for outbound access;
- no inbound internet firewall rule;
- one immutable Ubuntu 24.04 runner image family;
- narrowly scoped service accounts;
- a Workload Identity Federation pool/provider for GitHub;
- an evidence bucket;
- a compiler-cache bucket;
- Cloud Logging;
- budget alerts;
- quotas; and
- an expired-resource janitor.

Project, region, zone, network, and bucket names are configured inputs. The
initial recommended region is us-central1 because N2 capacity is broadly
available, but qualification is not enabled until quota and actual availability
are verified.

### Keyless GitHub-to-Google authentication

GitHub Actions authenticates to Google Cloud through Workload Identity
Federation using GitHub OIDC. No downloaded service-account JSON key is stored
in GitHub.

The provider condition must restrict:

- the exact GitHub repository;
- protected main or release branch refs;
- the exact release workflow identity; and
- the expected repository owner.

The controller impersonates a provisioner service account with only the
permissions required to create, inspect, stop, and delete release-runner VMs
and disks.

Google documents GitHub deployment-pipeline federation here:
https://docs.cloud.google.com/iam/docs/workload-identity-federation-with-deployment-pipelines

### Ephemeral GitHub runners

Every Google Cloud worker:

1. boots from a pinned immutable image;
2. receives a short-lived just-in-time GitHub runner configuration;
3. registers with the ephemeral option;
4. accepts exactly one Actions job;
5. streams runner and system logs to Cloud Logging;
6. uploads gate evidence before completion;
7. deregisters automatically;
8. shuts down; and
9. is deleted with its work disk.

GitHub recommends ephemeral runners for autoscaling because one runner receives
one job:
https://docs.github.com/en/actions/reference/runners/self-hosted-runners

A GitHub App or other short-lived GitHub credential requests the just-in-time
runner configuration. A long-lived personal access token is not placed on the
VM.

### Immutable runner image

The image contains:

- Ubuntu 24.04 x64;
- a pinned GitHub runner version;
- Docker Engine and Compose;
- build-essential, Clang, CMake, pkg-config, protobuf, OpenSSL, ALSA, and Opus
  development packages;
- browser prerequisites;
- SIPp and baresip versions required by policy;
- cargo-audit, cargo-deny, cargo-public-api, and cargo-fuzz versions required
  by policy;
- packet capture and diagnostic tools;
- Google Cloud Ops Agent; and
- the bootstrap/cleanup service.

Rust toolchains remain installed from repository-pinned definitions so a
toolchain change is visible in the candidate. The image digest and tool
versions are captured by every worker.

The image is rebuilt through a separate reviewed workflow. Release jobs never
run an unreviewed mutable latest image.

### Evidence storage

Use both:

- GitHub Actions artifacts for convenient review; and
- a versioned Google Cloud Storage bucket for durable resume and aggregation.

Canonical object layout:

~~~text
gs://BUCKET/releases/0.3.5/candidates/CANDIDATE_ID/
  plan/
  gates/GATE_ID/REUSE_KEY/ATTEMPT_ID/
  aggregate/
  diagnostics/
~~~

Every uploaded object has a SHA-256 manifest. GitHub artifact attestations bind
the gate bundle to its workflow, repository, commit, and job. GCS object
generation and checksum are recorded in the gate attestation.

The active-release bucket uses uniform bucket-level access, object versioning,
retention, and lifecycle rules. A permanent retention lock is not enabled until
the owner reviews its irreversible consequences.

### Cleanup guarantees

Each instance and disk carries:

- release version;
- candidate ID;
- GitHub run and job IDs;
- worker class;
- created-at time; and
- expires-at time.

Cleanup runs in three layers:

1. worker shutdown after evidence upload;
2. controller finally-job deletion; and
3. a scheduled Google Cloud janitor that deletes expired labeled VMs and disks.

The collector fails if an assigned worker has no deletion receipt. Budget
alerts and a maximum instance quota limit accidental fan-out.

## Workflow architecture

### Root files

The implementation adds:

~~~text
.github/workflows/release-gates.yml
.github/workflows/release-gate-worker.yml
.github/workflows/release-publish.yml
config/release-gates.yaml
scripts/release_gate.py
scripts/test_release_gate.py
infra/release-runners/
docs/RELEASE_GATE_CATALOG.md
docs/REMOTE_RELEASE_GATE_AUTOMATION_PLAN.md
~~~

The root policy composes component catalogs. The existing rvoip-sip beta policy
continues to define SIP leaf gates until its entries are migrated without
coverage loss.

### Planner

The planner:

1. resolves the exact target commit;
2. verifies a clean tracked tree;
3. validates the 44-crate graph;
4. captures toolchain and release configuration;
5. selects all required release gates;
6. loads prior verified attestations;
7. computes input and dependency changes;
8. marks gates fresh-required, reusable, stale, missing, expired, or blocked;
9. builds an acyclic gate DAG;
10. balances runnable gates into duration-aware shards; and
11. writes an immutable candidate plan.

The plan is uploaded before any worker starts.

### Parallel core workspace phase

All 44 crates are always represented in a fresh core phase after a candidate
code change.

The scheduler uses dependency-aware shards rather than launching 44 completely
independent compilations. This preserves per-crate results while sharing build
work.

Initial target:

- six cargo-heavy shards;
- one standard runner for format/policy/tooling;
- one standard runner for documentation;
- one feature-matrix shard;
- one live-service shard group; and
- fail-fast disabled everywhere.

Each shard writes a separate result bundle for every gate it executes. A failed
crate gate does not prevent other gates in the shard from recording results
unless they truly depend on the failed build artifact.

Compilation uses:

- CARGO_INCREMENTAL=0;
- reduced test/debug information where policy permits;
- a shared content-addressed compiler cache;
- an isolated CARGO_TARGET_DIR per shard;
- a read-only dependency cache after prefetch; and
- locked/offline test execution wherever external network access is not the
  subject of the test.

### Specialty phases

Specialty gates run as separate jobs or small lifecycle groups:

- RTP/SRTP known-answer and external interoperability;
- SIP provider matrices;
- browser/Chromium;
- UCTP and transport interoperability;
- MOQT network behavior;
- identity live services;
- vCon PostgreSQL;
- TURN/public-NAT when selected;
- performance; and
- soaks.

Each lifecycle group has explicit setup, readiness, test, teardown, and
leftover sub-results.

### Performance phase

The performance lane has a repository-wide concurrency lock. Only one
n2-standard-32 performance VM is active at a time.

Resumable performance units:

1. runner and baseline preflight;
2. canonical 2K pass 1;
3. canonical 2K pass 2;
4. canonical 2K pass 3;
5. throughput and call-setup profiles;
6. media, transport, and resiliency profiles;
7. high-density burst;
8. monolithic one-hour soak;
9. split one-hour soak;
10. regression audit and reconciliation.

The three canonical passes and two soaks run sequentially on exclusive
performance VMs of the same policy-defined class. Running them concurrently on
one VM would corrupt their measurements.

Each unit uploads evidence before the next is scheduled. A VM loss therefore
reruns only the current unit.

### Collector

The collector:

- verifies every gate independently;
- recalculates source/input and definition digests;
- verifies GitHub provenance and GCS checksums;
- checks freshness and runner contracts;
- validates dependency attestations;
- rejects duplicates and conflicts;
- selects exactly one VALID_PASS per selected leaf gate;
- proves fresh 44-crate core/package coverage for the target commit;
- proves any specialty reuse against the target inputs;
- produces human and machine reports; and
- writes the aggregate release attestation.

The aggregate is not allowed to turn FAIL, MISSING, STALE, EXPIRED,
INFRA_ERROR, CANCELLED, or SKIP into PASS.

## Candidate and gate identities

### Candidate ID

The candidate ID binds:

- release version;
- target commit and Git tree;
- root gate policy digest;
- selected component policy digests;
- effective release configuration;
- toolchain contract; and
- planner schema version.

### Gate definition digest

The gate definition digest binds:

- permanent number and ID;
- title and description;
- exact command template;
- environment allowlist;
- runner class;
- source/input rules;
- dependency rules;
- timeout;
- required evidence;
- validators;
- thresholds; and
- freshness/reuse policy.

### Gate input digest

The input digest binds:

- relevant source and test files;
- Cargo dependency closure;
- Cargo manifests and lockfile semantics;
- selected features and target;
- build scripts and generated inputs;
- fixtures and schemas;
- peer/container/browser identities;
- topology and runtime configuration;
- threshold and baseline; and
- architecture documents when the gate makes an architectural claim.

### Reuse proof

A reused specialty gate records:

- original candidate and commit;
- target candidate and commit;
- original and target input manifests;
- identical gate definition digest;
- identical gate input digest;
- matching environment contract;
- freshness calculation;
- dependency proof; and
- reason reuse is allowed.

The target aggregate verifies the proof itself.

## Change impact rules

| Change | Fresh core/package | Specialty impact |
| --- | --- | --- |
| Any Rust source change | all 44 core/package gates | invalidate declared crate/dependency closure |
| Version-only coordinated prepare | all 44 core/package plus version surface | reuse specialties only after allowlisted-delta proof |
| Root Cargo.lock dependency change | all 44 core/package | invalidate every specialty using changed dependency |
| Workspace feature change | all 44 core/package | invalidate gates using affected feature |
| RTP/SRTP change | all 44 core/package | RTP, media, SIP media, WebRTC media, interop, relevant performance |
| SIP-only change | all 44 core/package | SIP unit/call-flow/provider and relevant performance |
| WebRTC/RTC change | all 44 core/package | WebRTC, Chromium, TURN/NAT, Amazon Connect, relevant performance |
| UCTP/transport change | all 44 core/package | UCTP, QUIC, WebTransport, WebSocket, network integration |
| MOQT change | all 44 core/package | MOQT network/lease and BridgeFu-facing compatibility |
| Identity/extension change | all 44 core/package | relevant protocol and live-service matrices |
| Test harness change | all 44 tooling/core as selected | invalidate gates driven by changed harness |
| Peer/browser image change | core as policy requires | invalidate that peer/browser gate |
| Performance recipe/threshold change | core tooling | invalidate corresponding performance and reports |
| Release script or documentation only | fresh tooling/package validation | no runtime specialty invalidation unless behavior/input changes |
| Toolchain/runner image change | all 44 core/package | invalidate environment-sensitive gates and baseline if necessary |
| Unknown/unmapped path | fail planning | no reuse until mapped or conservatively invalidated |

The impact engine is intentionally conservative. It optimizes expensive
evidence but does not attempt to avoid the fresh all-workspace core pass.

## Per-gate result contract

Each gate produces:

~~~text
attestation.json
summary.md
command.log.zst
results.json
artifacts/manifest.json
metrics.json
junit.xml
~~~

The attestation records:

- gate and candidate identities;
- execution commit and source tree;
- input manifest and digest;
- exact sanitized command;
- effective environment;
- crate and dependency scope;
- prerequisite attestation digests;
- Rust/Cargo/target/OS/kernel identity;
- GitHub and GCP run/worker identity;
- VM machine type, CPU, memory, disk, and image;
- container, peer, and browser digests;
- timestamps and duration;
- attempt;
- exit code;
- status and failure classification;
- evidence paths and SHA-256 hashes;
- GCS object generation/checksum; and
- GitHub artifact provenance.

Execution statuses:

- PASS;
- TEST_FAIL;
- INFRA_ERROR;
- CANCELLED; and
- BLOCKED_DEPENDENCY.

Collector states:

- VALID_PASS;
- STALE_INPUT;
- STALE_ENVIRONMENT;
- EXPIRED;
- MISSING;
- CONFLICT;
- RUNNING; and
- NOT_SELECTED.

Only VALID_PASS satisfies a release gate.

## Failure and resume behavior

### Product failure

Assertions, compile failures, threshold misses, cleanup leaks, malformed
evidence, protocol mismatches, or packaging failures are TEST_FAIL.

The workflow:

- retains the attempt;
- continues unrelated work;
- blocks only true dependents;
- does not retry automatically; and
- after a fix, schedules the fresh 44-crate core phase plus the invalidated
  specialty closure.

### Infrastructure failure

VM provisioning, Spot eviction, GitHub runner loss, DNS failure during
prefetch, image-registry outage, GCS outage, disk failure, and service startup
failure before product traffic are INFRA_ERROR.

The workflow:

- retains diagnostics;
- retries up to two times on a clean VM;
- does not invalidate unrelated PASS evidence;
- never converts the failure to SKIP; and
- requires review after the retry limit.

### Cancellation

Completed bundles already stored in GCS remain valid. Resume loads them and
schedules only incomplete work. An unchanged source never triggers a
force-all rerun.

## Remote performance baseline

The current local performance baseline cannot silently become the absolute
baseline for n2-standard-32.

One-time migration:

1. Identify the approved historical baseline commit and workload definitions.
2. Build a pinned performance runner image.
3. Run at least five calibration executions on n2-standard-32.
4. Record CPU platform, kernel, image, disk, network, compiler, and workload.
5. Review variance and reject samples only for recorded infrastructure
   failures.
6. Create a runner-class-specific baseline manifest.
7. Review thresholds against BENCHMARKING.md and
   crates/sip/rvoip-sip/docs/SIGNALING_PERFORMANCE_ARCHITECTURE.md.
8. Commit the baseline/policy change separately.
9. Run the 0.3.5 candidate against that approved baseline.

The hardware migration must not relax correctness, ASR, audio-delivery,
cleanup, RSS-slope, or protocol requirements.

The following architectural properties remain protected:

- sharded exact-key lookup;
- manager-owned deadline queues;
- bounded due and ingress batches;
- generation-qualified stale-work rejection;
- compact retained lifecycle representations;
- no per-terminal-call sleeper-task architecture;
- bounded resource cleanup;
- full application audio delivery where required;
- Tokio-only runtime behavior; and
- no Smol or alternate executor/reactor dependency.

## Standard output

The collector produces:

~~~text
qualification.json
crate-status.json
crate-status.md
gate-status.json
gate-status.csv
gate-status.md
failures.md
reuse-report.md
invalidation-report.md
worker-cost-report.md
artifact-index.json
release-evidence.tar.zst
~~~

The GitHub summary includes:

- 44/44 crate core status;
- 44/44 package status;
- selected/valid/stale/missing/failed gate counts;
- gate number and description;
- why a gate ran or was reused;
- invalidating files and dependency paths;
- worker type and duration;
- retry/attempt information;
- metrics and thresholds;
- evidence and log links;
- active Google Cloud resources;
- estimated worker cost; and
- the exact resume command.

Markdown is for review. Canonical JSON, hashes, and provenance are
authoritative.

## Security boundaries

- Release infrastructure workflows run only from protected main/release refs
  or explicit maintainer dispatch.
- No pull_request_target workflow provisions Google Cloud resources.
- Fork pull requests never run on self-hosted Google Cloud workers.
- Workload Identity Federation replaces service-account keys.
- GitHub OIDC conditions restrict repository, owner, ref, and workflow.
- Provisioner and worker service accounts use least privilege.
- Qualification workers never receive crates.io credentials.
- Publication uses a separate protected GitHub environment.
- Runner VMs have no inbound internet rule.
- Every runner is ephemeral and processes one job.
- Worker logs leave the VM before deletion.
- Third-party Actions are pinned by commit SHA.
- Container and browser images are pinned by digest/version.
- GCS buckets use uniform access and lifecycle controls.
- Cloud Audit Logs, budget alerts, quotas, and deletion receipts are required.

## Workflows and operator commands

### Start the first fresh remote qualification

After infrastructure and workflow changes merge:

~~~sh
gh workflow run release-gates.yml \
  --ref main \
  -f release_version=0.3.5 \
  -f target_ref=main \
  -f mode=fresh \
  -f specialty_gates=true \
  -f performance_gates=true
~~~

The first 0.3.5 run starts fresh. Deleted local artifacts are not imported.

### Inspect

~~~sh
gh run list --workflow release-gates.yml
gh run view RUN_ID
gh run download RUN_ID -n rvoip-release-qualification
~~~

Local review of a downloaded report:

~~~sh
python3 scripts/release_gate.py status \
  --qualification qualification.json
~~~

### Resume without source change

~~~sh
gh workflow run release-gates.yml \
  --ref main \
  -f release_version=0.3.5 \
  -f target_ref=main \
  -f mode=resume \
  -f previous_candidate_id=CANDIDATE_ID
~~~

Only missing, expired, cancelled, or infrastructure-failed gates execute.

### Continue after a code fix

Push the reviewed fix. The workflow creates a new candidate:

- all 44 core/package gates run fresh;
- unaffected specialty gates are verified and reused;
- affected specialty gates run remotely; and
- the collector creates a new aggregate.

### Final release verification

After all selected gates are VALID_PASS:

~~~sh
scripts/release.sh audit
scripts/release.sh prepare --version 0.3.5
scripts/release.sh verify \
  --version 0.3.5 \
  --beta-report-root /path/to/verified/release-report
~~~

Preparation changes the candidate. The prepared commit runs the mandatory
fresh workspace/package/version-surface gates. Specialty reuse is allowed only
after the version-only delta verifier accepts the change.

Publication remains a separate manually approved workflow.

## Implementation sequence

### PR 1: Root release gate policy and catalog

- Add config/release-gates.yaml.
- Inventory all 44 crates.
- Add stable gate and crate numbers.
- Compose the existing SIP beta catalog.
- Add dependency, runner, evidence, timeout, freshness, and reuse fields.
- Generate docs/RELEASE_GATE_CATALOG.md.
- Add duplicate, cycle, unmapped-input, inventory, and selection tests.

### PR 2: Gate planner, runner, attestation, and collector

- Add scripts/release_gate.py.
- Implement plan, run, verify-gate, status, resume, and collect.
- Implement deterministic input manifests and dependency closures.
- Implement fresh-core and selective-specialty policies.
- Write atomic bundles.
- Add tamper and mismatch tests.
- Integrate aggregate verification with scripts/release.py without enabling
  publication.

### PR 3: Google Cloud infrastructure as code

- Add infra/release-runners.
- Create the dedicated project inputs, WIF, service accounts, VPC, NAT,
  buckets, logs, quotas, budgets, and janitor.
- Build the pinned Ubuntu runner image.
- Add ephemeral just-in-time runner provisioning and deletion receipts.
- Perform plan-only review before any apply.
- Apply only after separate owner authorization.

### PR 4: Fresh 44-crate remote core

- Add dependency-balanced cargo-heavy shards.
- Add live-service containers for extension tests.
- Add feature matrices.
- Add strict Clippy, MSRV, stable, Tokio-only, and no-Smol gates.
- Add per-crate result bundles.
- Prove 44/44 fresh coverage after a test commit.
- Prove cancellation/resume.

### PR 5: Packaging and registry gates

- Run release audit and prepare simulation.
- Produce all 44 package manifests and archives.
- Validate internal version requirements and topological order.
- Verify crates.io-only dependency resolution.
- Add partial-publication resume simulation and tag prohibition.

### PR 6: Remote specialty gates

- Containerize/rework local-only PBX assumptions.
- Run Asterisk, FreeSWITCH, SIPp, strict-UA, and proxy lanes remotely.
- Add browser, WebRTC, UCTP, MOQT, external SRTP, provider, and live-service
  evidence.
- Preserve every existing scenario and cleanup check.
- Prove independent gate reuse and invalidation.

### PR 7: n2-standard-32 performance lane

- Provision the exclusive performance runner class.
- Create and approve the remote baseline.
- Run canonical 2K, load, burst, resiliency, teardown, leak, and soak units.
- Verify architecture and Tokio-only requirements.
- Prove each unit resumes independently.

### PR 8: Protected release publication

- Add release-publish.yml.
- Require a verified aggregate and protected approval.
- Use crates.io credentials only in the publication environment.
- Record all 44 package checksums.
- Resume partial publication at the same version.
- Tag and create the GitHub release only after all 44 are visible.

### PR 9: rvoip 0.3.5 qualification

- Merge the infrastructure and workflow changes.
- Start one fresh full remote qualification.
- Fix discovered defects through reviewed commits.
- Rerun fresh core plus only the affected specialty closure.
- Prepare the final 0.3.5 commit.
- Run final fresh workspace/package/version-surface gates.
- Collect the complete release report.
- Proceed through audit, verify, dry run, publication, visibility, tag, and
  GitHub release.

## Required release-gate framework tests

The framework must test:

- exact 44-crate inventory;
- deterministic candidate IDs;
- deterministic gate definitions and input digests;
- dependency-graph closure;
- stable numbering;
- selection and condition handling;
- DAG cycle rejection;
- unmapped-file rejection;
- atomic evidence upload;
- independent gate verification;
- altered log/artifact rejection;
- forged PASS rejection;
- wrong commit/input/runner rejection;
- version-only delta acceptance and rejection cases;
- fresh 44-crate core after any code change;
- selective specialty reuse;
- security freshness expiration;
- duplicate/conflicting result rejection;
- interruption after partial completion;
- unchanged resume executing no valid gate;
- one-crate change invalidation;
- root dependency change invalidation;
- DNS/prefetch infrastructure classification;
- Spot eviction classification;
- product assertion failure classification;
- retry limits;
- VM deletion receipts and janitor cleanup;
- performance exclusivity;
- interop teardown;
- aggregate completeness;
- final prepared-commit verification;
- 44 package archives and checksums;
- registry-only resolution;
- partial-publication resume; and
- tag refusal before complete publication.

## Acceptance criteria

The migration is complete only when:

1. The full release qualification runs with no developer computer online.
2. The target candidate has fresh core/package evidence for all 44 crates.
3. Every selected specialty gate is freshly run or has a verified exact-input
   reuse proof.
4. Every selected leaf gate independently verifies as VALID_PASS.
5. Cancelling a workflow preserves all uploaded work.
6. Unchanged resume runs no already-valid gate.
7. A code fix reruns the complete parallel core phase and only affected
   specialty gates.
8. A version-only prepare does not rerun unrelated one-hour soaks after the
   allowlisted-delta and version-surface gates pass.
9. Unknown changes fail closed.
10. DNS, VM, or registry infrastructure problems retry only affected work.
11. No performance workload shares a VM with unrelated work.
12. Signaling performance architecture and Tokio-only policies remain intact.
13. All VMs and disks have deletion receipts.
14. The collector rejects missing, stale, expired, conflicting, or modified
    evidence.
15. scripts/release.sh verify accepts the complete aggregate and rejects an
    incomplete aggregate.
16. Qualification jobs cannot access crates.io credentials.
17. Partial crates.io publication cannot create a tag or GitHub release.

## Expected time

Initial targets after caches and images are established:

| Phase | Target wall time |
| --- | ---: |
| fresh 44-crate core and feature matrix | 45-90 minutes |
| package/archive/registry matrix | 30-60 minutes |
| external/browser/live-service gates | 60-120 minutes in parallel |
| performance excluding soaks | 60-120 minutes |
| two exclusive one-hour soaks | approximately 2 hours |
| complete fresh release | 4-6 hours |
| unchanged resume | under 10 minutes |
| code fix with unaffected specialty evidence | 60-120 minutes |

The first remote baseline migration may take longer. The steady-state process
must not return to an all-day local monolith.

## Cost controls

- Use standard GitHub-hosted runners for lightweight public-repository work.
- Provision Google Cloud workers only when a gate is queued.
- Use n2-standard-8 for heavy functional shards.
- Use exactly one n2-standard-32 performance worker at a time.
- Use Spot only for retry-safe functional work.
- Apply per-job timeouts.
- Limit project vCPU and instance quotas.
- Attach expires-at labels to every resource.
- Run a scheduled orphan janitor.
- Enable Google Cloud budgets and alerts before full qualification.
- Report VM minutes, disk hours, network usage, and estimated cost by gate.
- Never schedule force-all automatically.
- Never rerun a valid long specialty gate without an input/environment
  invalidation or explicit owner request.

## Owner authorization gates

Separate approval is required before:

- creating or modifying the Google Cloud project;
- enabling billable APIs;
- creating WIF, service accounts, VPC, NAT, buckets, images, or runners;
- applying infrastructure;
- establishing the remote performance baseline;
- changing performance thresholds;
- running paid load/soak workers;
- enabling the protected publication workflow;
- publishing to crates.io;
- tagging; or
- creating the GitHub release.

Writing and reviewing code, policy, documentation, Terraform plans, and
workflow definitions does not itself authorize a cloud apply or paid run.

## Final recommendation

Keep the repository at its current GitHub location. Use GitHub Actions to plan,
dispatch, display, attest, and collect the release. Use ephemeral Google Cloud
runners for the heavy work:

- n2-standard-8 workers for parallel workspace, service, and interop shards;
- one exclusive n2-standard-32 worker with 128 GB RAM and a 1,200 GB SSD for
  performance and soaks; and
- durable GCS evidence so interruptions never erase completed gates.

For every changed candidate, run all 44 crate core/package gates freshly and in
parallel. Reuse only expensive specialty evidence that independently proves
its exact inputs and environment are unchanged. Run the first 0.3.5 remote
qualification completely fresh, then use the resume and impact model for any
bugs found during qualification.

This is the safest practical way to remove the release monolith while retaining
a real coordinated full-workspace release.
