# Examples

Multi-peer examples run each SIP peer as a separate OS process. This models real deployments (SIP peers are typically separate processes or machines) and avoids shared-state issues from running multiple peers in one process.

Each multi-peer example has a `run.sh` script that starts every peer and multiplexes their output with colored prefixes (`[SERVER]`, `[CLIENT]`, etc.):

```bash
./examples/<category>/<name>/run.sh
```

You can also run each peer separately in its own terminal for step-by-step debugging. For verbose logging, set `RUST_LOG=rvoip_session_core=debug`.

## Getting Started

Single-process example (two peers in one binary — good for a quick intro):

| Command | Description |
|---------|-------------|
| `cargo run --example hello` | Make and receive a SIP call |

## StreamPeer — sequential / client-side API

Use `StreamPeer` for clients, scripts, and test tools. Call methods, await results.

| Script | Description |
|--------|-------------|
| `./examples/streampeer/dtmf/run.sh` | Send DTMF digits during a call |
| `./examples/streampeer/hold_resume/run.sh` | Put a call on hold and resume it |
| `./examples/streampeer/audio/run.sh` | Bidirectional audio exchange with WAV output |
| `./examples/streampeer/blind_transfer/run.sh` | Three-party blind transfer (REFER) |
| `./examples/streampeer/registration/run.sh` | Register with a SIP registrar server |

## CallbackPeer — reactive / server-side API

Use `CallbackPeer` for servers, proxies, and IVR systems. Implement the `CallHandler` trait or use a built-in handler.

| Script | Description |
|--------|-------------|
| `./examples/callbackpeer/auto_answer/run.sh` | Auto-answer every call (simplest server) |
| `./examples/callbackpeer/closure/run.sh` | Closure-based handler, no trait needed |
| `./examples/callbackpeer/routing/run.sh` | Route calls by URI pattern matching |
| `./examples/callbackpeer/ivr/run.sh` | IVR menu with DTMF navigation |
| `./examples/callbackpeer/queue/run.sh` | Call center queue with deferred accept |
| `./examples/callbackpeer/custom/run.sh` | Full `CallHandler` trait (all 5 methods) |

## Advanced

| Script / Command | Description |
|------------------|-------------|
| `./examples/advanced/concurrent_calls/run.sh` | 5 concurrent callers + 1 answerer |
| `cargo run --example advanced_registrar_server` | Standalone registrar server (pair with `streampeer_registration_client`) |

## Asterisk

Remote Asterisk examples use `examples/asterisk/.env` for PBX address,
credentials, local bind address, and media ports.

| Script | Description |
|--------|-------------|
| `./examples/asterisk/run.sh` | Run the full sequence: TLS registration, UDP registration, TLS/SRTP hold/resume, then UDP hold/resume |
| `./examples/asterisk/registration/run.sh` | Register secure user 1001 over SIP TLS, then UDP user 2001 |
| `./examples/asterisk/tls_srtp_hold_resume/run.sh` | Register TLS/SRTP users 1001/1002 over SIP TLS, require SDES-SRTP, exercise hold/resume, and verify pre/post-resume audio |
| `./examples/asterisk/udp_hold_resume/run.sh` | Register UDP users 2001/2002, exercise hold/resume, and verify pre/post-resume audio |

The Asterisk lab profile uses users `1001` and `1002` for SIP TLS with
mandatory SDES-SRTP, and users `2001` and `2002` for UDP/plain RTP. All four
users share the single `SIP_PASSWORD` value from `.env`; endpoint auth
usernames default to the endpoint number.

For the TLS/SRTP Asterisk example, configure Asterisk PJSIP with a TLS
transport and endpoint media encryption set to mandatory SDES/SRTP. The default
example mode is a reachable TLS Contact: RVOIP registers over TLS and also
listens on `ENDPOINT_1001_TLS_LOCAL_PORT` / `ENDPOINT_1002_TLS_LOCAL_PORT` so
Asterisk can open TLS connections to the registered Contacts for `INVITE`,
`OPTIONS`, `BYE`, and re-INVITEs. That listener requires `TLS_CERT_PATH` and
`TLS_KEY_PATH`; if they are unset, `run.sh` generates a short-lived self-signed
certificate/key under the test output directory. If your Asterisk TLS policy
verifies the endpoint listener certificate, configure Asterisk to trust that
certificate or provide a trusted pair explicitly. Outbound verification of
Asterisk uses system roots, `TLS_CA_PATH`, or dev-only `TLS_INSECURE=1`.

Registered-flow mode is also available for Asterisk configurations using
`rewrite_contact` / `symmetric_transport`: set
`ASTERISK_TLS_CONTACT_MODE=registered-flow`. In that mode RVOIP keeps the
outbound TLS flow open and does not need a listener certificate/key.

## Running peers individually

Each peer is also a separate `cargo` example binary, so you can run them in separate terminals for debugging. For example, for `callbackpeer/auto_answer`:

```bash
# Terminal 1
cargo run -p rvoip-session-core --example callbackpeer_auto_answer_server

# Terminal 2
cargo run -p rvoip-session-core --example callbackpeer_auto_answer_client
```

Run `cargo run -p rvoip-session-core --example` with no name to see the full list.
