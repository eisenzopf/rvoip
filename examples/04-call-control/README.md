# 04 · Call control: hold, resume, DTMF

> **Beta status: Supported.** Re-INVITE hold/resume (RFC 3264) and RFC 4733 DTMF
> are beta-supported. See
> [`COMPATIBILITY_MATRIX.md`](../../crates/sip/rvoip-sip/docs/COMPATIBILITY_MATRIX.md).

## Overview

Once a call is up, all mid-call control happens through the [`SessionHandle`] —
the per-call object shared by every rvoip-sip API surface. This demo connects a
call and then drives **hold** (re-INVITE), **resume** (re-INVITE), and a short
**DTMF** burst (RFC 4733 telephone-event). The peer logs the DTMF it receives
off its per-call event stream.

## Demo flow

1. **peer** binds `:5061` and answers the inbound call.
2. **controller** binds `:5060`, dials the peer, and once connected:
   `hold()` → wait → `resume()` → wait → `send_dtmf('1' / '2' / '#')`.
3. peer prints each received DTMF digit, then both wind down on hangup.

## Architecture

```
   controller (:5060)                       peer (:5061)
        │  ── INVITE / 200 OK / ACK ────────▶ │
        │  ── re-INVITE (hold) ─────────────▶ │
        │  ── re-INVITE (resume) ───────────▶ │
        │  ── RTP telephone-event 1,2,# ────▶ │  logs DTMF
        │  ── BYE ──────────────────────────▶ │
```

## Quick start

```sh
./run_demo.sh
```

Or manually:

```sh
cargo run --bin peer -- --port 5061
cargo run --bin controller -- --port 5060 --peer-port 5061
```

## Expected output

```text
  [controller] connected as session-…
  [controller] ⏸  placed call on hold (re-INVITE)
  [controller] ▶  resumed call (re-INVITE)
  [controller] sent DTMF 1 (RFC 4733)
  …
  [peer] received DTMF 1
  [peer] ✅ done (received 3 DTMF digits)

✅ DEMO SUCCESSFUL — hold, resume, and DTMF exercised
```

## Command-line options

| Binary | Flag | Default |
|--------|------|---------|
| `controller` | `--port` / `--peer-port` | `5060` / `5061` |
| `peer` | `--port` | `5061` |

## Beta scope notes

- DTMF is **RFC 4733** (telephone-event in the RTP stream). SIP INFO DTMF is also
  available via `SessionHandle::send_info` if a peer requires it.
- Hold/resume are media-direction re-INVITEs (RFC 3264 offer/answer).

## Troubleshooting

- **No DTMF logged** — ensure the peer answered before the controller sent
  digits; the controller waits for `wait_for_answered` first.
- **Port conflicts** — change `--port` / `--peer-port`.

## Next steps

- [05-blind-transfer](../05-blind-transfer/) — move a call to a third party.
- In-crate reference: `stream_peer/02_call_control`.

[`SessionHandle`]: https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.SessionHandle.html
