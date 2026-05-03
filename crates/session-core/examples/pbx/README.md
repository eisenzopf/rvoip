# Unified PBX Interop Examples

This directory is the source of truth for `session-core` PBX interop examples.
The same scenario code can run against Asterisk or FreeSWITCH and through three
public API surfaces:

- `Endpoint`: simple account/profile API
- `StreamPeer`: event-stream peer API
- `CallbackPeer::builder`: closure-based reactive callback API

## Setup

Copy the provider template you need and edit local addresses and credentials:

```sh
cp env/asterisk.env.example env/asterisk.env
cp env/freeswitch.env.example env/freeswitch.env
```

`run.sh` also loads `examples/pbx/.env.local` when present. FreeSWITCH runs
also load `$HOME/Developer/freeswitch/freeswitch-local.env` when present.

## Runner

```sh
./run.sh --pbx asterisk --api all --scenario registration
./run.sh --pbx freeswitch --api all --scenario hold_resume
./run.sh --pbx both --api all --scenario all
```

Options:

- `--pbx asterisk|freeswitch|both`
- `--api endpoint|peer|streampeer|callback|all`
- `--scenario registration|basic_call|hold_resume|ring_cancel|dtmf|reject|blind_transfer|all`

The runner builds the four unified Cargo examples and stores logs/WAV evidence
under `examples/pbx/output/<provider>/<api>/<scenario>/<transport>/`.

## Cargo Examples

The runner orchestrates these examples by setting `PBX_PROVIDER`,
`PBX_SCENARIO`, `PBX_TRANSPORT`, and `PBX_ROLE`.

```sh
cargo run -p rvoip-session-core --features dev-insecure-tls --example pbx_streampeer
cargo run -p rvoip-session-core --features dev-insecure-tls --example pbx_endpoint
cargo run -p rvoip-session-core --features dev-insecure-tls --example pbx_callback_builder
cargo run -p rvoip-session-core --features dev-insecure-tls --example pbx_analyze
```

## Scenario Matrix

The unified suite exercises these scenarios against both PBXs and all three API
surfaces:

- registration/unregistration for TLS `1001` and UDP `2001`
- basic UDP call `2001 -> 2002`
- UDP and TLS/SRTP hold/resume
- UDP and TLS/SRTP ring/cancel
- UDP and TLS/SRTP DTMF
- UDP and TLS/SRTP reject/busy
- UDP and TLS/SRTP blind transfer to `2003`/`1003`

Asterisk registered-flow TLS/SRTP is provider-gated: set
`ASTERISK_TLS_CONTACT_MODE=registered-flow-symmetric` or
`ASTERISK_TLS_FLOW_REUSE=1` and run the TLS scenarios.

## Endpoint Notes

`Endpoint` intentionally remains a simple account API. Advanced scenarios still
use `SessionHandle` for per-call operations such as `hold`, `resume`,
`send_dtmf`, and `transfer_blind_and_wait_for_outcome`. Ring/cancel and reject
use `Endpoint::wait_for_incoming` plus `IncomingCall` decisions. This keeps the
simple Endpoint setup path under test while documenting where advanced
operations belong on the per-call handle.

## Provider Differences

Provider-specific differences are encoded in config defaults and capability
flags, not duplicated scenario code:

- Asterisk defaults to `SIP_PORT=5060`, `SIP_TLS_PORT=5061`,
  `SIP_PASSWORD=password123`, longer registration settle/retry windows, and
  optional registered-flow operation.
- FreeSWITCH defaults to `FREESWITCH_UDP_ADDR=127.0.0.1:5062`,
  `FREESWITCH_TLS_ADDR=127.0.0.1:5063`, `FREESWITCH_PASSWORD=1234`,
  `15070/15080` local SIP ports, and `SrtpSuitePolicy::FreeSwitchCompatible`.
- Asterisk target-side CANCEL may not always be surfaced by the PBX profile, so
  caller-side cancel remains the required assertion unless
  `ASTERISK_EXPECT_TARGET_CANCEL=1` is set.
