# rvoip-sip Production Readiness Gap Plan

Date: 2026-05-25

## Executive Verdict

`rvoip-sip` is not ready to be marketed as a broadly production-ready SIP
client and server application framework yet.

It is credible as an alpha or controlled-adopter SIP application layer for
specific, pinned scenarios: local examples, simple endpoint applications,
scripted clients, basic reactive servers, and selected Asterisk-style PBX
flows. The crate has a much stronger public API story than the lower-level
crates, and the `Endpoint`, `StreamPeer`, `CallbackPeer`,
`UnifiedCoordinator`, and `SessionHandle` surfaces are a reasonable shape for
developers.

The release is not yet ready for wide adoption because the project does not
have a complete published RFC compliance matrix, a release-gated SIP
compliance suite, published Asterisk and FreeSWITCH compatibility results,
carrier/proxy/SBC validation, or a performance report that shows stable server
behavior at expected production call rates. Several lower layers also have
documentation that overstates implemented media/security capability relative
to the code.

## What Wide Adoption Requires

For developers to choose `rvoip-sip` over mature SIP stacks, the release needs
to look less like a promising framework and more like a verified platform.

Release requirements:

- A truthful support matrix for every advertised API surface, transport, SIP
  method, RFC extension, media feature, and security mode.
- A runnable compliance test suite that proves the matrix in CI and can be
  reproduced by downstream users.
- Published interop results with Asterisk, FreeSWITCH, SIPp, PJSIP or baresip,
  and proxy/SBC-style deployments with Kamailio or OpenSIPS plus RTPengine
  where relevant.
- A benchmark report with repeatable methodology, throughput, latency,
  memory, soak, overload, and failure-recovery numbers.
- Stable, documented public APIs with examples that compile and run against
  the released crate set.
- Clear crate packaging: feature flags, MSRV, semver policy, docs.rs output,
  changelog, migration guide, and no stale crate names in user-facing docs.
- Security posture documentation: TLS validation, mTLS, SIP digest behavior,
  SRTP modes, trace redaction, keying limits, certificate handling, fuzzing,
  and disclosure process.
- Operational guidance: metrics, tracing, log redaction, backpressure,
  graceful shutdown, config examples, deployment topologies, and capacity
  planning.

## External Baseline

The practical release bar is defined by the SIP and media RFCs developers will
expect, plus the behavior of common SIP systems.

Core SIP and extension baseline:

- RFC 3261: SIP messages, transactions, dialogs, registration, OPTIONS,
  proxy behavior, transport rules, and request/response processing.
- RFC 3263: SIP server location through NAPTR, SRV, A, and AAAA resolution.
- RFC 3262: reliable provisional responses and PRACK.
- RFC 4028: session timers.
- RFC 3515: REFER and transfer behavior.
- RFC 6086: INFO method and INFO package framework.
- RFC 5626: client-initiated connections and outbound registration behavior.
- RFC 7118: SIP over WebSocket.
- RFC 3581: symmetric response routing with `rport`.
- RFC 3325: asserted identity behavior where trusted-network support is
  advertised.
- RFC 4475: SIP torture messages for parser and message-handling regression
  coverage.

Media, NAT, and security baseline:

- RFC 8866: SDP media descriptions.
- RFC 3264: SDP offer/answer behavior.
- RFC 3550: RTP and RTCP behavior.
- RFC 3711: SRTP.
- RFC 5764: DTLS-SRTP where WebRTC or modern browser interop is claimed.
- RFC 8445: ICE where NAT traversal or WebRTC edge support is claimed.
- RFC 8489: STUN.
- RFC 8656: TURN.

Interop baseline:

- Asterisk `res_pjsip` for PBX registration, calls, authentication, TLS, and
  SRTP.
- FreeSWITCH Sofia for PBX and B2BUA behavior.
- SIPp for deterministic protocol, load, negative, and regression scenarios.
- PJSIP or baresip for user-agent behavior.
- Kamailio or OpenSIPS for proxy, registrar, WebSocket, outbound, and
  high-throughput SIP routing scenarios.

## Release Quality Gates

### Gate 1: Claims and Documentation Accuracy

Release blocker until fixed:

- Root and crate docs must stop making broad "production-ready" claims unless
  the corresponding gate has passed.
- Stale crate names and deleted crates must be removed from use-case docs.
- Missing doc links in `RELEASE_HARDENING_PLAN.md` must either be restored or
  replaced.
- Lower-layer READMEs must distinguish implemented, partial, parser-only,
  experimental, and planned features.

