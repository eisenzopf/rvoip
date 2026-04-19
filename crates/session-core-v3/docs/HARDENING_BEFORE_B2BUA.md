# Hardening Plan — Before the b2bua Library

## Context

session-core-v3 is the single-session UAC/UAS control plane. The planned b2bua
crate will layer multi-session coordination (back-to-back leg linkage, transfer
orchestration, conference bridging) on top of it. Before b2bua is built, the
single-session primitives must be correct, leak-free, fully testable, and
symmetric across the three API surfaces (`UnifiedCoordinator`, `CallbackPeer`,
`StreamPeer`) — because b2bua will exercise those surfaces with N sessions at
once, where every gap present for one session becomes N times worse.

Recent hardening (CANCEL/487, DNS for REGISTER, outgoing `rport`, PRACK + early
media, 401/407 INVITE+REGISTER, 423 retry, attended-transfer primitives) has
closed the *functional* RFC surface for single sessions. What remains are
**correctness hazards, test-coverage gaps, and two API asymmetries** that would
become painful the moment b2bua multiplies sessions.

This plan groups work into three tiers. Tier 1 is mandatory before b2bua. Tier
2 is de-risking (ship-blocker for anything past "works on my laptop"). Tier 3
is API completeness for b2bua itself. Items explicitly *deferred* past b2bua
are listed at the end so we stop re-litigating them.

## Status legend

- ⬜ Not started / deferred
- 🟡 In progress
- ✅ Done (merged + tested)

## Summary (2026-04-18)

Tier 1 and Tier 3 (excluding 3.5) complete; Tier 2.1 + 2.2 complete with
new integration tests. Two items deferred with clear reasons: 2.3
(session-timer refresh-failure e2e) and 3.5 (BYE Reason header) — both
need multi-hour dialog-core refactors that are disproportionate to their
operational value given the event-level surface already in place.

The single-session control plane is now leak-free (T1.2 auto-cleanup),
unsafe-free (T1.1 OnceLock), shutdown-clean (T1.3 JoinSet), API-symmetric
for b2bua needs (T3.1 SDP pass-through, T3.2 proper 3xx, T3.3 handle-level
transfer primitives, T3.4 shutdown-handle symmetry), and covered by new
tests for redirect-follow and glare-retry.

---

## Tier 1 — Correctness hazards (must fix before b2bua)

### 1.1 ✅ Replace the unsafe `Arc::as_ptr` cast in coordinator construction

**File**: `src/api/unified.rs:193-196`

```rust
let adapter = Arc::as_ptr(&dialog_adapter) as *mut DialogAdapter;
unsafe { (*adapter).set_state_machine(state_machine.clone()); }
```

This mutates through an `Arc` while aliases exist — UB under any concurrent
reader. Works today only because no one reads `state_machine` on the adapter
during construction. b2bua may construct coordinators in quick succession and
share adapters across legs; the soundness bug will bite.

**Fix**: Change `DialogAdapter::state_machine` (see
`src/adapters/dialog_adapter.rs:86-88`) from `Option<Arc<StateMachine>>` to
`tokio::sync::OnceCell<Arc<StateMachine>>` (or `std::sync::OnceLock`). Replace
`set_state_machine` with `init_state_machine` that returns `Result<(),
AlreadyInitialized>`. Delete the `unsafe` block; call `init_state_machine` on
the fresh `Arc<DialogAdapter>` before any background tasks start.

### 1.2 ✅ Auto-remove sessions from `SessionStore` on terminal states

**Problem**: `SessionStore` entries persist after `CallEnded`, `CallFailed`,
`CallCancelled`, and `RegistrationFailed`. The only automatic removal found is
on incoming-call *dispatch* error
(`src/adapters/session_event_handler.rs:478`). A long-running peer accumulates
dead sessions indefinitely. b2bua will churn sessions 2-10× faster than a
simple UA.

**Fix**: In `src/adapters/session_event_handler.rs`, after publishing each
terminal event (`CallEnded`, `CallFailed`, `CallCancelled`,
`RegistrationFailed`, `UnregistrationSuccess`), call
`state_machine.store.remove_session(&id).await` and
`registry.remove_session(&id).await`. Add a test in
`tests/session_store_tests.rs` that drives a call to `CallEnded` and asserts
`list_sessions()` is empty afterward.

