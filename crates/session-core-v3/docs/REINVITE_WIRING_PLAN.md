# Re-INVITE / UPDATE event wiring + UAS-side glare — production plan

## Status

⬜ Proposed — not started

## Context

While building integration tests for T2.2 (491 glare retry) and T2.3
(session-timer refresh failure), investigation surfaced a real production
gap: **dialog-core's `SessionCoordinationEvent::ReInvite` never reaches
session-core-v3**.

Concretely:

- `invite_handler.rs:215` and `update_handler.rs:95` both emit
  `SessionCoordinationEvent::ReInvite`.
- That event travels through
  `DialogManager::emit_session_coordination_event` →
  `EventHub::publish_session_coordination_event` →
  `convert_coordination_to_cross_crate`.
- `convert_coordination_to_cross_crate` has 11 match arms but **no arm for
  `ReInvite`**, so it returns `None` and nothing is published to the
  `GlobalEventCoordinator`'s `dialog_to_session` channel.
- Session-core-v3's `session_event_handler.rs::handle_reinvite_received`
  exists but is **never invoked** — `event_str.contains("ReinviteReceived")`
  is always false because the event is never emitted.
- The state table has **no transition for `EventType::ReinviteReceived`** at
  all.

The hold/resume happy path works today only because the **UAC** side
(`Action::SendReINVITE` → wait for `Dialog200OK`) is fully wired. The **UAS**
side receiving a re-INVITE has no observable behavior in session-core-v3 —
the dialog stays in its current state and dialog-core's transaction layer
eventually times the server transaction out, unless some path I haven't
traced is auto-responding at a lower layer.

This is directly in the path of b2bua, which does re-INVITE flows (hold,
codec renegotiation, media retarget) all the time. It also blocks a
wire-level test for RFC 3261 §14.1 glare retry (T2.2) and session-timer
refresh failure (T2.3).

## Goals

1. **Close the production gap.** UAS-side re-INVITE and UPDATE receive real
   state-machine-driven responses via the existing YAML + global-event-bus
   patterns — no direct-send shortcuts in the event handler.
2. **Add RFC 3261 §14.1 UAS-side glare detection.** When we receive a
   re-INVITE while our own re-INVITE is still pending, automatically
   respond 491 Request Pending. No test hooks required; real glare is
   reproducible by two peers both initiating hold simultaneously.
3. **Make T2.2 and T2.3 real integration tests** using the same
   two-subprocess pattern as `cancel_integration.rs` /
   `session_timer_integration.rs`. Test hooks become minimal and only
   exist for the UPDATE-drop branch where no legitimate production
   analog exists.

## Non-goals

- Attended-transfer orchestration (multi-session linkage) — b2bua layer.
- Full 3xx-as-response-to-reINVITE plumbing.
- Rewriting `SessionCoordinationEvent` — only a small additive change.

---

## Plan

### Phase 1 — Production wiring (dialog-core + session-core-v3)

#### P1.1 Add `method` field to `SessionCoordinationEvent::ReInvite`

**File**: `crates/dialog-core/src/events/session_coordination.rs`

Today:
```rust
ReInvite {
    dialog_id: DialogId,
    transaction_id: TransactionKey,
    request: Request,
}
```

Change to:
```rust
ReInvite {
    dialog_id: DialogId,
    transaction_id: TransactionKey,
    request: Request,
    /// The method that triggered this coordination event. UPDATE and
    /// re-INVITE both flow through this variant today; the session
    /// layer needs to distinguish them for session-timer refresh and
    /// media renegotiation handling.
    method: Method,
}
```

**Callsite updates** (three total):

- `crates/dialog-core/src/protocol/invite_handler.rs:215` — set `method: Method::Invite`.
- `crates/dialog-core/src/protocol/update_handler.rs:95` — set `method: Method::Update`.
- `crates/dialog-core/src/manager/protocol_handlers.rs:320` and `:530` — set whichever method the request carries.

`request.method()` is free at each site; no extra plumbing.

#### P1.2 Convert `ReInvite` to cross-crate `ReinviteReceived`

