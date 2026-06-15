# 01 · Quickstart: your first peer-to-peer SIP call

> **Beta status: Supported.** UDP transport (interop-tested) and PCMU/PCMA media
> are in the beta contract. See
> [`COMPATIBILITY_MATRIX.md`](../../crates/sip/rvoip-sip/docs/COMPATIBILITY_MATRIX.md).

## Overview

The smallest possible end-to-end SIP call: a **caller** process dials a
**callee** process over loopback, the callee answers, media flows for ~1 second,
and the caller hangs up. No server, no registration, no auth.

It uses the [`StreamPeer`] surface — a sequential client API where each helper
(`invite`, `wait_for_answered`, `hangup_and_wait`) blocks until the next
matching event. That keeps simple clients and scripts linear and easy to read.
Start here, then branch out to the other scenarios.

## Demo flow

1. **callee** binds `sip:callee@127.0.0.1:5061` and waits for an INVITE.
2. **caller** binds `:5060` and sends `INVITE sip:callee@127.0.0.1:5061`.
3. Stack drives `INVITE → 100/180 → 200 OK → ACK`; both sides report connected.
4. Media (PCMU RTP) flows for ~1 second.
5. **caller** sends `BYE`; the callee observes the call end. Both exit cleanly.

## Architecture

```
   caller (:5060)                         callee (:5061)
        │  ── INVITE ───────────────────────▶ │
        │  ◀──────────────── 180 Ringing ──── │
        │  ◀──────────────── 200 OK ───────── │
        │  ── ACK ──────────────────────────▶ │
        │  ◀═══════════ RTP / PCMU ═════════▶ │   (~1s)
        │  ── BYE ──────────────────────────▶ │
        │  ◀──────────────── 200 OK ───────── │
```

## Quick start

```sh
./run_demo.sh
```

Or run the two sides by hand in separate terminals:

```sh
cargo run --bin callee -- --port 5061
cargo run --bin caller -- --port 5060 --peer-port 5061
```

## Expected output

```text
  [caller] inviting sip:callee@127.0.0.1:5061
  [caller] ✅ call connected as session-08559c68-…
  [caller] ✅ call completed, hung up cleanly
  [callee] listening on sip:callee@127.0.0.1:5061
  [callee] incoming call from User <sip:caller@127.0.0.1:5060>;tag=a788fd3d
  [callee] ✅ answered session-45c62b50-…
  [callee] ✅ call ended

✅ DEMO SUCCESSFUL — P2P call established and torn down cleanly
```

## Command-line options

| Binary | Flag | Default | Meaning |
|--------|------|---------|---------|
| `caller` | `--port` | `5060` | Local SIP/UDP port to bind |
| `caller` | `--peer-port` | `5061` | Callee's SIP port to dial |
| `callee` | `--port` | `5061` | Local SIP/UDP port to bind |

Set `RUST_LOG=info` (or `debug`) for stack-level tracing.

## Beta scope notes

- Media in this demo is **PCMU/PCMA**. G.729A/G.729AB is optional and
  intentionally not shown here; Opus and G.722 are post-beta.
- Transport is **UDP** on loopback. TCP and TLS are also supported; see
  [08-tls-transport](../08-tls-transport/).

## Troubleshooting

- **`Address already in use`** — another process holds `:5060`/`:5061`. Pass
  different `--port` / `--peer-port` values.
- **Caller hangs at "inviting"** — the callee isn't up yet. `run_demo.sh` waits
  for the port; if running by hand, start the callee first.

## Next steps

- [02-softphone-audio](../02-softphone-audio/) — pump real PCMU audio frames.
- [04-call-control](../04-call-control/) — hold, resume, and DTMF.
- API-tier reference: `cargo run -p rvoip-sip --example stream_peer_basic_call`
  (this example is the two-process productization of that in-crate example).

[`StreamPeer`]: https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.StreamPeer.html