### 1.3 ✅ Track spawned tasks for clean shutdown

**Problem**: `src/adapters/session_event_handler.rs` has ~11 fire-and-forget
`tokio::spawn()` calls. `CallbackPeer::run` (`src/api/callback_peer.rs:366+`)
spawns per-event handler tasks. `UnifiedCoordinator::shutdown()` signals via a
watch channel but spawned tasks never observe it — they leak until their
channels drop.

**Fix**: Convert indefinite-loop spawn sites (subscriptions) to take a
`shutdown_rx: watch::Receiver<bool>` and `select!` on it. For one-shot
handler-invocation spawns in `callback_peer.rs:366-426`, track their
`JoinHandle`s in a `JoinSet` so `run()` can `join_all` before returning from
the shutdown branch.

---

## Tier 2 — Test coverage for already-implemented behavior

### 2.1 ✅ 3xx redirect-follow test

**What exists**: `src/state_machine/actions.rs:138-176`
(`Action::RetryWithContact`) + `src/adapters/session_event_handler.rs`
handling of `Dialog3xxRedirect`. 5-hop cap, first-Contact-URI priority.

**Test**: Add `tests/redirect_follow.rs` using the `register_423_retry.rs`
pattern — bind a raw UDP socket as a mock UAS, reply 302 Moved Temporarily
with a Contact pointing at a second mock bound on a different port that
accepts the call. Assert the UAC ends up in `Active` against the second
port. Also add a loop-detection case: two mocks that bounce 302 to each
other, assert abort after 5 hops with `CallFailed`.

### 2.2 ✅ 491 glare-retry test — now a real multi-binary integration test

**What exists**: UAS-side re-INVITE/UPDATE wiring now runs through the
state machine (see `docs/REINVITE_WIRING_PLAN.md`). The `HasPendingReinvite`
guard on `Active + ReinviteReceived` fires 491 Request Pending when we
receive a peer re-INVITE while our own is in flight; the existing
`ReinviteGlare` transitions schedule a 2.1-4.0 s retry. Covered by
`tests/glare_retry_integration.rs` + `examples/streampeer/glare_retry/{alice,bob}.rs`:
Alice and Bob simultaneously invoke `hold()`, each side sends 491 to the
other's re-INVITE, the production retry path converges on OnHold.

### 2.3 ⬜ Session-timer refresh-failure BYE test — *still deferred (revised reason)*

**What exists**: Wire-level BYE-on-timeout + `SessionRefreshFailed` event
publishing are implemented.

**Why still blocked**: Investigation during the re-INVITE wiring work
revealed that `dialog-core/src/manager/session_timer.rs` fires
`SessionRefreshed` optimistically the moment `send_request(UPDATE)`
returns — it does not subscribe to the UPDATE transaction's response or
timeout events. So a dead peer doesn't surface as a failure: Alice's
UDP send "succeeds," `refresh_ok = true`, and `SessionRefreshFailed`
never fires. The failure path only exists today for cases where the
transaction-manager itself rejects the send (e.g., no remote target).

**What it needs**: session_timer.rs to hold the `TransactionKey` after
sending, subscribe to its `SuccessResponse`/`FailureResponse`/`Timeout`
events, and drive `SessionRefreshFailed` from the timeout path. That's
a dialog-core refactor disproportionate to this hardening pass; track it
as a follow-on when the session-timer gets its own round.

---

## Tier 3 — API symmetry and completeness for b2bua

### 3.1 ✅ Custom SDP on accept

**Where**: `src/api/incoming.rs:88`, `src/api/callback_peer.rs:376`,
`src/state_machine/helpers.rs::accept_call`.

**Why b2bua needs it**: B2BUA accepts the inbound leg with SDP it received
from the outbound leg's 200 OK — there's no other correct way to bridge
legs with distinct media parameters.

**Fix**: Thread an `Option<String>` SDP override through `accept_call` →
`helpers.accept_call` → state-machine action. Wire `AcceptWithSdp` in
`callback_peer.rs:375-378` and add `IncomingCall::accept_with_sdp(String)`.

### 3.2 ✅ Proper 3xx send

