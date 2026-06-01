# 05 · Blind transfer (REFER)

> **Beta status: Supported.** REFER (RFC 3515) + NOTIFY transfer progress are
> interop-tested. See
> [`COMPATIBILITY_MATRIX.md`](../../crates/sip/rvoip-sip/docs/COMPATIBILITY_MATRIX.md).

## Overview

A three-party blind transfer: **Alice** calls **Bob**; Bob blind-transfers her
to **Charlie**; Alice ends up talking to Charlie. "Blind" means Bob transfers
without first talking to Charlie (contrast with [06-attended-transfer](../06-attended-transfer/)).

Bob uses `transfer_blind_and_wait`, which sends the REFER and blocks until the
transfer lifecycle resolves. Alice drives the completion explicitly: on
`Event::ReferReceived` she hangs up with Bob and calls the REFER target.

## Demo flow

1. Alice (`:5060`) calls Bob (`:5061`); Bob answers.
2. After a moment Bob calls `transfer_blind_and_wait("sip:charlie@…")` → REFER.
3. Alice receives `ReferReceived`, hangs up with Bob, and calls Charlie (`:5062`).
4. Charlie (already waiting) answers; Alice ↔ Charlie is up. Everyone winds down.

## Architecture

```
   Alice (:5060) ── INVITE ─▶ Bob (:5061)
        │  ◀──── REFER (Refer-To: Charlie) ───┘
        │ hangup Bob
        └──── INVITE ─▶ Charlie (:5062)  ✅ connected
```

## Quick start

```sh
./run_demo.sh
```

Or manually, in three terminals (ports via env, defaults shown):

```sh
CHARLIE_PORT=5062 cargo run --bin charlie
BOB_PORT=5061 CHARLIE_PORT=5062 cargo run --bin bob
ALICE_PORT=5060 BOB_PORT=5061 cargo run --bin alice
```

## Expected output

```text
  [ALICE] Connected to Bob!
  [ALICE] Got REFER to sip:charlie@127.0.0.1:5062
  [ALICE] ✅ Connected to Charlie!
  [BOB] ✅ REFER accepted
  [CHARLIE] ✅ Answered!

✅ DEMO SUCCESSFUL — Alice was transferred to Charlie
```

## Configuration

Ports come from `ALICE_PORT` / `BOB_PORT` / `CHARLIE_PORT` (defaults
`5060` / `5061` / `5062`) so the three binaries can be placed independently.

## Beta scope notes

- REFER + the NOTIFY progress lifecycle (RFC 3515 / RFC 4235) are interop-tested.
- The transferee-drives-completion pattern keeps the flow explicit; the
  `transfer_blind_and_wait` outcome distinguishes REFER acceptance from failure.

## Troubleshooting

- **Alice never gets the REFER** — Bob must be answered first; `run_demo.sh`
  starts Charlie and Bob before Alice.
- **Port conflicts** — override the `*_PORT` env vars.

## Next steps

- [06-attended-transfer](../06-attended-transfer/) — consult the target first.
- In-crate reference: `stream_peer/05_blind_transfer`.