Required artifacts:

- `docs/COMPATIBILITY_MATRIX.md`: generated or manually maintained matrix.
- `docs/TOPOLOGY_PROFILES.md`: validated deployment profiles.
- `docs/INTEROP_CI_PLAN.md`: exact lab and CI plan.
- `docs/RELEASE_NOTES_NEXT.md`: release notes with tested claims only.
- A docs audit checklist run before every release.

### Gate 2: RFC Compliance Inventory

Release blocker until started and enforced:

- Every advertised RFC must have an owner, support level, test evidence, and
  known gaps.
- Every supported method must list parser support, transaction support, dialog
  support, public API exposure, and interop coverage.
- Every advertised header must list typed parse/serialize support, validation
  rules, compact form support where applicable, and examples.
- RFC compliance status must use a small fixed vocabulary:
  `Supported`, `Partial`, `Parser only`, `Interop tested`, `Not supported`,
  and `Not audited`.

Minimum matrix rows:

- SIP core: INVITE, ACK, BYE, CANCEL, REGISTER, OPTIONS.
- Dialog-changing and mid-call methods: re-INVITE, UPDATE, PRACK, INFO, REFER,
  NOTIFY, SUBSCRIBE.
- Messaging and presence: MESSAGE, PUBLISH, SUBSCRIBE/NOTIFY event packages.
- Transport: UDP, TCP, TLS, WS, WSS, DNS SRV/NAPTR, IPv4, IPv6.
- NAT and routing: `rport`, Via handling, Record-Route/Route, outbound,
  keepalives, contact rewriting assumptions.
- Auth and identity: digest, proxy auth, P-Asserted-Identity, STIR/SHAKEN.
- Media: SDP, RTP, RTCP, DTMF, hold/resume, SRTP, DTLS-SRTP, ICE, STUN, TURN.

### Gate 3: Compliance and Regression Suite

Release blocker for broad adoption:

- Run RFC 4475-style torture parsing in CI with a documented list of
  intentional exclusions.
- Run generated message validation for outbound requests and responses.
- Add SIPp scenarios for positive and negative protocol behavior.
- Add captured-wire regression tests for Asterisk, FreeSWITCH, and at least
  one strict client stack.
- Add fuzzing targets for parser, URI parser, header parser, SDP parser, and
  transaction input.
- Convert ignored skeleton tests into either active tests or tracked release
  backlog items.

Evidence expected in the release:

- CI job names and commands.
- Test counts by crate and feature flag.
- Known skipped cases with owner and reason.
- Reproducible fixtures under version control.

### Gate 4: Interop Lab

Release blocker for "production SIP client/server" language:

- Asterisk matrix: UDP, TCP, TLS, registration, digest auth, outbound call,
  inbound call, CANCEL, BYE, hold/resume, blind transfer, attended transfer
  primitives, DTMF, OPTIONS, PRACK, session timers, SDES-SRTP, and failure
  cases.
- FreeSWITCH matrix with the same baseline.
- SIPp matrix: UAC, UAS, proxy/B2BUA, retransmissions, malformed requests,
  timer behavior, overload, and recovery.
- Proxy matrix: Kamailio or OpenSIPS registration and routing, Record-Route,
  Route, DNS SRV, failover, WebSocket where claimed, and RTPengine when media
  anchoring is part of the scenario.
- Client matrix: PJSIP or baresip calls into `rvoip-sip` and calls from
  `rvoip-sip` into those clients.

The current PBX examples are a good start, but examples are not enough. The
release needs CI or nightly lab jobs, a published compatibility table, packet
captures for failures, and a policy for updating the matrix as peer versions
change.

### Gate 5: Performance and Reliability Report

Release blocker for server adoption:

- Define supported call-rate targets by topology and feature set.
- Publish methodology: hardware, OS, Rust version, peer versions, SIPp command
  lines, feature flags, media mode, TLS/SRTP mode, and config knobs.
- Publish setup CPS, teardown CPS, active sessions, registration throughput,
  PDD, p50/p95/p99 latencies, memory per call, CPU per call, queue depths, and
  failure rate.
- Add soak tests: at least 24 hours for a release candidate, including steady
  call churn and registration refresh.
- Add overload behavior: backpressure, graceful rejection, retry-after policy,
  and recovery after peer or network failure.

Known local evidence:

- `docs/archived/RVOIP_VS_ASTERISK.md` shows Asterisk succeeding at 30, 100, and 300
  CPS in the tested SIPp scenario, while `rvoip-sip` succeeded at 30 CPS,
  partially failed at 100 CPS, and failed at 300 CPS in that run.
