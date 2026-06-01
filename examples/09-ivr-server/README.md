# 09 · Reactive inbound server (IVR / routing)

> **Beta status: Supported.** The `CallbackPeer` reactive surface is supported.
> See
> [`COMPATIBILITY_MATRIX.md`](../../crates/sip/rvoip-sip/docs/COMPATIBILITY_MATRIX.md).

## Overview

Where `StreamPeer` is sequential (you call it), [`CallbackPeer`] is **reactive**
(it calls you): you register hooks and the library dispatches typed events into
them. That's the right model for servers — IVRs, routing front-ends, auto-answer
endpoints. This demo wires the builder form with `on_incoming`, `on_established`,
`on_dtmf`, and `on_ended`, and a scripted caller exercises it.

## Demo flow

1. **server** (the IVR, `:5120`) accepts inbound calls and reacts to DTMF. Press
   `0` and it would blind-transfer you to an operator.
2. **caller** (`:5121`) connects and sends `1`, `2`, `#`.
3. The IVR logs each press; the caller hangs up.

## Architecture

```
   caller (:5121) ── INVITE ─▶ IVR server (:5120)
        │  ◀──────── 200 OK ───────  on_incoming → Accept
        │  ── DTMF 1, 2, # ────────▶ on_dtmf hook fires per digit
        │  ── BYE ─────────────────▶ on_ended
```

## Quick start

```sh
./run_demo.sh
```

Or manually:

```sh
cargo run --bin server
cargo run --bin caller
```

## Expected output

```text
  [ivr] listening on sip:ivr@127.0.0.1:5120
  [ivr] incoming call from …
  [ivr] ✅ call … established
  [ivr] call … pressed 1
  [caller] ✅ connected to IVR
  [caller] sent DTMF 1

✅ DEMO SUCCESSFUL — IVR reacted to inbound call + DTMF
```

## Beta scope notes

- The example uses the builder closures. For complex servers, implement the
  [`CallHandler`] trait (see the in-crate `callback_peer/06_trait_handler`), or
  use the built-in `RoutingHandler` / `QueueHandler`.
- Pressing `0` triggers `transfer_blind` to an operator URI that isn't started in
  this two-process demo, so the caller only sends `1`, `2`, `#`.

## Troubleshooting

- **Caller can't connect** — start the server first; `run_demo.sh` waits for the
  IVR's port.

## Next steps

- [10-call-center-b2bua](../10-call-center-b2bua/) — route + bridge across legs.
- In-crate references: `callback_peer/` (six numbered handler scenarios).

[`CallbackPeer`]: https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.CallbackPeer.html
[`CallHandler`]: https://docs.rs/rvoip-sip/latest/rvoip_sip/trait.CallHandler.html