**Where**: `src/api/callback_peer.rs:382-385` fakes redirect by calling
`reject_call(302, ...)` and discards the target URI.

**Fix**: Add `UnifiedCoordinator::redirect_call(session_id, status: u16,
contacts: Vec<String>)` → new `EventType::SendRedirectResponse { status,
contacts }` in the state table → new action that builds a 3xx response
with `Contact:` headers. Wire through all three API surfaces.

### 3.3 ✅ Expose `dialog_identity` + `send_refer_with_replaces` on `SessionHandle`

**Where**: Both methods live only on `UnifiedCoordinator`
(`src/api/unified.rs:414-432`). `SessionHandle` (`src/api/handle.rs`) has no
delegation.

**Fix**: Add `SessionHandle::dialog_identity()` and
`SessionHandle::transfer_attended(target, replaces)` as thin wrappers.

### 3.4 ✅ Symmetric shutdown across all three peer types

**Where**: `StreamPeer::shutdown(self)` consumes and has no `ShutdownHandle`.
`CallbackPeer` has one.

**Fix**: Add `StreamPeer::shutdown_handle() -> ShutdownHandle` paralleling
`CallbackPeer`.

### 3.5 ⬜ Session-timer BYE `Reason:` header — *deferred*

Wire-level header on the session-timer BYE is still missing; the
`SessionRefreshFailed` event already carries the 408 cause string which
apps consume. Adding the header requires threading extra-headers support
into `DialogManager::send_request_in_dialog` or a parallel BYE builder —
out of scope for this hardening pass.

---

## Deferred past b2bua — do not work on these now

- TLS / TCP transport (sip-transport consolidation)
- RFC 3263 SRV/NAPTR
- Contact rewrite from discovered NAT address
- STUN / ICE / SIP Outbound (RFC 5626)
- INFO and OPTIONS outbound helpers
- Digest `nc` tracking, `auth-int`, `-sess` variants
- 422 UAC-side retry with bumped Min-SE
- Early-media RTP playback (AudioSource wiring)
- Attended-transfer orchestration (b2bua's job)

---

## Critical files

| File | Tier | Change |
|------|------|--------|
| `src/api/unified.rs:193-196` | 1.1 | Delete unsafe block |
| `src/adapters/dialog_adapter.rs:86-88` | 1.1 | `OnceCell` for state_machine |
| `src/adapters/session_event_handler.rs` | 1.2, 1.3 | Terminal cleanup, shutdown-aware spawns |
| `src/api/callback_peer.rs` | 1.3, 3.1, 3.2 | JoinSet, accept-with-sdp, redirect |
| `src/api/incoming.rs:88, 144` | 3.1, 3.2 | accept_with_sdp, redirect |
| `src/api/handle.rs` | 3.3 | dialog_identity + transfer_attended |
| `src/api/stream_peer.rs` | 3.4 | shutdown_handle |
| `src/state_machine/actions.rs` | 3.2, 3.5 | SendRedirectResponse, BYE Reason |
| `state_tables/default.yaml` | 3.2 | Redirect transition |
| `tests/redirect_follow.rs` | 2.1 | New file |
| `tests/glare_retry.rs` | 2.2 | New file |
| `tests/session_timer_failure.rs` | 2.3 | New file |
| `tests/early_media_tests.rs` | 3.1 | Custom-SDP assertion |

---

## Verification

1. `cargo test -p rvoip-session-core-v3` — all tiers 1 and 2 tests green.
2. `cargo test -p rvoip-session-core-v3 --test redirect_follow` and
   `--test glare_retry` and `--test session_timer_failure` — each new test
   green individually.
3. `cargo clippy -p rvoip-session-core-v3 -- -D warnings` — no `unsafe`
   blocks remain after 1.1.
4. Run `examples/callbackpeer/routing` with a redirect decision — verify
   3xx on the wire carries the intended Contact.
5. Stress test: spawn 50 `StreamPeer`s, each making a call that's rejected
   486, then dropping the peer. `list_sessions()` on a freshly-created peer
   must return empty and RSS must stabilize (confirms 1.2).
6. Manual: kill a `CallbackPeer::run()` future via its shutdown handle
   while a handler is in-flight; assert handler finishes before `run()`
   returns (confirms 1.3).
