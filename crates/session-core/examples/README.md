# Session-Core Examples

The examples are organized by public developer surface. Start with the API that
matches the shape of your application, then move down the ordered directories
from basic to more complex.

| Lane | Purpose |
| --- | --- |
| `endpoint/` | Simple account/profile API for softphones, PBX accounts, and demos |
| `stream_peer/` | Sequential clients, scripts, softphones, and test tools |
| `callback_peer/` | Reactive servers, IVR, routing, and queue-style applications |
| `unified/` | Explicit session orchestration for bridges, gateways, and B2BUA-style code |
| `regression/` | Protocol fixtures and behavior evidence |
| `pbx/` | Asterisk/FreeSWITCH interop matrix |

Run the local developer examples:

```sh
./crates/session-core/examples/run_all.sh
```

Run the protocol regression fixtures:

```sh
./crates/session-core/examples/regression/run_all.sh
```

## Endpoint

| Command | Description |
| --- | --- |
| `cargo run -p rvoip-session-core --example endpoint_local_call` | Two local endpoints make and receive a call |
| `cargo run -p rvoip-session-core --example endpoint_audio_roundtrip` | Two local endpoints exchange audio tones and verify received media |
| `cargo run -p rvoip-session-core --example endpoint_incoming_redirect` | Receive an INVITE and send SIP 302 redirect |
| `cargo run -p rvoip-session-core --example endpoint_registered_account` | Register to a PBX and call `SIP_TARGET` |

`endpoint_registered_account` is env-driven and is not part of the local
`run_all.sh` path. It expects `SIP_REGISTRAR`, `SIP_USERNAME`, `SIP_PASSWORD`,
and `SIP_TARGET`.

## StreamPeer

| Script / Command | Description |
| --- | --- |
| `cargo run -p rvoip-session-core --example stream_peer_basic_call` | Minimal sequential call flow |
| `./crates/session-core/examples/stream_peer/02_call_control/run.sh` | Hold, resume, and DTMF using `SessionHandle` |
| `./crates/session-core/examples/stream_peer/03_audio/run.sh` | Bidirectional audio exchange with optional WAV output |
| `./crates/session-core/examples/stream_peer/04_registration/run.sh` | Register and unregister through `StreamPeer` |
| `./crates/session-core/examples/stream_peer/05_blind_transfer/run.sh` | Three-party blind transfer |
| `./crates/session-core/examples/stream_peer/06_concurrent_calls/run.sh` | Multiple concurrent callers |

## CallbackPeer

| Script | Description |
| --- | --- |
| `./crates/session-core/examples/callback_peer/01_auto_answer/run.sh` | Built-in auto-answer handler |
| `./crates/session-core/examples/callback_peer/02_closure_gatekeeper/run.sh` | Closure-based incoming-call decision |
| `./crates/session-core/examples/callback_peer/03_builder_ivr/run.sh` | Builder hooks for incoming, established, DTMF, and ended events |
| `./crates/session-core/examples/callback_peer/04_routing_handler/run.sh` | URI-pattern routing handler |
| `./crates/session-core/examples/callback_peer/05_queue_handler/run.sh` | Deferred accept through a queue handler |
| `./crates/session-core/examples/callback_peer/06_trait_handler/run.sh` | Custom `CallHandler` implementation |

## UnifiedCoordinator

| Script / Command | Description |
| --- | --- |
| `cargo run -p rvoip-session-core --example unified_basic_call` | Minimal explicit-session call flow |
| `cargo run -p rvoip-session-core --example unified_event_filters` | Global vs per-session event streams |
| `cargo run -p rvoip-session-core --example unified_registration_server` | Standalone local registrar |
| `./crates/session-core/examples/unified/04_b2bua_bridge/run.sh` | Two-leg B2BUA-style RTP bridge |

`unified_registration_server` is a standalone server and is not included in
`run_all.sh`.

## Regression Fixtures

The `regression/` lane keeps examples that are useful as executable evidence
but too protocol-heavy for the first learning path: DTMF round-trip, TLS, SRTP,
CANCEL, PRACK, session timers, glare retry, and NOTIFY.

## PBX Interop

`pbx/` is the source of truth for Asterisk and FreeSWITCH interop. It runs the
same scenario matrix through `Endpoint`, `StreamPeer`, and
`CallbackPeer::builder`.

```sh
./crates/session-core/examples/pbx/run.sh --pbx asterisk --api all --scenario registration
./crates/session-core/examples/pbx/run.sh --pbx freeswitch --api all --scenario hold_resume
```

The full PBX matrix is opt-in because it depends on local PBX configuration.