- `docs/BENCHMARKING.md` defines a useful benchmark harness, but publishable
  release numbers and pass/fail thresholds are still needed.

### Gate 6: API, Packaging, and Developer Experience

Release blocker for wide adoption:

- Public API surfaces must have stable examples and clear use-case boundaries:
  `Endpoint`, `StreamPeer`, `CallbackPeer`, `UnifiedCoordinator`, and
  `SessionHandle`.
- Rustdoc examples must compile in CI.
- Examples must be grouped by task: softphone/client, IVR/server, PBX
  interop, B2BUA, proxy/gateway, registrar, media, and diagnostics.
- Feature flags must be documented and tested in meaningful combinations.
- Docs must include configuration recipes for common deployments.
- Crate versions, package names, README badges, docs.rs metadata, and changelog
  must be consistent across the workspace.
- Warnings and clippy lints should become release gates for the public crates
  or have an explicit allowlist with reasons.

### Gate 7: Security

Release blocker for real deployments:

- TLS client validation, server certs, mTLS, custom roots, SNI, and insecure
  development modes must be documented and tested.
- SRTP support must state exactly which keying modes work.
- DTLS-SRTP must either be completed and tested or removed from production
  claims.
- Trace redaction must be tested for Authorization, Proxy-Authorization,
  cookies, tokens, SDP secrets, and identity headers.
- SIP digest auth must be tested for nonce handling, qop, stale, proxy auth,
  replay resistance, and error cases.
- STIR/SHAKEN status must distinguish library support from carrier-grade
  certification.
- Fuzzing and dependency audit jobs should run before release.

## Top-Down Assessment

### `rvoip-sip`

Strengths:

- The application-facing API story is coherent. `Endpoint`, `StreamPeer`,
  `CallbackPeer`, `UnifiedCoordinator`, and `SessionHandle` map well to the
  kinds of client, server, IVR, B2BUA, and test code developers will write.
- `Config` exposes many production-shaped knobs: bind and advertised
  addresses, automatic provisional responses, PRACK policy, session timers,
  digest credentials, P-Asserted-Identity, outbound proxy, SIP outbound
  parameters, keepalives, TLS, mTLS, SRTP, STUN address discovery, media mode,
  codecs, tracing, queue sizes, and worker sizing.
- The crate root rustdoc is substantially better than the lower-layer docs and
  explains gateway, B2BUA, and SBC authoring patterns.
- PBX examples cover meaningful flows and can target Asterisk or FreeSWITCH.
- The state-machine wiring manifest is a good release-control artifact.

Gaps:

- The crate is still best described as alpha-quality. Broad production claims
  are not supported by the available compliance, interop, and performance
  evidence.
- The current performance evidence includes a serious high-CPS gap compared
  with Asterisk in the same SIPp harness.
- Some state-machine paths are intentionally direct or deferred. PUBLISH is
  deferred, and some MESSAGE, OPTIONS, and SUBSCRIBE behavior bypasses the
  main state table.
- Attended transfer appears to be primitives rather than a fully packaged
  orchestration workflow.
- Asterisk and FreeSWITCH examples are useful, but they are not yet a
  release-gated compatibility matrix.
- `Config::offered_codecs` can advertise codecs that media-core may not
  actually provide. That can produce negotiated sessions without working
  audio and needs a build-time or runtime guard.
- Existing docs reference compatibility, topology, interop, and release-note
  files that are not present in the docs directory.

### `rvoip-sip-dialog`

Strengths:

- Handles the major SIP methods expected by the application layer: INVITE,
  BYE, CANCEL, ACK, OPTIONS, REGISTER, UPDATE, INFO, REFER, SUBSCRIBE, NOTIFY,
  PRACK, and MESSAGE.
- Has tests for PRACK behavior, RFC 3263, rport, subscriptions, registration,
  generated SIP compliance, SDP negotiation, and identity signing/verification.
- Provides the central transaction/dialog behavior needed by the top-level
  crate.

Gaps:

- README and architecture docs are stale and still describe old package names
  and planned features that are now partially implemented.
- Several code paths still have TODO placeholders in routing, event adapter,
  SDP, and error response handling.
- PUBLISH is not wired as a complete application-level flow.
- PRACK and reliable provisional response support needs broader B2BUA and
  interop coverage.
- A full RFC 3261 transaction/dialog audit is still required.

### `rvoip-sip-core`

Strengths:

- Provides typed methods, headers, parser, serializer, validation hooks, SDP
  support, and RFC 4475-style torture tests.
- Generated validation can check outbound messages for required wire-level
  properties.
- Supports extension methods and a broad SIP header surface.

Gaps:

- README claims are ahead of verified release evidence.
- RFC 4475 torture coverage has exclusions and must be published with reasons.
- Header and method support needs a public matrix that distinguishes typed
  parse/serialize from complete behavior in transaction and dialog layers.
- Parser support for an SDP or SIP extension must not be presented as full
  media/application support unless the higher layers are validated.
- Parser, URI, header, and SDP fuzzing should be release-gated.

### `rvoip-sip-transport`

Strengths:

- Provides UDP, TCP, TLS, WS, and WSS feature surfaces.
- Preserves raw bytes for parser/transport boundaries.
- Includes an RFC 3263 resolver path with DNS support, SRV/NAPTR behavior, and
  SIP WebSocket labels.
- Has targeted TLS, WebSocket, resolver, and helper tests.

Gaps:

- WSS outbound client dialing has `NotImplemented` paths.
- README TODOs are stale relative to the resolver code and need to be
  reconciled.
- Failover, load balancing, health checking, stress behavior, and transport
  recovery need release-gated tests.
- Transport-level backpressure and graceful overload semantics need to be
  documented and validated.
- IPv6, DNS failover, TLS certificate edge cases, and WebSocket interop need a
  published matrix.

### `media-core`

Strengths:

- Provides media-session control, RTP processing integration, G.711-oriented
  codec paths, audio frame APIs, RTP bridge tests, and performance-oriented
  tests.
- The crate has the right shape for anchoring media in SIP examples and B2BUA
  flows.

Gaps:

- Local planning docs identify missing recording, announcement, DTMF framework,
  conference enhancement, IVR support, and WebRTC/ICE work for production B2BUA
  use cases.
- Public docs overclaim production media features relative to the visible
  implementation state.
- Call-control APIs that expose playback or recording behavior need verified
  media backends rather than stubs or placeholders.
- Codec advertisement must be tied to actual codec availability.
- Media quality evidence is missing: jitter, loss, RTCP, DTMF, transcoding,
  latency, CPU, and long-running audio correctness.

### `rtp-core`

Strengths:

- Provides RTP/RTCP packet, session, transport, SRTP, stats, buffering,
  synchronization, STUN loopback, and security module structure.
- SRTP and SDES-style support appear relevant for the current SIP/PBX examples.

Gaps:

- DTLS implementation has explicit incomplete and unimplemented paths.
- TCP transport has placeholder `NotImplemented` behavior.
- SRTP transport integration docs still list unfinished tasks.
- ZRTP and MIKEY documentation conflicts with TODOs and implementation
  comments.
- Some feedback paths panic because generators moved to media-core.
- WebRTC-grade claims require DTLS-SRTP, ICE, STUN, TURN, RTP/RTCP feedback,
  and browser interop evidence that is not present.

### Supporting Crates

`auth-core`, `rvoip-sip-registrar`, `rvoip-sip-proxy`, and
`rvoip-stir-shaken` should be included in the release audit even though they
were not the main focus of this pass.

Required follow-up:

- Digest auth matrix and negative tests.
- Registrar expiration, multi-contact, path/outbound, unregister, stale nonce,
  and scale tests.
- Proxy routing, Record-Route, strict/loose routing, failover, loop detection,
  and overload tests.
- STIR/SHAKEN signing and verification docs, test vectors, error cases, and
  certification boundary.

## Gap Plan

| Phase | Goal | Exit criteria |
|-------|------|---------------|
| P0 | Stop overclaiming and define release tiers | READMEs and use-case docs match implementation; stale doc links fixed; release tier names documented as `alpha`, `interop-preview`, `production-candidate`, and `production`. |
| P1 | Build the RFC inventory | RFC matrix covers SIP, transport, SDP, RTP, SRTP, NAT traversal, auth, identity, and supported extensions with test evidence for each row. |
| P2 | Make compliance executable | CI runs parser torture tests, generated message validation, SIPp positive/negative scenarios, fuzz smoke jobs, and cross-crate integration tests. |
| P3 | Automate interop | Asterisk and FreeSWITCH suites run in CI or nightly; Kamailio/OpenSIPS proxy profile is added; PJSIP or baresip client profile is added; packet captures are stored for failing cases. |
| P4 | Fix server performance | General full-media beta profile passes up to 2,000 CPS; higher results are published only as tuned profiles with explicit configuration, hardware, topology, and caveats. |
| P5 | Harden media and security | Codec advertising is guarded; SDES-SRTP matrix is published; DTLS-SRTP is completed or de-scoped; ICE/TURN stance is explicit; DTMF, recording, playback, and media quality tests are release-gated where claimed. |
| P6 | Stabilize API and packaging | Semver policy, MSRV, feature flags, docs.rs, changelog, migration docs, rustdoc examples, examples, and lint gates are release-ready. |