**File**: `crates/dialog-core/src/events/event_hub.rs` in
`convert_coordination_to_cross_crate` (around line 156).

Add:
```rust
SessionCoordinationEvent::ReInvite { dialog_id, request, method, .. } => {
    let session_id = self
        .dialog_to_session_id
        .get(&dialog_id)
        .map(|r| r.clone())
        .unwrap_or_else(|| dialog_id.to_string());
    let sdp = if !request.body().is_empty() {
        Some(String::from_utf8_lossy(request.body()).to_string())
    } else {
        None
    };
    Some(RvoipCrossCrateEvent::DialogToSession(
        DialogToSessionEvent::ReinviteReceived {
            session_id,
            sdp,
            method: method.to_string(), // "INVITE" or "UPDATE"
        },
    ))
}
```

The `method` string field is a new addition to
`DialogToSessionEvent::ReinviteReceived` in
`crates/infra-common/src/events/cross_crate.rs:421`. Additive change,
no breakage.

#### P1.3 Extend session-core-v3's `EventType::ReinviteReceived` with method

**File**: `crates/session-core-v3/src/state_table/types.rs:181`

Today:
```rust
ReinviteReceived { sdp: Option<String> },
```

Change to:
```rust
ReinviteReceived { sdp: Option<String>, method: ReinviteMethod },
```

Where `ReinviteMethod` is a new enum:
```rust
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum ReinviteMethod {
    Invite,
    Update,
}
```

Update `EventType::normalize()` to strip the `sdp` field and keep
`method` — the method matters for state-table lookup (UPDATE may have
different response rules than re-INVITE).

Actually: to keep the state-table lookup simple, split into two events:

```rust
ReinviteReceived { sdp: Option<String> },   // re-INVITE
UpdateReceived   { sdp: Option<String> },   // UPDATE (RFC 3311)
```

Two separate `EventType` variants, two separate YAML transitions. This
matches the dedicated-variant pattern already used for `Dialog180Ringing`
vs `Dialog183SessionProgress`, etc.

#### P1.4 Dispatch the new events from `session_event_handler`

**File**: `crates/session-core-v3/src/adapters/session_event_handler.rs`

At the dispatch matcher (around line 278), add:
```rust
} else if event_str.contains("ReinviteReceived") {
    self.handle_reinvite_or_update_received(&event_str).await?;
}
```

Rework `handle_reinvite_received` (line 1062) into
`handle_reinvite_or_update_received` that parses the `method` field and
dispatches `EventType::ReinviteReceived { sdp }` or
`EventType::UpdateReceived { sdp }` accordingly. Uses the same
debug-format parsing pattern as the other handlers.

#### P1.5 YAML transitions for UAS-side responses

**File**: `crates/session-core-v3/state_tables/default.yaml`

Add four transitions (Active + OnHold, one per method):

```yaml
# RFC 3261 §14 — UAS receives re-INVITE on an established dialog, no
# pending outgoing re-INVITE. Respond 200 OK and stay in Active.
- role: "Both"
  state: "Active"
  event:
    type: "ReinviteReceived"
  next_state: "Active"
  actions:
    - type: "NegotiateSDPAsUAS"
    - type: "SendSIPResponse"
      code: 200
      reason: "OK"
  description: "UAS answers incoming re-INVITE (hold/resume/renegotiation)"

- role: "Both"
  state: "OnHold"
  event:
    type: "ReinviteReceived"
  next_state: "OnHold"
  actions:
    - type: "NegotiateSDPAsUAS"
    - type: "SendSIPResponse"
      code: 200
      reason: "OK"
  description: "UAS answers re-INVITE while already on hold (resume from peer)"

# RFC 3311 — UPDATE for mid-dialog modification / session-timer refresh.
- role: "Both"
  state: "Active"
  event:
    type: "UpdateReceived"
  next_state: "Active"
  actions:
    - type: "SendSIPResponse"
      code: 200
      reason: "OK"
  description: "UAS answers UPDATE (session-timer refresh)"

- role: "Both"
  state: "OnHold"
  event:
    type: "UpdateReceived"
  next_state: "OnHold"
  actions:
    - type: "SendSIPResponse"
      code: 200
      reason: "OK"
  description: "UAS answers UPDATE while on hold"
```

