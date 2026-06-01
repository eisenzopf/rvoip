# 06 · Attended transfer (consultative, REFER + Replaces)

> **Beta status: Supported.** REFER (RFC 3515) with an RFC 3891 `Replaces` is
> beta-supported. See
> [`COMPATIBILITY_MATRIX.md`](../../crates/sip/rvoip-sip/docs/COMPATIBILITY_MATRIX.md).

## Overview

An *attended* (consultative) transfer: the transferor talks to the transfer
target **before** completing the transfer. **Alice** calls **Bob**; Bob places a
**consultation** call to **Charlie**; then Bob attended-transfers Alice to
Charlie. Unlike a blind transfer, the REFER carries a `Replaces` header that
identifies the consultation dialog.

Three primitives make this work (all on [`SessionHandle`]):

- `consultation.dialog_identity()` → the SIP dialog identity (Call-ID + tags).
- `DialogIdentity::to_replaces_value()` → formats it as an RFC 3891 `Replaces`.
- `original.transfer_attended(target, &replaces)` → sends Alice a REFER whose
  Refer-To target embeds that `Replaces`.

Alice completes the transfer by calling the Refer-To target; the embedded
`Replaces` (also surfaced separately on `Event::ReferReceived.replaces`) ties the
new call to the consultation leg.

## Demo flow

1. Alice (`:5060`) calls Bob (`:5061`); Bob answers (the *original* call).
2. Bob calls Charlie (`:5062`) — the *consultation* call — and waits for answer.
3. Bob reads the consultation's `dialog_identity`, formats `Replaces`, and calls
   `transfer_attended` on the Alice leg.
4. Alice receives the REFER (with `Replaces`), hangs up with Bob, and calls the
   Refer-To target. Charlie answers the transferred call. ✅

## Architecture

```
   Alice (:5060) ── INVITE ─▶ Bob (:5061) ── INVITE (consult) ─▶ Charlie (:5062)
        │  ◀── REFER (Refer-To: Charlie; Replaces=consult dialog) ──┘
        │ hangup Bob
        └──────────────── INVITE (completes transfer) ─▶ Charlie  ✅ connected
```

## Quick start

```sh
./run_demo.sh
```

Or manually, in three terminals (defaults shown):

```sh
CHARLIE_PORT=5062 cargo run --bin charlie
BOB_PORT=5061 CHARLIE_PORT=5062 cargo run --bin bob
ALICE_PORT=5060 BOB_PORT=5061 cargo run --bin alice
```

## Expected output

```text
  [ALICE] Got REFER to sip:charlie@127.0.0.1:5062
  [ALICE] Replaces = Some("session-…@rvoip-sip;to-tag=…;from-tag=…")
  [ALICE] ✅ Connected to Charlie (attended transfer complete).
  [BOB] Consultation with Charlie established.
  [BOB] Attended-transferring Alice to Charlie (Replaces=…)
  [CHARLIE] Consultation answered.
  [CHARLIE] ✅ Answered transferred call.

✅ DEMO SUCCESSFUL — Alice was attended-transferred to Charlie
```

## Beta scope notes

- The transferor side uses the supported attended-transfer primitives
  (`dialog_identity`, `to_replaces_value`, `transfer_attended`). The transferee
  side (placing the replacing INVITE) is application orchestration — this example
  shows the canonical implementation.

## Troubleshooting

- **"consultation dialog identity not confirmed yet"** — the consultation call
  wasn't fully answered before Bob read its identity; the example waits, but a
  slow/declined Charlie can trip this.
- **Port conflicts** — override the `*_PORT` env vars.

## Next steps

- [05-blind-transfer](../05-blind-transfer/) — the simpler, no-consultation form.
- [10-call-center-b2bua](../10-call-center-b2bua/) — server-side call handling.
- API reference: [`SessionHandle::transfer_attended`](https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.SessionHandle.html#method.transfer_attended).

[`SessionHandle`]: https://docs.rs/rvoip-sip/latest/rvoip_sip/struct.SessionHandle.html
