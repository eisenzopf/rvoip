# CANCEL Path — Fix Plan

Two independent bugs surfaced while writing the CANCEL integration test
for v0.2. Each has a small, self-contained fix. This plan describes
both; they can land in either order.

## Bug A — UAS-side CANCEL handler rejects its own server transaction

### Where

`crates/dialog-core/src/manager/protocol_handlers.rs::handle_cancel_method`
→ `cancel_invite_transaction_with_dialog`
→ `crates/dialog-core/src/manager/transaction_integration.rs::cancel_invite_transaction_with_dialog`
→ `crates/dialog-core/src/transaction/manager/mod.rs::cancel_invite_transaction`

The last function explicitly rejects server transactions:

```rust
if invite_tx_id.method() != &Method::Invite || invite_tx_id.is_server() {
    return Err(Error::Other(format!(
        "Transaction {} is not an INVITE client transaction", invite_tx_id
    )));
}
```

That function is *correctly* named — it cancels a *client-side*
outgoing INVITE (i.e., sends a CANCEL request). The server-side
responsibilities on receiving CANCEL are entirely different:

1. Respond 200 OK on the CANCEL transaction.
2. Respond 487 Request Terminated on the pending INVITE server
   transaction (if still in the Proceeding state).
3. Terminate the dialog and notify the upper layers.

None of that happens today.

### Fix

Rewrite `handle_cancel_method` to do the UAS work directly instead of
delegating to the client-side helper. Sketch:

```rust
async fn handle_cancel_method(&self, request: Request) -> DialogResult<()> {
    // 1. Create server transaction for the CANCEL itself.
    let source = SourceExtractor::extract_from_request(&request);
    let cancel_tx = self.transaction_manager
        .create_server_transaction(request.clone(), source).await?;
    let cancel_tx_id = cancel_tx.id().clone();

    // 2. Find the matching INVITE server transaction by branch.
    let invite_tx_id = self.transaction_manager
        .find_invite_transaction_for_cancel(&request).await?;

    match invite_tx_id {
        Some(invite_tx_id) if invite_tx_id.is_server() => {
            // 3a. 200 OK to CANCEL — RFC 3261 §9.2.
            let ok = response_builders::create_response(&request, StatusCode::Ok);
            self.transaction_manager.send_response(&cancel_tx_id, ok).await?;

            // 3b. 487 Request Terminated to the pending INVITE.
            let original_invite = self.transaction_manager
                .get_server_transaction_request(&invite_tx_id).await?;
            let terminated = response_builders::create_response(
                &original_invite,
                StatusCode::RequestTerminated,
            );
            self.transaction_manager.send_response(&invite_tx_id, terminated).await?;

            // 3c. Terminate dialog + publish session event (existing code).
            self.terminate_dialog_for_tx(&invite_tx_id, "CANCEL received").await?;
        }
        _ => {
            // 481 — unchanged.
            let not_found = response_builders::create_response(
                &request, StatusCode::CallOrTransactionDoesNotExist);
            self.transaction_manager.send_response(&cancel_tx_id, not_found).await?;
        }
    }
    Ok(())
}
```

Dependencies:
- `transaction_manager.get_server_transaction_request(tx_id)` — may need
  to be added if not present. The equivalent for client transactions is
  `utils::get_transaction_request(&self.client_transactions, tx_id)`;
  mirror against `server_transactions`.
- The existing `terminate_dialog_for_tx` logic lives inside
  `cancel_invite_transaction_with_dialog` today — extract it into a
  helper so both the client-CANCEL and server-CANCEL paths share it.

### Test

The ignored `tests/cancel_integration.rs::cancel_emits_callcancelled_event`
becomes the forcing function. Remove the `#[ignore]`, run, expect
Alice's `handle.hangup()` to produce `Event::CallCancelled` once Bug B
below is also fixed.

Stand-alone unit coverage in dialog-core: a test that pushes a raw
CANCEL datagram at a `UnifiedDialogApi` with an active incoming INVITE
and asserts that (a) 200 OK for CANCEL and (b) 487 for INVITE appear
on the outbound socket. This is buildable with the existing
`transaction::manager::tests` fixtures.

### Risk

Low. The change is additive — non-CANCEL paths untouched. The only
shared surface is the `terminate_dialog_for_tx` extraction, and it
retains today's semantics (dialog terminate + session event).

---

## Bug B — No public API path from `hangup()` to CANCEL

### Where

