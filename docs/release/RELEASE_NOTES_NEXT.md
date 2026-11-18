# rvoip 0.5.0 Release Notes

These notes describe the coordinated 48-crate `0.5.0` release. The latest
published release is `0.4.0`. Protected publication requires qualification
of the exact source commit; the signed qualification artifact and the GitHub
release identify that commit, the workflow run, the complete gate inventory,
the measured performance and the publication result.

## Headline

`0.5.0` is a breaking minor release about carrier-scale throughput and
overload behaviour, and it moves the whole dependency set forward.

- **Media executor v2.** A new opt-in executor runs media on dedicated,
  CPU-pinned lanes sized by a host plan. Installing
  `MediaExecutorConfig::v2()` turns it on; `v1()` stays the default, so
  nothing changes until an application opts in.
- **SIP overload hardening.** TLS flows apply back-pressure instead of
  dropping responses. New calls can be shed with a stateless 503 before any
  state exists, and overload rejections carry `Retry-After`. Hangup races
  end calls cleanly.
- **An RFC 7501/7502 benchmark gates every release.** From `0.5.0`, release
  qualification measures rvoip's Session Establishment Rate and RFC 6076
  latencies on a pinned testbed.
- **Webex BYOVA and Amazon Connect A2A.** Two contact-centre adapters connect
  Webex Contact Center virtual agents and Amazon Connect external agents to
  rvoip.
- **Rust 1.95 and current dependencies.** The minimum supported Rust version
  is 1.95, and the upgrades held back in `0.4` land, including nom 8 and
  axum 0.8.

Applications upgrading from `0.4.0` should read the Breaking changes section
of the [changelog](../../CHANGELOG.md) before bumping. Each entry there ends
with a one-line migration.

## Media executor v2

`MediaExecutorConfig::v2()` turns on every switch of the new executor. The
per-call task graph, `v1()`, remains the default.

- Media runs on independent lanes, each with its own scheduler, timer and
  I/O driver, pinned to CPUs by a host plan.
- Receive loops and paced sends run inside one poller per lane. Inbound SRTP
  is authenticated, decoded and delivered inline on the socket owner, and RTP
  is sent synchronously.
- Lanes wake on a 2 ms tick and keep their deadlines in a timing wheel.
  SRTP, RTCP and receive statistics stay with the socket owner, and per-call
  hot state is packed into a few cache lines.
- Per-call sends and receives fire from a per-lane call table. The SIP
  inbound pump runs fused on the socket owner with zero-copy playout.
- Media graphs that bridge a SIP leg to an in-process AI run on that leg's
  lane, so caller audio reaches the AI session without a task hop.

The changelog gives each switch's measured CPU-per-call effect. The
benchmark DUT runs executor v2, so the release benchmark measures it under
load.

## SIP overload and call lifecycle

- **TLS back-pressure.** A slow TLS peer slows the flow instead of losing
  responses or having its connection closed.
- **Transaction layer.** Deadlocks between the transaction layer and the
  transaction user are fixed.
- **Stateless admission.** New dialog-forming INVITEs can be shed with a
  stateless `503` before any transaction or dialog exists. Overload
  rejections carry `503 Service Unavailable` with `Retry-After`.
- **Hangup.** Hangup and crossed-BYE races end calls cleanly. Ownerless
  sessions are swept, and established calls hang up from the hold, mute and
  bridged states.
- **Gateways.** Outbound peer leases tie an outbound call to a live peer
  connection. A trusted proxy's accepted TLS flow is reused for in-dialog
  requests. Inbound contexts carry the observed peer address and caller
  number.
- **Protocol coverage.** UAS capabilities are configurable and advertised in
  OPTIONS. Message bodies follow RFC 5621. A single-address-family endpoint
  can run on IPv6.
- **Hot paths.** Signaling and media hot paths copy, lock and scan less per
  call.

`Require: replaces` is now answered with `420 Bad Extension`, since rvoip
does not perform RFC 3891 replacement itself. The changelog explains how to
re-enable it for an application that does.

## RFC 7501/7502 benchmark

`0.5.0` is the first release gated on an RFC 7501/7502 SIP benchmark. The
device under test is `rvoip-bench-uas`, a terminating UAS on `CallbackPeer`
that echoes media through executor v2's inline path. It is not published.

- **What it measures.** The lane measures the Session Establishment Rate:
  the highest attempt rate rvoip sustains with zero failed attempts. It also
  records RFC 6076 latencies, over TLS and UDP, signaling-only and with
  G.711 media.
