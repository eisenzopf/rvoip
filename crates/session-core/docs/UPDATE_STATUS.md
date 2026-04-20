# UPDATE Method (RFC 3311) Status

## Summary

`dialog-core` fully implements the SIP UPDATE method — both send and receive paths.
`session-core` does **not** currently use UPDATE anywhere in its public API.

## Why this doc exists

During `streampeer_hold_resume` testing we observed dialog-core warnings:
```
WARN rvoip_dialog_core::transaction::client::non_invite:
  Timer F (Timeout) fired in state Trying ... UPDATE:client
```

Root cause: `DialogAdapter::send_reinvite_session()` was calling
`UnifiedDialogApi::send_update()` instead of sending a proper re-INVITE. The UAS
on the other side either didn't answer the UPDATE in time or couldn't match it
to its existing dialog, and Timer F (32 s) fired.

**Fixed in** `src/adapters/dialog_adapter.rs::send_reinvite_session()`: it now
calls `send_request_in_dialog(dialog_id, Method::Invite, body)`, i.e. an actual
re-INVITE per RFC 3261 §14. This is also the RFC-recommended method for
mid-dialog SDP changes (hold, resume, codec renegotiation) — UPDATE is primarily
useful for session timer refreshes and early-dialog SDP updates before 200 OK.

## Current coverage

| Direction | Status |
|-----------|--------|
| Send UPDATE (UAC / UAS mid-dialog) | **Implemented** in `dialog-core/src/transaction/method/update.rs` — not currently invoked from session-core |
| Receive UPDATE | **Implemented** in `dialog-core/src/protocol/update_handler.rs` — emits a session coordination event (behaviour untested from v3) |
| Session-Expires / Min-SE refresh via UPDATE (RFC 4028) | **Not implemented** — requires session timer work |
| Early-dialog SDP update via UPDATE (RFC 3311 §5.1) | **Not implemented** — early media codec changes go through re-INVITE today |

## When UPDATE would be needed

1. **RFC 4028 session timers** — periodic dialog refresh without disturbing media. Roadmap item in the Phase C plan.
2. **Early-dialog media changes** — a UAS that wants to change codec/direction during 180 Ringing with early media; re-INVITE isn't allowed in the early dialog, so UPDATE is the only option.
3. **Hold/resume over strict proxies** — a few legacy SBCs preferentially route UPDATE for SDP changes. We can add an opt-in Config flag later if that becomes an issue in the field.

## Recommendation

Keep dialog-core's UPDATE support (it's complete and tested at the transaction
layer) but do not route any session-core public API through it until session
timers are implemented. The fix applied today removes the Timer F noise.
