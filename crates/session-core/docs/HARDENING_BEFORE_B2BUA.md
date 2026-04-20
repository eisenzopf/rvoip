# Hardening Plan — Before the b2bua Library

## Context

session-core is the single-session UAC/UAS control plane. The planned b2bua
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

## Summary (2026-04-19)

All Tier 1, Tier 2, and Tier 3 items complete. The last two deferred
items (2.3 session-timer refresh-failure e2e and 3.5 BYE Reason header)
closed together in one pass along with a pre-existing dialog-core bug
exposed by the new await-the-response code path (UAS mid-dialog response
routing to the wrong server transaction when both an INVITE-server and a
later UPDATE-server were live for the same dialog).

The single-session control plane is now leak-free (T1.2 auto-cleanup),
unsafe-free (T1.1 OnceLock), shutdown-clean (T1.3 JoinSet), API-symmetric
for b2bua needs (T3.1 SDP pass-through, T3.2 proper 3xx, T3.3 handle-level
transfer primitives, T3.4 shutdown-handle symmetry), covered by new
multi-binary integration tests for redirect-follow, glare-retry, and
session-timer refresh-failure, and RFC 4028 §10 compliant with a proper
`Reason: SIP ;cause=408` header on the refresh-failure BYE.

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

### 2.3 ✅ Session-timer refresh-failure BYE test

`dialog-core/src/manager/session_timer.rs` was rewritten to subscribe to
the refresh transaction's outcome via
`TransactionManager::subscribe_to_transaction` (plus a `last_response`
peek to handle the race where the peer answers before we subscribe).
`SessionRefreshed` now fires only on a 2xx; anything else (4xx/5xx/6xx,
timeout, transport error) falls through to the RFC 4028 §9 re-INVITE
fallback, and if that also fails the dialog is torn down with a BYE
carrying `Reason: SIP ;cause=408 ;text="Session expired"` (item 3.5).
Covered by `tests/session_timer_failure_integration.rs` +
`examples/streampeer/session_timer_failure/{alice,bob}.rs`: Bob accepts
the call then exits at t≈1.5 s; Alice observes `SessionRefreshFailed`
within 15 s with the UPDATE-timed-out + re-INVITE-transport-error cause
string.

The rewrite also fixed a pre-existing dialog-core bug exposed by the new
await path: UAS mid-dialog responses (`send_response_for_session`) were
picking an arbitrary server transaction for the dialog instead of the
one currently awaiting a response, so UPDATE 200 OKs were being built
with the INVITE's Via/branch — invisible under the old optimistic
behaviour, hard failure under the new one. Fixed by filtering candidate
transactions on `is_server()` + open state (Initial/Trying/Proceeding)
and preferring non-INVITE when both are live, and by associating the
UAS UPDATE transaction with its dialog in `process_update_in_dialog`
(the re-INVITE path already did this).

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

### 3.5 ✅ Session-timer BYE `Reason:` header

`request_builder_from_dialog_template` and `bye_for_dialog` now take an
`extra_headers: Option<Vec<TypedHeader>>` parameter. A new
`DialogManager::send_bye_with_reason(dialog_id, Reason)` method is the
high-level entry point; `session_timer.rs` calls it on refresh failure
with `Reason::new("SIP", 408, Some("Session expired"))` per RFC 4028 §10.
Wire coverage: `tests/session_timer_failure_integration.rs` drives the
full path; apps additionally still see the cause surfaced via the
`SessionRefreshFailed` event string.

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
| `tests/glare_retry_integration.rs` | 2.2 | New file |
| `tests/session_timer_failure_integration.rs` | 2.3 | New file |
| `examples/streampeer/session_timer_failure/{alice,bob}.rs` | 2.3 | New peer binaries |
| `dialog-core/src/manager/session_timer.rs` | 2.3 | Await transaction outcomes; tear down with 408 Reason |
| `dialog-core/src/protocol/update_handler.rs` | 2.3 | Associate UPDATE server-tx with dialog |
| `dialog-core/src/api/unified.rs` | 2.3 | `send_response_for_session` picks pending server-tx |
| `dialog-core/src/manager/core.rs` | 3.5 | `send_bye_with_reason` |
| `dialog-core/src/transaction/dialog/{mod,quick}.rs` | 3.5 | `extra_headers` plumbing |
| `tests/early_media_tests.rs` | 3.1 | Custom-SDP assertion |

---

## Verification

1. `cargo test -p rvoip-session-core` — all tiers 1 and 2 tests green.
2. `cargo test -p rvoip-session-core --test redirect_follow` and
   `--test glare_retry` and `--test session_timer_failure` — each new test
   green individually.
3. `cargo clippy -p rvoip-session-core -- -D warnings` — no `unsafe`
   blocks remain after 1.1.
4. Run `examples/callbackpeer/routing` with a redirect decision — verify
   3xx on the wire carries the intended Contact.
5. Stress test: spawn 50 `StreamPeer`s, each making a call that's rejected
   486, then dropping the peer. `list_sessions()` on a freshly-created peer
   must return empty and RSS must stabilize (confirms 1.2).
6. Manual: kill a `CallbackPeer::run()` future via its shutdown handle
   while a handler is in-flight; assert handler finishes before `run()`
   returns (confirms 1.3).
