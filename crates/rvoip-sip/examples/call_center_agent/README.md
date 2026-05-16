# Call-center agent walkthrough

**Pattern mapping:** SIP_API_DESIGN_2 §11.2 (CallbackPeer for inbound
routing), §3.3 builder lifecycle, RFC 3515 blind transfer.

## What it demonstrates

| Step | API |
|---|---|
| Agent registers | `coord.register(uri, user, pass).with_expires(s).send()` |
| Inbound call routes through CallHandler | `CallbackPeerBuilder::new(cfg).on_incoming(fn)` |
| Accept the inbound | `incoming.accept().await` |
| Blind transfer (RFC 3515 REFER) | `coord.refer(&call_id, target_uri).send()` |
| Customer follows the REFER | Handled internally by the customer's state machine — no extra application code |

## Run

```
cargo run --example call_center_agent
```

The example boots four coordinators in-process:
- mock registrar (raw UDP, embedded)
- customer (UAC, places call to the agent)
- agent (`CallbackPeer` via `CallbackPeerBuilder`, accepts then transfers)
- colleague (`CallbackPeer<AutoAccept>`, receives the transferred call)

## Pattern note: closing over the agent's own coord

The handler needs the agent's `UnifiedCoordinator` to dispatch the
REFER, but the coord doesn't exist until the peer is built. The example
uses an `Arc<OnceLock<Arc<UnifiedCoordinator>>>` so the closure can
read the coord lazily after `peer.coordinator().clone()` lands. This
pattern is reusable for any CallbackPeer that needs to issue further
outbound requests against its own coord.

## Attended transfer

Attended transfer (`coord.refer(&session, target).with_replaces(&hdr).send()`)
follows the same shape — the handler holds the original call on hold,
places a second consultation call, then issues a REFER with the
`Replaces:` header pointing at the consultation dialog. See
`SIP_API_DESIGN_2.md §11.2` for the full pattern.