- **Minor releases.** A minor release such as this one searches each row
  for its rate and runs an overload-and-recovery trial with admission on. It
  renders an RFC 7502-style report into the gate evidence.
- **Patch releases.** A patch release confirms each row against the recorded
  rate.
- **Where it runs.** The testbed is pinned by environment id, which covers
  the instance types, image and caller build. Its design and contract are in
  [RFC7502_BENCHMARK.md](../../crates/sip/rvoip-sip/docs/RFC7502_BENCHMARK.md).

## Webex BYOVA and Amazon Connect A2A

- **Webex BYOVA.** The experimental `rvoip-webex-byova` adapter connects
  Webex Contact Center BYOVA (Bring Your Own Virtual Agent) over Cisco's
  WebSocket protocol, and over gRPC with the `grpc` feature.
  - Every connection is authenticated before it is read, and published with
    its principal.
  - Calls expose caller inputs, interruption, handoff and exact
    reply-boundary marks.
  - rvoip does not redistribute Cisco's protobuf files. The `grpc` feature
    fetches them at build time and checks them against pinned SHA-256 hashes.
- **Amazon Connect A2A.** The developer-preview `rvoip-amazon-a2a` adapter
  connects Amazon Connect external agents (A2A), with native media bridging,
  turn traces, interruption and hand-back. Enable it with the facade's
  `amazon-a2a` feature.
- **Vapi.** Vapi calls can opt into ordered, lossless media delivery. This
  replaces the removed `raw_session` bypass.

Neither contact-centre adapter has been qualified against live Cisco or
Amazon traffic yet.

## Toolchain and dependencies

The minimum supported Rust version is **1.95** (was 1.91).

The dependency set moves forward:

- nom 8 in the SIP parser;
- axum 0.8 and axum-server 0.9;
- the RustCrypto wave with rand 0.10;
- prost and tonic 0.14;
- jsonwebtoken 12;
- reqwest 0.13 on ring;
- sqlx 0.9;
- redis 1;
- web-transport 0.13;
- cpal 0.18;
- the smaller upgrades from the grouped dependabot update.

Nothing on `0.4`'s held-back list is still held back in a published crate.
Where a dependency's types are in rvoip's public API, the changelog's
Breaking changes entry names the affected items and the migration.

Three changes need attention even in code that never names those crates.

### metrics 0.24 and OpenTelemetry 0.33

rvoip emits into whatever metrics recorder and OpenTelemetry global
provider the application installs. Both are keyed to their crate's semver
line. Install a metrics 0.24 recorder (metrics-exporter-prometheus 0.18)
and an OpenTelemetry 0.33 provider (tracing-opentelemetry 0.34). An
application still on metrics 0.23 or OpenTelemetry 0.32 builds and runs,
but rvoip's metrics and spans silently disappear.

### WebRTC on `rtc` 0.21.1

`rvoip-rtc` is rebased onto upstream `rtc` 0.21.1. Two behaviours change for
every WebRTC user, including code that only uses `rvoip-webrtc`:

- **Create data channels before the offer.** SCTP is no longer started for
  a media-only session, so a data channel opens only on a session whose offer
  carried it. A channel added to an established media-only session needs a
  new offer and answer before it opens.
- **Relay-only ICE needs a TURN server.** A peer connection with
  `IceTransportPolicy::Relay` and no TURN server is rejected when it is
  built.

Code that uses `rvoip-rtc` or `rvoip-webrtc-stack` directly follows
upstream's 0.21 API. The changelog lists the migration.

### Explicit rustls crypto provider

Every rustls client and server rvoip builds now names its provider (ring)
instead of using rustls's process-level default. Applications whose
dependency graph compiles both ring and aws-lc-rs, for example through an
AWS SDK, no longer panic with "Could not automatically determine the
process-level CryptoProvider".

Three dependencies can only use the process-level default: DTLS-SRTP,
redis TLS and sqlx Postgres TLS. For those, rvoip installs ring only if no
provider is installed yet. Install your own at startup to choose.

## Interoperability

### Asterisk and FreeSWITCH

The PBX release gates check RTCP and RFC 4028 session-timer behaviour
against real Asterisk and FreeSWITCH, beyond the protocol matrices they
already ran:

- RTCP over negotiated mux;
- RTCP on a separate port;
- no RTCP toward a peer that declines mux;
- session timers, refreshed in both directions.

Each cell also records the PBX's own verdict.

### Jambonz OSS

Jambonz OSS remains a mandatory external SIP interoperability peer.

- **Roles.** It is tested as an independent SBC/B2BUA, registrar and
  RTPengine media anchor. The tests use the same `Endpoint`, `StreamPeer`
  and `CallbackPeer` APIs as Asterisk and FreeSWITCH.