## Concrete Release Blockers

1. No complete RFC support and compliance matrix exists.
2. No published compliance suite proves RFC claims across parser, transport,
   dialog, application, media, and RTP layers.
3. General full-media beta performance evidence is not yet published through
   the 2,000 CPS gate.
4. Asterisk and FreeSWITCH interop examples are not yet release gates with a
   published compatibility matrix.
5. Transport, media, and RTP docs contain overclaims or contradictions.
6. DTLS-SRTP, ICE, TURN, WSS outbound, media recording/playback, and several
   advanced media features are incomplete or not proven.
7. Use-case docs and some READMEs reference stale crate names, planned crates,
   or missing release artifacts.
8. Workspace warning/clippy policy is too permissive for a production release
   unless explicit release exceptions are documented.

## Suggested Release Tiers

`alpha`:

- Current best label. Suitable for examples, local experimentation, controlled
  PBX integration work, and developers willing to work with the maintainers.

`interop-preview`:

- Requires Asterisk and FreeSWITCH matrices, SIPp regression suite, corrected
  docs, and published known limitations.

`production-candidate`:

- Requires RFC matrix, compliance CI, performance report, 24-hour soak,
  security audit pass, API freeze, and release notes with tested claims only.
  The default full-media performance claim is capped at 2,000 CPS; any higher
  result is a tuned profile, not a general-user promise.

`production`:

- Requires at least one production-candidate cycle without release-blocking
  regressions, compatibility matrix updates for supported peer versions, and a
  documented maintenance/security process.

## Source Baseline

- [RFC 3261: SIP](https://www.rfc-editor.org/rfc/rfc3261)
- [RFC 3263: SIP server location](https://www.rfc-editor.org/rfc/rfc3263)
- [RFC 3262: reliable provisional responses](https://www.rfc-editor.org/rfc/rfc3262)
- [RFC 4028: SIP session timers](https://www.rfc-editor.org/rfc/rfc4028)
- [RFC 3515: SIP REFER](https://www.rfc-editor.org/rfc/rfc3515)
- [RFC 6086: SIP INFO package framework](https://www.rfc-editor.org/rfc/rfc6086)
- [RFC 5626: SIP outbound](https://www.rfc-editor.org/rfc/rfc5626)
- [RFC 7118: SIP over WebSocket](https://www.rfc-editor.org/rfc/rfc7118)
- [RFC 3581: SIP symmetric response routing](https://www.rfc-editor.org/rfc/rfc3581)
- [RFC 3325: SIP asserted identity](https://www.rfc-editor.org/rfc/rfc3325)
- [RFC 4475: SIP torture test messages](https://www.rfc-editor.org/rfc/rfc4475)
- [RFC 8866: SDP](https://www.rfc-editor.org/rfc/rfc8866)
- [RFC 3264: SDP offer/answer](https://www.rfc-editor.org/rfc/rfc3264)
- [RFC 3550: RTP](https://www.rfc-editor.org/rfc/rfc3550)
- [RFC 3711: SRTP](https://www.rfc-editor.org/rfc/rfc3711)
- [RFC 5764: DTLS-SRTP](https://www.rfc-editor.org/rfc/rfc5764)
- [RFC 8445: ICE](https://www.rfc-editor.org/rfc/rfc8445)
- [RFC 8489: STUN](https://www.rfc-editor.org/rfc/rfc8489)
- [RFC 8656: TURN](https://www.rfc-editor.org/rfc/rfc8656)
- [SIPp](https://sipp.sourceforge.net/)
- [Asterisk `res_pjsip` documentation](https://docs.asterisk.org/Configuration/Channel-Drivers/SIP/Configuring-res_pjsip/)
- [FreeSWITCH Sofia SIP stack documentation](https://developer.signalwire.com/freeswitch/FreeSWITCH-Explained/Configuration/Sofia-SIP-Stack/)
- [PJSIP feature overview](https://docs.pjsip.org/en/latest/overview/features.html)
- [Kamailio feature overview](https://www.kamailio.org/w/features/)
