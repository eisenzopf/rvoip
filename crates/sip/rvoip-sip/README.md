# rvoip-sip

[![Crates.io](https://img.shields.io/crates/v/rvoip-sip.svg)](https://crates.io/crates/rvoip-sip)
[![docs.rs](https://docs.rs/rvoip-sip/badge.svg)](https://docs.rs/rvoip-sip)
[![Rust 1.91+](https://img.shields.io/badge/rust-1.91%2B-orange.svg)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/eisenzopf/rvoip/blob/main/LICENSE)
[![Repository](https://img.shields.io/badge/github-eisenzopf%2Frvoip-24292f.svg)](https://github.com/eisenzopf/rvoip)
[![GitHub issues](https://img.shields.io/github/issues/eisenzopf/rvoip.svg)](https://github.com/eisenzopf/rvoip/issues)

`rvoip-sip` is the application-facing SIP session layer for RVoIP. It
coordinates dialog state, registration, media setup, call control, transfer,
DTMF, hold/resume, custom SIP headers, and app-visible events so Rust
applications can behave like programmable SIP endpoints without owning SIP
transaction or RTP details directly.

The workspace is preparing the strict-gate `0.3.5` release candidate. It can
publish only after a fresh full-beta run passes without skipped gates and is
bound to the exact clean release source. The generated
[beta release report](docs/BETA_RELEASE_REPORT.md) is authoritative for the
tested PBX, proxy, SIPp, strict-UA, security, performance, and soak boundaries.
Historical exception and carry-forward reports remain immutable history and
do not qualify `0.3.5`.

## At a glance

| Need | Start with |
| --- | --- |
| Make calls from a softphone or PBX account | [`Endpoint`](https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.Endpoint.html) |
| Write a sequential client, script, or test | [`StreamPeer`](https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.StreamPeer.html) |
| Build a reactive server, IVR, router, or queue | [`CallbackPeer`](https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.CallbackPeer.html) |
| Compose multiple call legs or a B2BUA | [`UnifiedCoordinator`](https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.UnifiedCoordinator.html) |
| Control an active call | [`SessionHandle`](https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.SessionHandle.html) |
| Check Asterisk, FreeSWITCH, Kamailio, or OpenSIPS status | [Interoperability status](#interoperability-status) |
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
rvoip-sip = "0.3.5"
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
state, typed per-call events, and decoded/encoded audio frames.

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

PBX interop setup, environment variables, and scenario coverage are documented
in [`examples/pbx/README.md`](examples/pbx/README.md). The terminal softphone
is documented in [`examples/sip_client/README.md`](examples/sip_client/README.md).

## Interoperability status

The `0.3.5` candidate requires revision-bound PASS evidence for Asterisk,
FreeSWITCH, Kamailio, and OpenSIPS. Kamailio and OpenSIPS must each pass both
adjacency orders over UDP, TCP, and verified TLS. Publication remains blocked
unless the generated report records the complete required matrix as PASS.

The 0.3.2 full release run passed all 16 selected PBX and interoperability
gates. Asterisk and FreeSWITCH were executed as external PBX peers; Kamailio
and OpenSIPS were named and audited, but their proxy/RTPengine topology was
explicitly de-scoped rather than silently presented as tested.

| Peer/tool | 0.3.2 status | Executed scope |
| --- | --- | --- |
| **Asterisk** | **Interop matrix passed** | `Endpoint`, `StreamPeer`, and `CallbackPeer` across registration, basic call, G.729A/G.729AB, hold/resume, ring-cancel, RFC 4733 DTMF, rejection, and blind transfer over UDP and TLS |
| **FreeSWITCH** | **Interop matrix passed** | The same API, scenario, codec, and UDP/TLS matrix as Asterisk |
| **SIPp** | **Standalone matrix passed** | 30, 100, 300, 1,000, and 2,000 CPS with 100% configured call completion |
| **baresip** | **Strict-UA check passed** | External user-agent call against the rvoip SIP listener |
| **Kamailio** | **Not release-tested** | Proxy/RTPengine investigation track; only the explicit de-scope audit passed |
| **OpenSIPS** | **Not release-tested** | Proxy/RTPengine investigation track; only the explicit de-scope audit passed |

The machine-bound [0.3.2 gate
record](docs/BETA_GATE_EXCEPTION.md), [compatibility
matrix](docs/COMPATIBILITY_MATRIX.md), and [topology
profiles](docs/TOPOLOGY_PROFILES.md) define the exact claim. These results do
not imply carrier certification or untested peer-version/topology coverage.

## Capabilities

- SIP call setup and teardown with registration lifecycle support.
- INVITE, REGISTER, BYE, CANCEL, REFER, NOTIFY, INFO, PRACK, session timer,
  redirect, provisional response, and glare-retry paths covered by examples or
  regression fixtures.
- UDP and TLS SIP paths in the beta-candidate evidence set.
- RTP media sessions, bidirectional audio frames, RFC 4733 DTMF, and
  SDES-SRTP negotiation state. The exact supported and fail-closed boundaries
  are documented in [Crypto capability boundaries](docs/CRYPTO_CAPABILITIES.md).
- Hold/resume, blind transfer, REFER/NOTIFY progress, attended-transfer
  primitives, and transfer outcome events.
- Builder-shaped outbound requests with custom headers, carry-through reports,
  header policy enforcement, body helpers, and SIP trace redaction hooks.
- B2BUA and gateway helpers under `server::*`, including bridge strategy,
  contact resolution, and transfer orchestration helpers.
- Performance recipes and tuning hooks for local labs, PBX media server
  profiles, and signaling-heavy test profiles.

## 0.3.2 release evidence

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
rvoip = { version = "0.3.2", features = ["sip", "vapi"] }
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
| `g729` | Optional G.729A/G.729AB media support with PT 18 SDP and Annex B `fmtp` negotiation. |
| `perf-tests` | Opt-in performance gate and benchmark support. |
| `dhat` | Heap profiling support for `examples/profiling/dhat_*.rs`. |
| `tokio-console` | Tokio console support for profiling examples; requires `RUSTFLAGS="--cfg tokio_unstable"`. |

## Known limits

- This is a beta release approved with one performance exception, not a broad
  production-readiness claim.
- Carrier SBC readiness is partial and not certified.
- Kamailio/OpenSIPS plus RTPengine were explicitly de-scoped; they are named
  validation targets, not 0.3.2 interoperability claims.
- WebRTC/browser interop, ICE, TURN, DTLS-SRTP, and WSS outbound are outside
  the SIP beta claim unless separately completed and tested.
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