UPDATE transitions don't include `NegotiateSDPAsUAS`: UPDATE for
session-timer refresh carries no SDP (per RFC 4028 §9). If a UPDATE
body is present for session modification, that's a future enhancement.

#### P1.6 Wire `NegotiateSDPAsUAS` to use the incoming re-INVITE's SDP

The action already exists. Confirm it picks up `session.remote_sdp` which
is set by the executor when `ReinviteReceived { sdp: Some(_) }` is
processed. If not, add an executor arm:

```rust
EventType::ReinviteReceived { sdp } => {
    if let Some(sdp_data) = sdp {
        session.remote_sdp = Some(sdp_data.clone());
        session.sdp_negotiated = false; // force renegotiation
    }
}
```

### Phase 2 — UAS-side glare detection (RFC 3261 §14.1)

#### P2.1 Add `HasPendingReinvite` guard

**File**: `crates/session-core-v3/src/state_table/types.rs` (Guard enum)
and `src/state_machine/guards.rs`.

```rust
// types.rs
Guard::HasPendingReinvite,

// guards.rs
Guard::HasPendingReinvite => session.pending_reinvite.is_some(),
```

#### P2.2 Add `SendSIPResponse(491)` glare transition

```yaml
# RFC 3261 §14.1 — UAS-side glare: incoming re-INVITE while we have
# our own re-INVITE in flight. Respond 491 Request Pending so the peer
# backs off and retries. Put this transition BEFORE the normal 200 OK
# transition in the YAML ordering, since the state table evaluates
# guards in order.
- role: "Both"
  state: "Active"
  event:
    type: "ReinviteReceived"
  guards:
    - "HasPendingReinvite"
  next_state: "Active"
  actions:
    - type: "SendSIPResponse"
      code: 491
      reason: "Request Pending"
  description: "UAS-side glare: reject incoming re-INVITE while our own is pending"

- role: "Both"
  state: "Resuming"
  event:
    type: "ReinviteReceived"
  next_state: "Resuming"
  actions:
    - type: "SendSIPResponse"
      code: 491
      reason: "Request Pending"
  description: "UAS-side glare during Resuming (always — we're mid re-INVITE)"
```

The `Resuming` transition doesn't need the guard because being in
`Resuming` state implies we have a re-INVITE in flight.

#### P2.3 Confirm guard evaluation order in the YAML loader

Verify the YAML loader honors guard ordering — the
`HasPendingReinvite`-guarded transition must be checked before the
unguarded one. Current state-table code (`yaml_loader.rs`) needs
inspection to confirm it returns the first matching transition.
If it doesn't, use distinct `role` or rely on guard-check semantics.

### Phase 3 — Integration tests (real wire flows)

#### P3.1 T2.2 — Real 491 glare retry test

**Files**:
- `crates/session-core-v3/examples/streampeer/glare_retry/alice.rs`
- `crates/session-core-v3/examples/streampeer/glare_retry/bob.rs`
- `crates/session-core-v3/tests/glare_retry_integration.rs`

**Alice**:
1. Calls Bob, waits for Active.
2. Subscribes to events.
3. At a synchronized moment (sleep-to-timestamp or environment-var
   "start time"), calls `handle.hold()`.
4. Expects glare: either sees her own 491 → `ReinviteGlare` →
   `ScheduleReinviteRetry` → re-attempt → `CallOnHold`, or she wins
   the race and Bob retries.
5. Asserts final state is `CallOnHold` (either side's retry succeeded
   and both are on hold).
6. Exits 0 on success.

**Bob**: mirrors Alice. Both call `hold()` at the synchronized moment;
one will send 491 to the other's re-INVITE purely because of the
`HasPendingReinvite` guard. RFC-compliant, no test hooks.

