# rvoip-sip

[![Crates.io](https://img.shields.io/crates/v/rvoip-sip.svg)](https://crates.io/crates/rvoip-sip)
[![docs.rs](https://docs.rs/rvoip-sip/badge.svg)](https://docs.rs/rvoip-sip)
[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/eisenzopf/rvoip/blob/main/LICENSE)
[![Repository](https://img.shields.io/badge/github-eisenzopf%2Frvoip-24292f.svg)](https://github.com/eisenzopf/rvoip)
[![GitHub issues](https://img.shields.io/github/issues/eisenzopf/rvoip.svg)](https://github.com/eisenzopf/rvoip/issues)

`rvoip-sip` is the application-facing SIP session layer for RVoIP. It
coordinates dialog state, registration, media setup, call control, transfer,
DTMF, hold/resume, custom SIP headers, and app-visible events so Rust
applications can behave like programmable SIP endpoints without owning SIP
transaction or RTP details directly.

This crate is currently a **beta candidate** for bounded SIP client, server,
PBX, gateway, and B2BUA scenarios. It is intended for developers who want a
Rust-native SIP control surface with runnable examples and explicit interop
evidence.

## At a glance

| Need | Start with |
| --- | --- |
| Make calls from a softphone or PBX account | [`Endpoint`](https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.Endpoint.html) |
| Write a sequential client, script, or test | [`StreamPeer`](https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.StreamPeer.html) |
| Build a reactive server, IVR, router, or queue | [`CallbackPeer`](https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.CallbackPeer.html) |
| Compose multiple call legs or a B2BUA | [`UnifiedCoordinator`](https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.UnifiedCoordinator.html) |
| Control an active call | [`SessionHandle`](https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.SessionHandle.html) |

Start with `Endpoint` unless you already know you need event-stream ownership,
callback dispatch, or custom multi-leg orchestration. The higher-level surfaces
are thin wrappers over `UnifiedCoordinator`, so applications can move down a
level without switching protocol stacks.

## Install

`rvoip-sip` uses the workspace minimum supported Rust version. The current MSRV
is **Rust 1.88**.

```toml
[dependencies]
rvoip-sip = "0.2"
tokio = { version = "1", features = ["full"] }
```

For repository development:

```sh
git clone https://github.com/eisenzopf/rvoip.git
cd rvoip
RUSTUP_TOOLCHAIN=1.88 cargo check -p rvoip-sip --all-targets
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
| Callback IVR pair | `./crates/rvoip-sip/examples/callback_peer/03_builder_ivr/run.sh` |
| Unified B2BUA bridge | `./crates/rvoip-sip/examples/unified/04_b2bua_bridge/run.sh` |
| Terminal softphone | `cargo run -p rvoip-sip --example sip_client` |
| Asterisk/FreeSWITCH interop | `./crates/rvoip-sip/examples/pbx/run.sh --pbx asterisk --api all --scenario registration` |

PBX interop setup, environment variables, and scenario coverage are documented
in [`examples/pbx/README.md`](examples/pbx/README.md). The terminal softphone
is documented in [`examples/sip_client/README.md`](examples/sip_client/README.md).

## Capabilities

- SIP call setup and teardown with registration lifecycle support.
- INVITE, REGISTER, BYE, CANCEL, REFER, NOTIFY, INFO, PRACK, session timer,
  redirect, provisional response, and glare-retry paths covered by examples or
  regression fixtures.
- UDP and TLS SIP paths in the beta-candidate evidence set.
- RTP media sessions, bidirectional audio frames, RFC 4733 DTMF, and
  SDES-SRTP negotiation state.
- Hold/resume, blind transfer, REFER/NOTIFY progress, attended-transfer
  primitives, and transfer outcome events.
- Builder-shaped outbound requests with custom headers, carry-through reports,
  header policy enforcement, body helpers, and SIP trace redaction hooks.
- B2BUA and gateway helpers under `server::*`, including bridge strategy,
  contact resolution, and transfer orchestration helpers.
- Performance recipes and tuning hooks for local labs, PBX media server
  profiles, and signaling-heavy test profiles.

## Beta-candidate evidence

The beta-candidate gate completed with 0 failures and 0 skips from a clean tree
with Rust/Cargo `1.88.0`. The full evidence bundle is generated locally under
`beta-report/` by the gate script (an untracked artifact directory, not part of
the repository); the committed performance summary lives in
[`docs/BETA_PERFORMANCE_REPORT.md`](docs/BETA_PERFORMANCE_REPORT.md).

| Area | Evidence |
| --- | --- |
| Gate result | `0` failures, `0` skips |
| PBX interop | `192 / 192` Asterisk and FreeSWITCH rows passed |
| Strict UA | baresip strict-UA evidence archived |
| SIPp standalone | 30, 100, 300, 1,000, and 2,000 CPS matrix passed |
| Security | dependency advisory audit and parser fuzz smoke passed |
| Soak | `35,109 / 35,109` calls, ASR `1.0`, retained objects `0`, Bob active audio receivers `0` |
| Memory | peak RSS `292.1 MB`, post-drain RSS slope `1.5 MB/hr` against a `10 MB/hr` gate |

The 24-hour soak is explicitly waived for the beta candidate; the archived
30-minute soak is the accepted beta-candidate bar. For the exact claim
boundaries, see:

- [`docs/BETA_RELEASE_CHECKLIST.md`](docs/BETA_RELEASE_CHECKLIST.md)
- [`docs/COMPATIBILITY_MATRIX.md`](docs/COMPATIBILITY_MATRIX.md)
- [`docs/RFC_COMPLIANCE_MATRIX.md`](docs/RFC_COMPLIANCE_MATRIX.md)
- [`docs/SECURITY_POSTURE.md`](docs/SECURITY_POSTURE.md)
- [`docs/BETA_PERFORMANCE_REPORT.md`](docs/BETA_PERFORMANCE_REPORT.md)
- [`docs/TOPOLOGY_PROFILES.md`](docs/TOPOLOGY_PROFILES.md)
- [`docs/INTEROP_CI_PLAN.md`](docs/INTEROP_CI_PLAN.md)

## Validation and operations

Local development checks:

```sh
RUSTUP_TOOLCHAIN=1.88 cargo check -p rvoip-sip --all-targets
crates/rvoip-sip/scripts/beta_gate.sh --local
crates/rvoip-sip/scripts/beta_gate.sh --security
```

Full external evidence requires the local PBX, SIPp, strict-UA, and performance
dependencies used by the gate script:

```sh
BETA_RUN_LOCAL_PBX=1 RUSTUP_TOOLCHAIN=1.88 \
  crates/rvoip-sip/scripts/beta_gate.sh --full --require-external
```

Operational references:

- [`docs/BENCHMARKING.md`](docs/BENCHMARKING.md) for reproducible performance
  test shapes and artifact conventions.
- [`docs/TUNING.md`](docs/TUNING.md) for runtime profile and deployment
  tuning guidance.
- [`docs/INTEROP_CI_PLAN.md`](docs/INTEROP_CI_PLAN.md) for PBX, SIPp, and
  strict-UA runner expectations.

## Feature flags

| Flag | Status |
| --- | --- |
| default | Empty default feature set used by the beta-candidate baseline. |
| `event-history` | Optional retained event inspection for debugging and tests. |
| `persistence` | Experimental persistence hooks; applications must validate their own storage behavior. |
| `generated-validation` | Development and CI validation for generated SIP messages. |
| `dev-insecure-tls` | Local test-only TLS convenience; never enable for deployed systems. |
| `perf-tests` | Opt-in performance gate and benchmark support. |
| `dhat` | Heap profiling support for `examples/profiling/dhat_*.rs`. |
| `tokio-console` | Tokio console support for profiling examples; requires `RUSTFLAGS="--cfg tokio_unstable"`. |

## Known limits

- This is a beta candidate, not a broad production-readiness claim.
- Carrier SBC readiness is partial and not certified.
- Kamailio/OpenSIPS plus RTPengine are de-scoped from beta-candidate claims.
- WebRTC/browser interop, ICE, TURN, DTLS-SRTP, and WSS outbound are outside
  the beta-candidate claim unless separately completed and tested.
- The default full-media performance claim is bounded to the documented
  beta-candidate profiles and artifacts. Higher tuned-profile results need
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