- **Pinning.** The release profile pins the latest reviewed open-source
  component line, currently Jambonz OSS `0.9.14`. Source archives and containers are digest-verified, and
  qualification fails if the pins are no longer the selected upstream heads.
- **Matrix.** The mandatory UDP/plain-RTP matrix covers:
  - authenticated registration;
  - PCMU and PCMA calls with bidirectional audio;
  - provisional and final call signaling;
  - hold and resume;
  - RFC 4733 DTMF;
  - CANCEL and 487;
  - rejection;
  - REFER/NOTIFY blind transfer;
  - replacement INVITE;
  - BYE from either side;
  - resource cleanup.

  `Refer-To` remains mandatory for REFER. RFC 3892 `Referred-By` remains
  optional, and is preserved unchanged when a peer supplies it.
- **Exclusions.** These are outside the Jambonz profile: G.729, AMR,
  TLS/SRTP, the RVoIP-as-B2BUA scenario, WebRTC, PSTN, application verbs,
  recording, high availability and load. Codec and transport support
  elsewhere in rvoip is not reduced by these peer-specific exclusions.

The qualification record below names the Jambonz OSS release line this
candidate was tested against.

## Performance evaluation

The protected `0.5.0` qualification is a `release-minor` qualification:
the full remote release profile plus the minor RFC 7502 benchmark lane. It
requires:

- the canonical 2,000-CPS passes;
- the full performance and resiliency matrix;
- the high-density full-media burst;
- the monolithic and split soaks;
- the teardown and churn tests;
- regression comparison against the reviewed performance baselines;
- the RFC 7502 searches, overload trial and report.

It produces structured JSON and Markdown metrics and a SHA-256 index of every
performance artifact of the run. Results from earlier releases are
historical baselines and cannot qualify this release.

General-user 10,000 CPS full-media capability is not claimed. The supported
envelope remains bounded by the 2,000-CPS real-media evidence, the exact host
configuration, workloads and soak durations recorded for this release, and
the benchmark rates measured on the pinned testbed.

The signed qualification artifact is the authority for the measured
results:

- the host shape, source commit and workload configuration;
- ASR and error counts, and setup-latency percentiles;
- CPU and RSS behaviour, media delivery and cleanup;
- the measured Session Establishment Rates;
- the artifact hashes.

The GitHub release links that artifact and its workflow run.

<!-- generated:changes:start -->
<!-- generated:changes:end -->

## Compatibility

`0.5.0` is not source-compatible with `0.4.0` for:

- applications that match exhaustively on the enums that gained variants;
- applications that build the structs that gained fields with struct
  literals;
- applications that name the dependency types now on newer major lines
  (nom, axum, jsonwebtoken, prost and tonic, sqlx, reqwest, rand, redis,
  ordered-float, quick-xml, validator, cpal, parking_lot and web-transport);
- WebRTC code that uses `rvoip-rtc` or `rvoip-webrtc-stack` directly.

Each case has a one-line migration in the changelog's Breaking changes
section.

Rust 1.95 is required.

Runtime behaviour changes for every application in these cases:

- the metrics and OpenTelemetry coupling above;
- the two WebRTC behaviours above;
- `Require: replaces` answered with 420;
- a call whose media a peer BYE stopped first now ends normally instead of
  as a media failure.

Executor v2 changes nothing unless installed.

The `0.4.0` tag, crates and qualification evidence remain immutable release
history. Qualification reuses earlier evidence only where a gate's exact
inputs are unchanged, through the candidate's receipt chain.

<!-- generated:qualification:start -->
## Qualification record

The protected Release Qualification workflow runs from a clean `main`
commit. It records every selected gate as PASS, including:

- the full crate matrix, feature bundles and CodeQL policy;
- Asterisk, FreeSWITCH, Jambonz OSS, Kamailio, OpenSIPS, SIPp and strict-UA
  interoperability;
- the security, performance, resiliency, soak and regression gates;
- the RFC 7502 benchmark;
- the source-fence and cleanup gates.

It produces a signed aggregate bound to the exact source SHA and immutable
artifact hashes.

The protected Release Publish workflow accepts that aggregate only after
verifying that:

- its source SHA is on `main`;
- its version is exactly `0.5.0`;
- its evidence is complete and bound to the release commit through the
  receipt chain;
- the dry-run and live publication inputs resolve to the same commit.

The resulting GitHub release links the exact qualification run and the
Performance evaluation report. The protected `v0.5.0` tag identifies the
source.
<!-- generated:qualification:end -->