**Timing**: the simultaneous hold may not reliably produce glare every
run. Mitigation: Alice sleeps until `start_ms` from env, Bob the same —
sub-10ms synchronization. If flaky, add env-var `RVOIP_TEST_FORCE_GLARE`
that delays Bob's re-INVITE response by ~20ms to guarantee glare, but
*the 491 response itself still comes from the YAML guard path*, not a
test bypass.

**Replaces**: the previous `tests/glare_retry.rs` state-table unit test.

#### P3.2 T2.3 — Real session-timer refresh failure test

**Files**:
- `crates/session-core-v3/examples/streampeer/session_timer_failure/alice.rs`
- `crates/session-core-v3/examples/streampeer/session_timer_failure/bob.rs`
- `crates/session-core-v3/tests/session_timer_failure_integration.rs`

**Alice**: UAC with `Config.session_timer_secs = Some(4)`,
`session_timer_min_se = 2`. Accepts call, subscribes to events, asserts
she sees `Event::SessionRefreshFailed` within 15s.

**Bob**: UAS. Needs to make Alice's UPDATE refresh fail. Production
analog: UAS exits or crashes mid-call. Do exactly that — Bob accepts the
call, sleeps 3 seconds (past call establishment, before refresh), then
exits its process. Alice's UPDATE lands on a dead port, transaction
layer reports the failure (ICMP unreachable or transaction timeout),
session_timer.rs falls through to re-INVITE attempt, also fails, BYE is
sent, `SessionRefreshFailed` fires.

**No test hooks required**. This matches real-world "remote crashed"
scenarios. The only concern is timer F (32s) for non-invite transaction
timeout — verify ICMP port unreachable shortens this to an immediate
send error. If not, add `RVOIP_TEST_TRANSACTION_TIMEOUT_MS=1000` env
var read by transaction-core (also useful for other tests).

### Phase 4 — Docs + cleanup

#### P4.1 Update `RFC_COMPLIANCE_STATUS.md`

Flip 491 glare retry and session-timer refresh failure rows to ✅ with
test references, and add a new row:

```
| UAS-side re-INVITE/UPDATE response | ✅ | State machine answers 200 OK via ReinviteReceived/UpdateReceived YAML transitions (Phase 1). UAS-side glare auto-detection per RFC 3261 §14.1 via HasPendingReinvite guard (Phase 2). |
```

#### P4.2 Update `HARDENING_BEFORE_B2BUA.md`

Flip T2.2 and T2.3 from ⬜ to ✅ with test names. Remove the "deferred"
text.

#### P4.3 Delete stub `tests/glare_retry.rs`

The state-table unit test is superseded by the integration test.

---

## Files touched (summary)

| File | Phase | Change |
|------|-------|--------|
| `crates/dialog-core/src/events/session_coordination.rs` | 1.1 | Add `method` field to `ReInvite` |
| `crates/dialog-core/src/protocol/invite_handler.rs:215` | 1.1 | Set `method: Method::Invite` |
| `crates/dialog-core/src/protocol/update_handler.rs:95` | 1.1 | Set `method: Method::Update` |
| `crates/dialog-core/src/manager/protocol_handlers.rs:320,530` | 1.1 | Set method from request |
| `crates/dialog-core/src/events/event_hub.rs` | 1.2 | Add `ReInvite → ReinviteReceived` arm |
| `crates/infra-common/src/events/cross_crate.rs:421` | 1.2 | Add `method: String` to ReinviteReceived |
| `crates/session-core-v3/src/state_table/types.rs` | 1.3, 2.1 | Split into `ReinviteReceived`/`UpdateReceived`; add `HasPendingReinvite` guard |
| `crates/session-core-v3/src/state_machine/guards.rs` | 2.1 | Implement `HasPendingReinvite` match arm |
| `crates/session-core-v3/src/state_machine/executor.rs` | 1.6 | Wire SDP from incoming re-INVITE into `session.remote_sdp` |
| `crates/session-core-v3/src/adapters/session_event_handler.rs` | 1.4 | Split handler by method, dispatch correct `EventType` |
| `crates/session-core-v3/src/state_table/yaml_loader.rs` | 1.3 | Parse `ReinviteReceived`, `UpdateReceived` event names |
| `crates/session-core-v3/state_tables/default.yaml` | 1.5, 2.2 | 4 new transitions (normal) + 2 glare transitions |
| `crates/session-core-v3/examples/streampeer/glare_retry/{alice,bob}.rs` | 3.1 | New |
| `crates/session-core-v3/examples/streampeer/session_timer_failure/{alice,bob}.rs` | 3.2 | New |
| `crates/session-core-v3/tests/glare_retry_integration.rs` | 3.1 | New |
| `crates/session-core-v3/tests/session_timer_failure_integration.rs` | 3.2 | New |
| `crates/session-core-v3/tests/glare_retry.rs` | 4.3 | Delete (superseded) |
| `crates/session-core-v3/Cargo.toml` | 3.1, 3.2 | Register 4 new example binaries |
| `crates/session-core-v3/docs/RFC_COMPLIANCE_STATUS.md` | 4.1 | Flip rows, add UAS re-INVITE row |
| `crates/session-core-v3/docs/HARDENING_BEFORE_B2BUA.md` | 4.2 | Mark T2.2, T2.3 done |

