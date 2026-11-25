# rvoip 0.5.1 Release Notes

0.5.1 adds UDP TURN transport and per-contact Amazon Connect relay credentials,
plus an explicit shorter patch-release path. The workspace publishes all 48
public crates together; the Rust requirement remains 1.95.

## TURN and ICE

WebRTC peers can allocate UDP relays, route media through the selected relay,
and release allocations on close or drop. The regression suite covers relay
nomination, exact packets in both directions, invalid credentials, server
fallback, and cleanup. The runnable example also verifies direct ICE with
`ice_servers: []`.

Amazon Connect can obtain temporary credentials from Chime `JOIN_ACK`, with
configurable credential sources, relay-only policy, and timeouts. Empty static
ICE servers do not disable contact-specific relay provisioning. The existing
candidate retry budget is unchanged. TURN over TCP/TLS and replacement Chime
credentials during an established contact are not included.

Start with [the TURN/ICE test recipe](../TURN_ICE.md) and
[Connect relay configuration](../CONNECT_TURN.md). Local protocol tests do not
claim live-provider or arbitrary-NAT compatibility.

## Release checks

This release uses `release-patch-smoke`: regular unit, integration, regression,
example, and doc tests; explicit direct ICE and TURN relay checks; formatting,
Clippy, feature-bundle builds, and package inventory checks. Final crate
archives are built and verified once by the protected publisher. Signed evidence binds
the complete selected gate set to the qualified source commit. See the
qualification record below and the GitHub release attachments for commands,
results, and fresh/reused evidence.

## Performance evaluation

**NOT RUN for 0.5.1.** The shorter path intentionally omits benchmarks, load,
long-running soak/fuzz campaigns, and extended external interoperability jobs.
The 0.5.0 benchmark reports remain historical evidence for their recorded
commits and environments; they are not a new performance measurement of 0.5.1.
General-user 10,000 CPS full-media capability remains outside this release's
claims.

## Interoperability

The extended Asterisk, FreeSWITCH, Jambonz OSS, Kamailio, OpenSIPS, and SIPp
release matrix was not repeated by this patch profile. Regular automated
integration tests remain included. Use the full release profile when that
extended qualification is required.

For context, the [published 0.5.0 notes](https://github.com/eisenzopf/rvoip/blob/v0.5.0/docs/release/RELEASE_NOTES_NEXT.md)
describe the Jambonz OSS 0.9.15 profile: `Endpoint`, `StreamPeer`, and
`CallbackPeer` over UDP, PCMU/PCMA audio, DTMF, CANCEL, and transfer. Its
exclusions include G.729, AMR, TLS/SRTP, WebRTC, and PSTN. Those boundaries
remain historical; this patch does not claim a fresh execution or expand
them. The transfer `Referred-By` header remains optional.

## Compatibility

Existing default behavior remains available. Configure relay-only behavior
explicitly when direct media destinations are blocked, and keep credentials,
server URLs, ports, and timeouts deployment-specific. Applications upgrading
from 0.4.x must also follow the breaking-change migrations in the historical
0.5.0 changelog.

<!-- generated:changes:start -->
## Changes in this release

### Fixed

- The managed MoQ relay test accepts either valid first-subscriber cache state.
  Exact audio, retained-fetch, cleanup, and deterministic cold-fallback checks
  remain enforced. This is a test correction, with no runtime change.

### Added

- Add UDP TURN allocation and relay media transport, per-contact Amazon Connect
  credentials from Chime JOIN_ACK or the configured control endpoint, configurable
  relay-only ICE, bounded allocation/send/close operations, and allocation cleanup.
  Regular regression tests and runnable examples check relay nomination, exact
  bidirectional packets, invalid credentials, fallback, cleanup, and direct ICE
  with an empty ICE-server list. TCP/TLS TURN and credential replacement during an
  established contact remain outside this implementation. (#482)
- Patch releases can explicitly select `release-patch-smoke` to run regular unit,
  integration, regression, example, and doc tests, including direct ICE and TURN
  relay checks, without benchmark, soak/load, or extended release jobs. Package
  verification and signed exact-commit evidence remain required. Reports disclose
  the reduced scope and mark performance qualification NOT RUN; feature and
  breaking releases retain full qualification. (#484)

### Documentation

- The rvoip-sip README, release checklist, compatibility and RFC matrices,
  security posture, topology profiles, interop plan, PBX lab README, and
  `docs/CARRIERS.md` now cite the published 0.5.0 qualification (run
  45304004340), including the first protected evidence for the Asterisk and
  FreeSWITCH RTCP and session-timer cells and the RFC 7501/7502 benchmark rates.
  Its reports are archived under
  `crates/sip/rvoip-sip/docs/releases/qualification/20261124T012650Z-45304004340`. (#474)
- The 0.5.0 RFC 7501/7502 benchmark report (IETF format) is now in the
  repository, in the 0.5.0 qualification archive, and linked from the README,
  the rvoip-sip README and `RFC7502_BENCHMARK.md`. It covers the sustained
  Session Establishment Rates of rvoip-bench-uas on one c8i.4xlarge, the test
  setup, every search trace, the RFC 6076 metrics, and the R4 overload and
  recovery trial. The README adds a short "Performance at a glance (0.5.0)"
  table from the same evidence. (#481)
<!-- generated:changes:end -->

<!-- generated:qualification:start -->
## Qualification record

Candidate `fa09d7213dc41e5482a51c94cb31672f325084a2` passed `74` gates in the protected `release-patch-smoke` [workflow run](https://github.com/eisenzopf/rvoip/actions/runs/45696234357): workspace crate tests, integration and regression tests, standalone examples, doc tests, direct ICE and TURN relay regressions, and package inventory checks. Benchmarks, load/soak tests, long fuzz campaigns, and external interoperability qualification were intentionally not run. This is reduced patch qualification.

The `0.5.1` version-bump commit must pass the same profile, bound to its exact commit, before protected publication. Reused receipts require matching inputs; the release archives include the accepted-gate ledger and explicit NOT RUN performance report.
<!-- generated:qualification:end -->
