# rvoip-sip

[![Crates.io](https://img.shields.io/crates/v/rvoip-sip.svg?release=0.3.11)](https://crates.io/crates/rvoip-sip/0.3.11)
[![docs.rs](https://img.shields.io/docsrs/rvoip-sip/0.3.11?label=docs)](https://docs.rs/rvoip-sip/0.3.11/rvoip_sip/)
[![Rust 1.91+](https://img.shields.io/badge/rust-1.91%2B-orange.svg)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/eisenzopf/rvoip/blob/main/LICENSE)
[![Repository](https://img.shields.io/badge/github-eisenzopf%2Frvoip-24292f.svg)](https://github.com/eisenzopf/rvoip)
[![GitHub issues](https://img.shields.io/github/issues/eisenzopf/rvoip.svg)](https://github.com/eisenzopf/rvoip/issues)

`rvoip-sip` is the application-facing SIP session layer for RVoIP. It
coordinates dialog state, registration, media setup, call control, transfer,
DTMF, hold/resume, custom SIP headers, and app-visible events so Rust
applications can behave like programmable SIP endpoints without owning SIP
transaction or RTP details directly.

The published `0.3.11` release came from a strict, exact-source qualification.
Its generated [release report](docs/BETA_RELEASE_REPORT.md), [gate
ledger](docs/BETA_GATE_REPORT.md), and [performance
report](docs/BETA_PERFORMANCE_REPORT.md) are authoritative for the tested PBX,
proxy, SIPp, strict-UA, security, performance, and soak boundaries. The
[immutable qualification history](docs/releases/qualification/README.md)
retains prior releases and the detailed 0.3.11 performance evaluation.

## At a glance

| Need | Start with |
| --- | --- |
| Make calls from a softphone or PBX account | [`Endpoint`](https://docs.rs/rvoip-sip/0.3.11/rvoip_sip/api/endpoint/struct.Endpoint.html) |
| Write a sequential client, script, or test | [`StreamPeer`](https://docs.rs/rvoip-sip/0.3.11/rvoip_sip/api/stream_peer/struct.StreamPeer.html) |
| Build a reactive server, IVR, router, or queue | [`CallbackPeer`](https://docs.rs/rvoip-sip/0.3.11/rvoip_sip/api/callback_peer/struct.CallbackPeer.html) |
| Compose multiple call legs or a B2BUA | [`UnifiedCoordinator`](https://docs.rs/rvoip-sip/0.3.11/rvoip_sip/api/unified/struct.UnifiedCoordinator.html) |
| Control an active call | [`SessionHandle`](https://docs.rs/rvoip-sip/0.3.11/rvoip_sip/api/handle/struct.SessionHandle.html) |
| Check Asterisk, FreeSWITCH, Jambonz, Kamailio, or OpenSIPS status | [Interoperability status](#interoperability-status) |
| Bridge a SIP caller to a native Vapi WebSocket agent | [`rvoip-vapi`](#extensions-and-native-vapi-websocket-agents) |

Start with `Endpoint` unless you already know you need event-stream ownership,
callback dispatch, or custom multi-leg orchestration. The higher-level surfaces
are thin wrappers over `UnifiedCoordinator`, so applications can move down a
level without switching protocol stacks.

## Install

`rvoip-sip` uses the workspace minimum supported Rust version. The current MSRV
is **Rust 1.91**.

```toml
[dependencies]
rvoip-sip = "0.3.11"
tokio = { version = "1", features = ["full"] }
```

For repository development:

```sh
git clone https://github.com/eisenzopf/rvoip.git
cd rvoip
RUSTUP_TOOLCHAIN=1.91 cargo check -p rvoip-sip --all-targets
```

## Quick start

Run a local two-endpoint call first:

```sh
cargo run -p rvoip-sip --example endpoint_local_call
```

For a registered PBX account, the `Endpoint` facade keeps the application code
focused on account setup and call control:

```rust,no_run
use std::time::Duration;

use rvoip_sip::{Endpoint, EndpointProfile, Result};

# async fn example() -> Result<()> {
let mut endpoint = Endpoint::builder()
    .name("alice")
    .account("1001")
    .password("secret")
    .registrar("sips:pbx.example.com:5061")
    .profile(EndpointProfile::AsteriskTlsSrtpRegisteredFlow)
    .build()
    .await?;

endpoint.register().await?;

let call = endpoint
    .call_and_wait("1002", Some(Duration::from_secs(30)))
    .await?;

call.send_dtmf('1').await?;
call.hangup_and_wait(Some(Duration::from_secs(5))).await?;
endpoint.shutdown().await?;
# Ok(())
# }
```

See [`examples/endpoint/03_registered_account/main.rs`](examples/endpoint/03_registered_account/main.rs)
for the env-driven PBX account runner.

## Choose an API surface

| API | Use it for | Programming model |
| --- | --- | --- |
| `Endpoint` | Softphones, PBX accounts, demos, simple IVR legs | Account/profile builder plus call helpers |
| `StreamPeer` | Clients, scripts, softphones, integration tests | Sequential calls plus event waits |
| `CallbackPeer` | Servers, IVR, routing apps, queue-style apps | Closure builder or `CallHandler` callbacks |
| `UnifiedCoordinator` | Bridges, gateways, custom peer types, B2BUAs | Explicit session IDs and orchestration methods |
| `SessionHandle` | Per-call operations from any surface | Hangup, progress waits, DTMF, hold/resume, transfer, audio |

`SessionHandle` is the per-call control object shared by the peer surfaces. It
currently exposes deterministic teardown, answered/progress waits, RFC 4733
DTMF, hold/resume, blind transfer, REFER/NOTIFY lifecycle events, SDES-SRTP
state, feature-gated DTLS-SRTP state, typed per-call events, and
decoded/encoded audio frames.

Applications composing `SipAdapter` through `rvoip-core` may request a
per-connection codec change with `Orchestrator::renegotiate_media`. The SIP
adapter translates that request into a one-shot re-INVITE offer for the exact
dialog generation and returns only after the peer's SDP answer is committed.

Carrier-facing configurations can enable `PlayoutConfig` to place inbound
audio on a local media clock. The buffer reorders RTP timestamps, emits
repeat-with-fade PLC for missing G.711 frames, tracks remote clock skew, and
drains excess depth so burst jitter does not become permanent latency.
`Config::carrier_sbc` enables the production default; generic/local configs
remain unbuffered for compatibility and deterministic packet-level labs.
The call's stable media and stream descriptor remain unchanged on rejection;
on success the existing stream updates both media pumps without changing its
identity or application channels. Codec preferences never mutate coordinator-
wide offer policy and therefore cannot bleed into concurrent calls.

## Examples

The examples are organized by developer surface in
[`examples/README.md`](examples/README.md).

| Scenario | Command |
| --- | --- |
| Local call through `Endpoint` | `cargo run -p rvoip-sip --example endpoint_local_call` |
| Local audio round trip | `cargo run -p rvoip-sip --example endpoint_audio_roundtrip` |
| Registered PBX account | `cargo run -p rvoip-sip --example endpoint_registered_account` |
| Sequential client/test API | `cargo run -p rvoip-sip --example stream_peer_basic_call` |
| Reactive auto-answer server | `cargo run -p rvoip-sip --example callback_peer_auto_answer_server` |
| Callback IVR pair | `./crates/sip/rvoip-sip/examples/callback_peer/03_builder_ivr/run.sh` |
| Unified B2BUA bridge | `./crates/sip/rvoip-sip/examples/unified/04_b2bua_bridge/run.sh` |
| Terminal softphone | `cargo run -p rvoip-sip --example sip_client` |
| Asterisk interop matrix | `./crates/sip/rvoip-sip/examples/pbx/run.sh --pbx asterisk --api all --scenario all` |
| FreeSWITCH interop matrix | `./crates/sip/rvoip-sip/examples/pbx/run.sh --pbx freeswitch --api all --scenario all` |
| Jambonz interop matrix | `./crates/sip/rvoip-sip/examples/pbx/run.sh --pbx jambonz --api all --scenario all --transport UDP` |

PBX interop setup, environment variables, and scenario coverage are documented
in [`examples/pbx/README.md`](examples/pbx/README.md). The terminal softphone
is documented in [`examples/sip_client/README.md`](examples/sip_client/README.md).

## Interoperability status

The published release recorded fresh, revision-bound PASS evidence at commit
`77a99cd38a07641294cf7dc547146b115b135dc7` for Asterisk, FreeSWITCH, Jambonz,
SIPp, baresip, Kamailio, and OpenSIPS. Kamailio and OpenSIPS each passed both
adjacency orders (peer-first and rvoip-first) over UDP, TCP, and TLS
(`interop.remote-proxies.*`) plus the registrar-proxy matrix through an
rtpengine media relay (`interop.proxy-pbx.*`). The generated
[gate ledger](docs/BETA_GATE_REPORT.md) is the exact record; publication is
blocked unless it records the complete required matrix as PASS.

| Peer/tool | Status | Executed scope |
| --- | --- | --- |
| **Asterisk** | **Release-gated; matrix passed** (`interop.asterisk-matrix`) | `Endpoint`, `StreamPeer`, and `CallbackPeer` across registration, basic call, G.729A/G.729AB, hold/resume, ring-cancel, RFC 4733 DTMF, rejection, and blind transfer over UDP and TLS |
| **FreeSWITCH** | **Release-gated; matrix passed** (`interop.freeswitch-matrix`) | The same API, scenario, codec, and UDP/TLS matrix as Asterisk |
| **Jambonz OSS 0.9.9** | **0.3.11 release gate passed** (`interop.jambonz-matrix`) | The same registered-user `Endpoint`, `StreamPeer`, and `CallbackPeer` scenario runner used for Asterisk and FreeSWITCH, across the applicable UDP SIP/SDP/RTP B2BUA matrix |
| **SIPp** | **Release-gated; standalone matrix passed** (`interop.sipp-matrix`) | 30, 100, 300, 1,000, and 2,000 CPS with 100% configured call completion |
| **baresip** | **Release-gated; strict-UA check passed** (`interop.strict-ua`) | External user-agent call against the rvoip SIP listener |
| **Kamailio** | **Release-gated; proxy matrix passed** | Proxy interoperability in both adjacency orders over UDP, TCP, and TLS (`interop.remote-proxies.kamailio.*`), plus the `Endpoint` all-scenario matrix through an rtpengine media relay with AMR passthrough required (`interop.proxy-pbx.kamailio.matrix`) |
| **OpenSIPS** | **Release-gated; proxy matrix passed** | The same adjacency-order, transport, and rtpengine AMR passthrough matrix as Kamailio (`interop.remote-proxies.opensips.*`, `interop.proxy-pbx.opensips.matrix`) |

The machine-bound [current gate ledger](docs/BETA_GATE_REPORT.md),
[compatibility matrix](docs/COMPATIBILITY_MATRIX.md), and [topology
profiles](docs/TOPOLOGY_PROFILES.md) define the exact claim; the
[0.3.2 gate record](docs/BETA_GATE_EXCEPTION.md) is historical. These results
do not imply carrier certification or untested peer-version/topology coverage.

## Capabilities

- SIP call setup and teardown with registration lifecycle support.
- INVITE, REGISTER, BYE, CANCEL, REFER, NOTIFY, INFO, PRACK, session timer,
  redirect, provisional response, and glare-retry paths covered by examples or
  regression fixtures.
- UDP and TLS SIP paths in the beta-candidate evidence set.
- RTP media sessions, bidirectional audio frames, RFC 4733 DTMF, SDES-SRTP,
  and feature-gated DTLS-SRTP negotiation state. DTLS-SRTP accepts both the
  `UDP/TLS/RTP/SAVP` and `UDP/TLS/RTP/SAVPF` profiles, and the offered
  `a=setup` role is selectable with `DtlsSetupRole` (`Actpass`, `Active`,
  `Passive`) through `Config::with_dtls_setup_role`. The exact supported and
  fail-closed boundaries are documented in
  [Crypto capability boundaries](docs/CRYPTO_CAPABILITIES.md).
- STUN-discovered RTP mapping: `Config::stun_server` runs an RFC 8489 probe
  from the call's own RTP socket (rtp-core `TransportStunClient`) so the
  public IP and port rendered in SDP match the media path. Discovery failure
  falls back to the local address, and a static `Config::media_public_addr`
  takes precedence when the mapping is already known.
- Listener per-source request budget:
  `SipListenerAuthPolicy::with_source_rate_limit(SipSourceRateLimit)` drops
  over-budget sources (keyed by IP, trusted trunks included) before any other
  admission check, and `with_ingress_observer(Arc<dyn SipIngressObserver>)`
  reports every `SipIngressEvent` with its `SipIngressOutcome` (`Admitted`,
  `Rejected`, or `Dropped`).
- Hold/resume, blind transfer, REFER/NOTIFY progress, attended-transfer
  primitives, and transfer outcome events.
- Builder-shaped outbound requests with custom headers, carry-through reports,
  header policy enforcement, body helpers, and SIP trace redaction hooks.
- B2BUA and gateway helpers under `server::*`, including bridge strategy,
  contact resolution, and transfer orchestration helpers.
- Performance recipes and tuning hooks for local labs, PBX media server
  profiles, and signaling-heavy test profiles.

## Current 0.3.11 release evidence

Protected run
[`34074372543`](https://github.com/eisenzopf/rvoip/actions/runs/34074372543)
qualified the exact published commit
`77a99cd38a07641294cf7dc547146b115b135dc7`: **213/213 gates passed**, all
213 were fresh, and all 108 legacy release requirements were covered.

| Evidence | Current record |
| --- | --- |
| Release disposition and provenance | [`docs/BETA_RELEASE_REPORT.md`](docs/BETA_RELEASE_REPORT.md) |
| Complete accepted-gate ledger | [`docs/BETA_GATE_REPORT.md`](docs/BETA_GATE_REPORT.md) |
| Performance observations | [`docs/BETA_PERFORMANCE_REPORT.md`](docs/BETA_PERFORMANCE_REPORT.md) |
| Detailed performance gate evaluation | [`current-performance-evaluation.md`](docs/releases/qualification/20260907T042726Z-34074372543/current-performance-evaluation.md) |
| Machine-readable performance evaluation | [`current-performance-evaluation.json`](docs/releases/qualification/20260907T042726Z-34074372543/current-performance-evaluation.json) |
| Evidence artifact index | [`current-performance-artifact-index.json`](docs/releases/qualification/20260907T042726Z-34074372543/current-performance-artifact-index.json) |
| Signed report manifest | [`QUALIFICATION_REPORT_ATTESTATION.json`](docs/QUALIFICATION_REPORT_ATTESTATION.json) |
| Immutable release archive | [`20260907T042726Z-34074372543`](docs/releases/qualification/20260907T042726Z-34074372543) |

The detailed evaluation records three clean 65,000-call canonical runs, a
high-density media burst with full audio delivery, a 60-minute monolithic
soak, and a 60-minute split 500-call soak. All recorded performance policies
passed. The exact measurements and claim boundaries are in the linked reports;
they are evidence for the tested candidate and environment, not a performance
SLA.

## Historical 0.3.2 exception evidence

The clean, unchanged full run recorded 106 PASS, 2 FAIL, and 0 SKIP results.
The project owner accepted one root policy deviation: high-density full-media
burst ASR was 0.9928 against the 0.995 requirement. The second failed record is
the reporting roll-up of that same miss, not an independent product failure.
All 16 selected PBX and interoperability gates passed.

| Area | Evidence |
| --- | --- |
| Full gate | `106 / 108` PASS, `2` FAIL, `0` SKIP; strict status NON-RC, release disposition APPROVED-WITH-EXCEPTION |
| PBX interop | Asterisk and FreeSWITCH all-API/all-scenario UDP/TLS matrices passed |
| Proxy targets | Kamailio/OpenSIPS de-scope audit passed; external proxy interop was not executed |
| Strict UA | baresip strict-UA matrix passed |
| SIPp standalone | 30, 100, 300, 1,000, and 2,000 CPS passed with 100% configured call completion |
| Security | dependency advisory audit and parser fuzz smoke passed |
| Canonical 2K | Three source-identical passes; `65,000 / 65,000` calls and ASR `1.0` in each run |
| Monolithic soak | 3,600 seconds, `587 / 587` calls, retained objects `0`, active audio receivers `0`, RSS gate `12.7 MB/hr` against `15 MB/hr` |
| Accepted deviation | High-density full-media burst `17,871 / 18,000`, ASR `0.9928`; all 129 failures were answer timeouts and non-timeout errors were zero |

For the exact claim boundaries and immutable evidence, see:

- [`docs/BETA_RELEASE_EXCEPTION.md`](docs/BETA_RELEASE_EXCEPTION.md)
- [`docs/BETA_GATE_EXCEPTION.md`](docs/BETA_GATE_EXCEPTION.md)
- [`docs/BETA_PERFORMANCE_EXCEPTION.md`](docs/BETA_PERFORMANCE_EXCEPTION.md)
- [`docs/BETA_RELEASE_CHECKLIST.md`](docs/BETA_RELEASE_CHECKLIST.md)
- [`docs/COMPATIBILITY_MATRIX.md`](docs/COMPATIBILITY_MATRIX.md)
- [`docs/RFC_COMPLIANCE_MATRIX.md`](docs/RFC_COMPLIANCE_MATRIX.md)
- [`docs/SECURITY_POSTURE.md`](docs/SECURITY_POSTURE.md)
- [`docs/TOPOLOGY_PROFILES.md`](docs/TOPOLOGY_PROFILES.md)
- [`docs/INTEROP_CI_PLAN.md`](docs/INTEROP_CI_PLAN.md)

## Extensions and native Vapi WebSocket agents

`rvoip-sip` stays focused on the SIP product, but it composes with all 14
optional workspace extension crates through the `rvoip` facade and shared
orchestrator.

| Group | Companion extensions |
| --- | --- |
| AI and conversation data | [`rvoip-harness`](../../extensions/rvoip-harness), [`rvoip-vapi`](../../extensions/rvoip-vapi), [`rvoip-vcon`](../../extensions/rvoip-vcon), [`rvoip-vcon-postgres`](../../extensions/rvoip-vcon-postgres) |
| Caller trust | [`rvoip-stir-shaken`](../../extensions/rvoip-stir-shaken) |
| Authentication providers | [`rvoip-oidc`](../../extensions/rvoip-oidc), [`rvoip-keycloak`](../../extensions/rvoip-keycloak), [`rvoip-ldap`](../../extensions/rvoip-ldap), [`rvoip-redis`](../../extensions/rvoip-redis), [`rvoip-ims-aka`](../../extensions/rvoip-ims-aka) |
| User lifecycle | [`rvoip-saml`](../../extensions/rvoip-saml), [`rvoip-scim`](../../extensions/rvoip-scim), [`rvoip-webauthn`](../../extensions/rvoip-webauthn) |
| Audit and observability | [`rvoip-audit`](../../extensions/rvoip-audit) |

New in 0.3.2, `rvoip-vapi` is a native Rust `ConnectionAdapter` for Vapi's
bidirectional raw-audio WebSocket transport. It can attach directly to an
active SIP or WebRTC caller connection, originate the Vapi agent, bridge
full-duplex μ-law 8 kHz or PCM 16 kHz audio, expose typed events and
control/context messages, and supervise both sides of teardown. No
third-party telephony intermediary is required between rvoip and Vapi.

Enable the facade integration with:

```toml
rvoip = { version = "0.3.11", features = ["sip", "vapi"] }
```

See the complete [`rvoip-vapi` README](../../extensions/rvoip-vapi/README.md),
the runnable [`14-vapi-agent`](../../../examples/14-vapi-agent) server, and the
[full extension catalog](../../../README.md#extensions). The adapter and other
extensions remain developer-preview unless their own documentation states a
narrower qualification.

## Validation and operations

Local development checks:

```sh
RUSTUP_TOOLCHAIN=1.91 cargo check -p rvoip-sip --all-targets
crates/sip/rvoip-sip/scripts/beta_gate.sh --local
crates/sip/rvoip-sip/scripts/beta_gate.sh --security
```

Full external evidence requires the local PBX, SIPp, strict-UA, and performance
dependencies used by the gate script:

```sh
crates/sip/rvoip-sip/scripts/full_beta_release.sh
```

The wrapper prepares and strictly validates the Homebrew Docker/Colima stack,
both local PBX lab directories, the three canonical 2K evidence runs, every
external interop dependency, the literal-all performance configuration, and
packaged release reporting before it invokes the full gate.

Operational references:

- [`docs/SIGNALING_PERFORMANCE_ARCHITECTURE.md`](docs/SIGNALING_PERFORMANCE_ARCHITECTURE.md)
  for the sharded lookup, consolidated deadline, compact retention, bounded
  batch, generation-fencing, and other SIP-stack comparison rationale.
- [`docs/BENCHMARKING.md`](docs/BENCHMARKING.md) for reproducible performance
  test shapes and artifact conventions.
- [`docs/TUNING.md`](docs/TUNING.md) for runtime profile and deployment
  tuning guidance.
- [`docs/INTEROP_CI_PLAN.md`](docs/INTEROP_CI_PLAN.md) for PBX, SIPp, and
  strict-UA runner expectations.

## Feature flags

| Flag | Status |
| --- | --- |
| default | Empty default feature set used by the beta release baseline. |
| `event-history` | Optional retained event inspection for debugging and tests. |
| `persistence` | Experimental persistence hooks; applications must validate their own storage behavior. |
| `generated-validation` | Development and CI validation for generated SIP messages. |
| `dev-insecure-tls` | Local test-only TLS convenience; never enable for deployed systems. |
| `g729` | Optional G.729A/G.729AB media support with PT 18 SDP and Annex B `fmtp` negotiation. Also forwards `rvoip-core/g729` so the core media graph can transcode bridged legs. |
| `amr-nb` | Optional AMR narrowband media support with RFC 4867 payload framing, DTX, CMR, and `mode-set`/`octet-align` negotiation. |
| `amr-wb` | The same for AMR wideband (G.722.2) at 16 kHz. Also forwards `rvoip-core/amr-wb` so the core media graph can transcode bridged legs. |
| `amr` | Both AMR variants. |
| `opus` | Optional Opus media support; requires libopus on the build host. |
| `opus-sim` | Deprecated compatibility alias for `opus`; selects the same real backend. |
| `all-codecs` | `g729` + `opus` + `amr`. |
| `dtls-srtp` | SIP DTLS-SRTP keying over the call's RTP socket with SHA-256 SDP fingerprint binding. |
| `perf-tests` | Opt-in performance gate and benchmark support. |
| `dhat` | Heap profiling support for `examples/profiling/dhat_*.rs`. |
| `tokio-console` | Tokio console support for profiling examples; requires `RUSTFLAGS="--cfg tokio_unstable"`. |
| `test-hooks` | Test-only fault injection (`SipAdapter::inject_media_failure_for_test`); absent from ordinary builds. |
| `perf-infra-memory-diagnostics` | `perf-tests` plus `rvoip-infra-common` memory diagnostics for targeted investigation runs. |
| `perf-media-diagnostics` | `perf-tests` plus `rvoip-media-core` perf diagnostics. |
| `perf-media-memory-diagnostics` | `perf-tests` plus `rvoip-media-core` memory diagnostics. |
| `perf-rtp-memory-diagnostics` | `perf-tests` plus `rvoip-rtp-core` memory diagnostics. |
| `perf-call-setup-diagnostics` | `perf-tests` plus the hidden `call_setup_diag` state-machine instrumentation. |
| `perf-system-allocator` | Perf-only allocator A/B switch that disables the mimalloc global allocator. |

## Known limits

- The current qualification passed every gate, but PASS applies to the exact
  tested commit, commands, peers, and environment; it is not a broad
  production-readiness claim.
- Carrier SBC readiness is partial and not certified.
- Kamailio/OpenSIPS plus rtpengine are release-gated only in the proxy matrix
  recorded in the gate ledger (both adjacency orders over UDP, TCP, and TLS,
  plus the rtpengine AMR passthrough run); other proxy versions, images, and
  topologies are not claimed.
- WebRTC/browser interop, TURN, and WSS outbound remain outside the SIP beta
  claim unless separately completed and tested. SIP DTLS-SRTP is a distinct
  feature-gated 0.3.11 capability whose claim is bounded by fresh protected
  release evidence.
- The default full-media performance claim is bounded to the documented
  beta release profiles and artifacts. Higher tuned-profile results need
  their own topology, hardware, configuration, and caveats.
- Blind transfer is validated; attended transfer is exposed as primitives
  rather than a full consultation-call workflow.

## Contributing

Use the public issue tracker for bugs, interop gaps, and documentation problems:
[`github.com/eisenzopf/rvoip/issues`](https://github.com/eisenzopf/rvoip/issues).
When reporting SIP interop behavior, include the peer, transport, media
security mode, relevant SIP trace, and the smallest command or example that
reproduces the behavior.

## License

Licensed under the MIT license, See the repository
[`LICENSE`](https://github.com/eisenzopf/rvoip/blob/main/LICENSE).