---

## Verification

1. `cargo check -p rvoip-dialog-core` — event additions compile.
2. `cargo check -p rvoip-session-core-v3` — state table loader, event
   dispatch, YAML transitions all compile and load.
3. `cargo test -p rvoip-session-core-v3 --tests -- --test-threads=1` — all
   existing tests still pass, especially `hold_resume` (which now drives
   real state-machine re-INVITE responses).
4. `cargo test -p rvoip-session-core-v3 --test glare_retry_integration`
   — passes in under 20s; Alice observes either her own retry or Bob's
   success, lands in `CallOnHold`.
5. `cargo test -p rvoip-session-core-v3 --test session_timer_failure_integration`
   — passes in under 25s; Alice observes `SessionRefreshFailed` after Bob
   crashes.
6. Manual: run the `hold_resume` example with `RUST_LOG=info`, confirm
   the UAS side's log shows `EventType::ReinviteReceived` being dispatched
   and `Action::SendSIPResponse(200, OK)` firing (proves the
   production path is live).
7. Manual: `cargo run --example streampeer_glare_retry_alice &` +
   `cargo run --example streampeer_glare_retry_bob` — watch tcpdump for
   a 491 on the wire, followed by a retry INVITE, followed by 200 OK.

---

## Risk + open questions

- **Simultaneous hold timing in P3.1** — may need a small test-only hint
  (delay flag on Bob) to make glare reliable. The 491 *response itself*
  still comes from the guarded YAML transition, so the production code
  path is fully exercised.
- **ICMP port unreachable for P3.2** — depends on OS behavior. On Linux,
  UDP send to a closed port returns a socket error. On macOS it's
  typically silent. If session_timer.rs can't detect the failure
  quickly, add a transaction-core test-mode timeout override.
- **YAML guard ordering** — need to verify
  `yaml_loader.rs`/state-table lookup picks the *guarded* transition
  before the unguarded one. If it uses first-match, ordering in the YAML
  matters; if it uses guard-aware selection, the guard does its job
  automatically. Needs a 10-minute spike before Phase 2 starts.
- **`dialog_to_session_id` availability in event_hub** — the cross-crate
  conversion in P1.2 needs to map `dialog_id` → `session_id`. Confirm
  this map is present on `EventHub` (it is in other arms; verify for
  `ReInvite`). No new plumbing expected.
- **Impact on session-core-v2** — none. v2 is being retired; even if
  breakage surfaces, it's expected.

---

## Sequencing

1. Phase 1 end-to-end (1.1 → 1.6) as one PR — the production fix.
2. Phase 2 (glare detection) as a second PR — needs Phase 1 to land
   first because the guarded transition depends on the wiring.
3. Phase 3 (integration tests) as the same PR as Phase 2, since they
   prove the glare detection works.
4. Phase 4 (docs) bundled with Phase 3.

All four phases should land together in practice — they're one coherent
feature. Splitting into two PRs is only for review ergonomics.