`crates/session-core-v3/state_tables/default.yaml` — `CancelCall`
(internal event) has transitions from `UAC/Initiating` and
`UAC/Ringing` that run `SendCANCEL + CleanupDialog + CleanupMedia`.
`HangupCall` (what `SessionHandle::hangup()` fires) has transitions
only from `Active` and `OnHold` / `Answering`.

A draft that added `UAC/Initiating/HangupCall → SendCANCEL` broke
`tests/unified_api_tests.rs::test_multiple_calls`. Root cause: RFC 3261
§9.1 says CANCEL *should not* be sent before at least a provisional
response has been received. In `test_multiple_calls`, no UAS is
listening, so the UAC is still in Initiating with no 1xx — sending
CANCEL in that state both (a) violates the RFC and (b) errors out
because dialog-core's `send_cancel` can't complete.

### Fix

Two state-table entries with the RFC-correct split:

```yaml
# Before a provisional response has arrived, there's nothing to
# CANCEL on the wire yet. Tear down session state locally; Timer B
# (INVITE transaction timeout) will eventually fire in dialog-core
# if the remote was reachable, at which point a stray 2xx would be
# ignored because the dialog is gone.
- role: "UAC"
  state: "Initiating"
  event:
    type: "HangupCall"
  next_state: "Terminated"
  actions:
    - type: "CleanupDialog"
    - type: "CleanupMedia"
  publish:
    - "CallEnded"
  description: "Hangup before any provisional — local teardown only"

# Once ringing (180 or reliable 18x received), CANCEL is the right
# wire message per RFC 3261 §9.1.
- role: "UAC"
  state: "Ringing"
  event:
    type: "HangupCall"
  next_state: "Cancelled"
  actions:
    - type: "SendCANCEL"
    - type: "CleanupDialog"
    - type: "CleanupMedia"
  publish:
    - "CallCancelled"
  description: "Hangup while ringing — sends CANCEL"
```

Additionally add the same pair for `EarlyMedia` state (reliable 183
path) — also wire-eligible for CANCEL since a provisional has been
received.

No public API change. `SessionHandle::hangup()` now:
- Initiating → local teardown, no wire message.
- Ringing / EarlyMedia → CANCEL + 487 (once Bug A is fixed).
- Active / OnHold / Answering → BYE (unchanged).

### Alternative considered

Adding a distinct `SessionHandle::cancel()` API. Rejected: forces
callers to reason about SIP state. "Stop this call" is one user intent
— the state machine is the right place to dispatch.

### Test

- `tests/unified_api_tests.rs::test_multiple_calls` — continues to
  pass (Initiating path now runs only cleanup, no SendCANCEL).
- `tests/cancel_integration.rs::cancel_emits_callcancelled_event` —
  unignored once both bugs are fixed.

### Risk

Low. New transitions only; existing Active/OnHold hangup paths
untouched. The Initiating "local teardown only" branch has precedent
in dialog-core's existing `DialogTimeout` path (which also tears down
locally without a wire CANCEL).

---

## Files to change

| Bug | File | Change |
|-----|------|--------|
| A | `crates/dialog-core/src/manager/protocol_handlers.rs` | Rewrite `handle_cancel_method` to do UAS work directly |
| A | `crates/dialog-core/src/manager/transaction_integration.rs` | Extract shared `terminate_dialog_for_tx` helper |
| A | `crates/dialog-core/src/transaction/manager/mod.rs` | Add `get_server_transaction_request` if missing |
| B | `crates/session-core-v3/state_tables/default.yaml` | Add three HangupCall transitions (Initiating, Ringing, EarlyMedia) |
| A+B | `crates/session-core-v3/tests/cancel_integration.rs` | Remove `#[ignore]`; verify end-to-end |
| A+B | `crates/session-core-v3/docs/RFC_COMPLIANCE_STATUS.md` | Mark the 487 row ✅ |

## Verification

1. `cargo test -p rvoip-dialog-core` — 317 tests stay green; new UAS
   CANCEL unit test passes.
2. `cargo test -p rvoip-session-core-v3` — 168 tests stay green;
   `test_multiple_calls` still passes; `cancel_emits_callcancelled_event`
   (was ignored) now passes.
3. Manual wire check with a real UAS (Asterisk / FreeSWITCH) or with
   `sipp`: place a call, hang up during Ringing, confirm 200 OK on
   CANCEL and 487 on INVITE appear on the wire.

## Estimated effort

- Bug A: ~2-3 hours including test.
- Bug B: ~30 minutes.
- Bundled PR so the integration test flips `#[ignore]` → passing in one commit.
