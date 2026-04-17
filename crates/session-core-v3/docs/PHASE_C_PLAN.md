# Phase C Implementation Plan — PRACK (RFC 3262) + Session Timers (RFC 4028)

Detailed, pick-up-and-go plan for the remaining RFC-compliance gaps. Write
this assuming a future session will execute it without rediscovering the
codebase. File paths and line numbers are accurate as of commit after Phase
A+B+C.1.1 shipped.

---

## Status when this plan was written

### Done (committed in prior sessions)

- Phase A: `CallCancelled` for 487, 181/182/199 provisional mapping, UPDATE audit (and UPDATE→re-INVITE fix in hold/resume).
- Phase B: 3xx redirect auto-follow, 491 glare retry, REGISTER 423 auto-retry.
- Phase C.1.1: `RAck` header type in `rvoip-sip-core` (`crates/sip-core/src/types/rack.rs`). `RSeq`, `Method::Prack`, `HeaderName::RAck`/`RSeq`, and `Require`/`Supported` with `100rel` option-tag already existed from prior work.

### Not done (this plan covers these)

| Sub-phase | Summary |
|-----------|---------|
| C.1.2 | UAC: detect `Require: 100rel` + `RSeq` on 18x, auto-generate PRACK with `RAck`, track RSeq per dialog |
| C.1.3 | UAS: generate reliable 18x with `Require: 100rel`, retransmit with T1 backoff per RFC 3262 §3, handle incoming PRACK |
| C.1.4 | `session-core-v3` `Config.use_100rel: RelUsage` flag + wiring |
| C.1.5 | PRACK integration test (two peers, reliable 183 + PRACK exchange) |
| C.2.1 | `sip-core` `SessionExpires` and `MinSE` header types |
| C.2.2 | `dialog-core` session-timer negotiation (send `Session-Expires:` / `Min-SE:` / `Supported: timer`, handle 422, echo in 200 OK) |
| C.2.3 | `dialog-core` refresh scheduling (per-dialog timer, UPDATE/re-INVITE at half-expiry, 408-reason BYE on failure) |
| C.2.4 | `session-core-v3` session-timer config + `SessionRefreshed`/`SessionRefreshFailed` events |
| C.2.5 | Session-timer integration test |

**Estimated effort**: ~5 days of focused work. Split across at least two
sessions (PRACK first, then session timers) to keep PR scope reviewable.

---

## C.1 — PRACK / 100rel (RFC 3262)

### C.1.2 — UAC auto-PRACK on reliable 18x

**Goal**: when the UAC receives a provisional (1xx) response that carries
`Require: 100rel` and `RSeq: <n>`, auto-send PRACK with `RAck: <n> <cseq> INVITE`
and wait for 200 OK to that PRACK before proceeding. Validate monotonic RSeq
per dialog; duplicates are no-ops, out-of-order responses are dropped per
RFC 3262 §4.

**Files to modify / create**

| Path | Action | Why |
|------|--------|-----|
| `crates/dialog-core/src/dialog/dialog_impl.rs` (Dialog struct around line 26-98) | **modify** — add fields | Track `last_rseq_acked: Option<u32>` and `pending_prack: Option<RSeq>` to detect dups and drive retransmit-on-retransmit dedupe |
| `crates/dialog-core/src/transaction/dialog/quick.rs` | **modify** — new `prack_for_dialog()` helper | Mirrors `bye_for_dialog` / `update_for_dialog` pattern (see lines 56–80 for BYE, 217–259 for UPDATE) |
| `crates/dialog-core/src/transaction/dialog/mod.rs` — `request_builder_from_dialog_template` (line 101) | **modify** — add RAck header insertion when `method == Method::Prack` | The builder doesn't know about RAck; we add it after build returns |
| `crates/dialog-core/src/manager/unified.rs` | **modify** — new `send_prack()` | Follow `send_bye` (line 723) or `send_update` (line 842) pattern |
| `crates/dialog-core/src/api/unified.rs` | **modify** — new `send_prack()` public API | Follow `send_bye` (line 787) / `send_update` (line 920) |
| `crates/dialog-core/src/transaction/client/invite.rs` around line 447-459 | **modify** — detect 100rel on provisional responses | Already a good hook point: `TransactionEvent::ProvisionalResponse` is emitted from here |
| `crates/dialog-core/src/manager/transaction_integration.rs` | **modify** — new handler for `ProvisionalResponse` that triggers auto-PRACK | Need a hook in the dialog manager's transaction event loop |

**Implementation sketch**

```rust
// 1. quick.rs — build a PRACK request from dialog state
pub fn prack_for_dialog(
    call_id: impl Into<String>,
    from_uri: impl Into<String>,
    from_tag: impl Into<String>,
    to_uri: impl Into<String>,
    to_tag: impl Into<String>,
    rseq: u32,             // from the 18x's RSeq header
    invite_cseq: u32,      // from the original INVITE's CSeq
    prack_cseq: u32,       // next local CSeq for this dialog
    local_address: SocketAddr,
    route_set: Option<Vec<Uri>>,
) -> Result<Request> {
    let template = DialogRequestTemplate {
        // same pattern as bye_for_dialog: copy call_id/from/to/tags/route_set
        ..
        local_cseq: prack_cseq,
    };
    let mut req = request_builder_from_dialog_template(&template, Method::Prack, None, None)?;

    // Add RAck header after build — the builder doesn't know about it
    let rack = RAck::new(rseq, invite_cseq, Method::Invite);
    req.headers.push(TypedHeader::RAck(rack));
    Ok(req)
}
```

```rust
// 2. manager/unified.rs — orchestrate dialog lookup + send
pub async fn send_prack(
    &self,
    dialog_id: &DialogId,
    rseq: u32,
) -> ApiResult<TransactionKey> {
    // Look up dialog to get from/to/tags/route_set + the INVITE's cseq
    let dialog = self.core.get_dialog(dialog_id)?;
    let invite_cseq = dialog.invite_cseq
        .ok_or_else(|| ApiError::Internal {
            message: "send_prack: dialog has no INVITE CSeq".into(),
        })?;
    let prack_cseq = dialog.next_local_cseq();

    let request = prack_for_dialog(
        dialog.call_id.clone(),
        dialog.local_uri.to_string(),
        dialog.local_tag.clone().unwrap_or_default(),
        dialog.remote_uri.to_string(),
        dialog.remote_tag.clone().unwrap_or_default(),
        rseq,
        invite_cseq,
        prack_cseq,
        self.core.local_address,
        dialog.route_set.clone(),
    )?;

    // Send as a new non-INVITE client transaction (PRACK is a non-INVITE method)
    let tx_key = self.core.transaction_manager()
        .create_non_invite_client_transaction(request, dialog.remote_addr)
        .await?;
    Ok(tx_key)
}
```

```rust
// 3. transaction_integration.rs — auto-PRACK on reliable 18x
async fn handle_provisional_response(
    &self,
    dialog_id: &DialogId,
    response: &Response,
) -> DialogResult<()> {
    let status = response.status_code();
    if !(100..200).contains(&status) || status == 100 {
        return Ok(()); // ignore 100 Trying and non-1xx
    }
    // Check 100rel requirement
    let requires_100rel = response.headers.iter().any(|h| {
        matches!(h, TypedHeader::Require(r) if r.contains_tag("100rel"))
    });
    let rseq_hdr: Option<&RSeq> = response.headers.iter().find_map(|h| {
        if let TypedHeader::RSeq(r) = h { Some(r) } else { None }
    });
    let (Some(rseq), true) = (rseq_hdr, requires_100rel) else {
        return Ok(()); // Unreliable 18x — nothing to do
    };

    // Dedupe / monotonic check
    let mut dialog = self.core.get_dialog_mut(dialog_id)?;
    if let Some(last) = dialog.last_rseq_acked {
        if rseq.value <= last {
            tracing::debug!(
                "Duplicate/out-of-order reliable 18x for dialog {}: RSeq {} <= last acked {}",
                dialog_id, rseq.value, last
            );
            return Ok(()); // duplicate retransmit — we'd already PRACKed it
        }
    }
    dialog.last_rseq_acked = Some(rseq.value);
    drop(dialog);

    // Send PRACK
    self.send_prack(dialog_id, rseq.value).await?;
    Ok(())
}
```

**Hook point**: the cleanest place to wire the provisional-response check is
`manager/transaction_integration.rs`'s existing event-loop around
`TransactionEvent::ProvisionalResponse`. Grep for `ProvisionalResponse` in
that file; if there's no match-arm for it, add one.

**Testing (unit level)**

- Unit test in `dialog-core`: a mock dialog, feed a synthetic 183 with
  `Require: 100rel` + `RSeq: 1`; assert `send_prack` is invoked with the right
  arguments.
- Unit test for dedup: fire the same 183 twice; assert `send_prack` is called
  exactly once.

---

### C.1.3 — UAS reliable-provisional retransmission

**Goal**: when the UAS sends any 18x with SDP (typically 183 Session Progress
with early media) and the UAC offered `Supported: 100rel`, wrap it with
`Require: 100rel` + `RSeq: <n>` and retransmit with T1 backoff until PRACK
arrives or 64*T1 timeout fires.

This is the **largest single piece of remaining work** and the one most likely
to need careful design during implementation. It touches `transaction-core`
(or `dialog-core/src/transaction/`) internals.

**Files**

| Path | Action |
|------|--------|
| `crates/dialog-core/src/dialog/dialog_impl.rs` | add `local_rseq_counter: u32` (incremented per reliable 18x), `outstanding_reliable_provisionals: HashMap<u32, OutstandingProvisional>` |
| `crates/dialog-core/src/protocol/invite_handler.rs` | when sending 18x with body, wrap reliably if INVITE had `Supported: 100rel` or `Require: 100rel` |
| `crates/dialog-core/src/transaction/server/reliable_invite.rs` (new) | timer-based retransmission FSM (§3 of RFC 3262: T1, 2·T1, 4·T1, …, cap at T2=4s, abort at 64·T1) |
| `crates/dialog-core/src/protocol/prack_handler.rs` (new) | incoming PRACK: validate `RAck` matches an outstanding provisional, send 200 OK for the PRACK (non-INVITE server transaction), stop retransmits |
| `crates/dialog-core/src/manager/core.rs` | route incoming PRACK requests (method == `Method::Prack`) to the prack_handler |

**Retransmission machinery sketch**

```rust
pub struct OutstandingProvisional {
    pub rseq: u32,
    pub response: Response,       // full 18x to retransmit
    pub retries: u32,
    pub next_timeout: Instant,
    pub abort_deadline: Instant,  // send + 64·T1
}

impl DialogManager {
    fn start_reliable_provisional_retransmit(
        &self,
        dialog_id: DialogId,
        response: Response,
        rseq: u32,
    ) {
        let provisional = OutstandingProvisional {
            rseq,
            response: response.clone(),
            retries: 0,
            next_timeout: Instant::now() + Duration::from_millis(500), // T1
            abort_deadline: Instant::now() + Duration::from_millis(32_000), // 64·T1
        };

        // Store on dialog
        // ...

        // Spawn a retransmit task
        let mgr = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(...)).await;
                // Check if PRACKed (remove from outstanding map). If yes, exit.
                // Otherwise resend + double timeout up to 4s cap.
                // After 64·T1: give up, terminate dialog with 504 or similar.
            }
        });
    }
}
```

The UPDATE retransmit in `crates/dialog-core/src/transaction/client/non_invite.rs`
is a useful reference — same T1 backoff structure applies.

**PRACK handler sketch**

```rust
// prack_handler.rs
async fn handle_prack(&self, request: Request, ...) -> DialogResult<()> {
    let rack = request.headers.iter().find_map(|h| {
        if let TypedHeader::RAck(r) = h { Some(r) } else { None }
    }).ok_or_else(|| DialogError::Protocol("PRACK missing RAck".into()))?;

    let dialog = self.find_dialog_for_request(&request)?;
    let mut dialog_mut = self.get_dialog_mut(&dialog)?;

    // Find outstanding provisional matching rack
    let matched = dialog_mut.outstanding_reliable_provisionals
        .remove(&rack.rseq);
    if matched.is_none() {
        // No match — could be a spurious PRACK; RFC 3262 says ignore
        return Ok(());
    }
    // Send 200 OK to the PRACK
    let ok = build_ok_response(&request);
    self.send_response(transaction_id, ok).await?;
    Ok(())
}
```

**Testing**

- Integration test in `dialog-core`: start a UAS, have it send a reliable
  183 that gets PRACKed; assert no retransmits happen.
- Integration test: UAS sends reliable 183, UAC drops the first PRACK;
  assert the UAS retransmits after 500 ms and succeeds on the second PRACK.

---

### C.1.4 — `session-core-v3` `Config.use_100rel`

**Files**

- `crates/session-core-v3/src/api/unified.rs` — `Config` struct: add `pub use_100rel: RelUsage` with a new enum `RelUsage { NotSupported, Supported, Required }`, default `Supported`.
- `crates/session-core-v3/src/api/unified.rs` — `create_dialog_api()` around line 479: thread `use_100rel` into the `DialogManagerConfig`.
- `crates/dialog-core/src/config.rs` (if one exists) — add `use_100rel: RelUsage` field.
- `crates/dialog-core/src/protocol/invite_handler.rs` — on outgoing INVITE, inject `Supported: 100rel` or `Require: 100rel` per config.
- `crates/dialog-core/src/protocol/invite_handler.rs` — on incoming INVITE, if our policy is `Required` but caller didn't list 100rel in `Supported:`, respond `420 Bad Extension` with `Unsupported: 100rel`.

**Testing**

Unit tests in `session-core-v3` that inspect the config propagation;
integration test lives under C.1.5.

---

### C.1.5 — PRACK integration test

**File**: `crates/session-core-v3/tests/prack_test.rs` (new)

**Scenario**: Two peers in separate binaries, similar to the existing
`examples/callbackpeer/auto_answer/` pattern:

1. UAS configured with `use_100rel: Required`.
2. UAS's `on_incoming_call` defers the accept and the framework sends a
   reliable 183 Session Progress with SDP.
3. UAC (`use_100rel: Supported`) should receive the reliable 183 and send PRACK.
4. UAC receives 200 OK for the PRACK.
5. UAS then accepts with 200 OK; normal ACK flow follows.
6. Assert both sides see the call as Established within 2 s.

Additional negative test in the same file:
- UAC with `use_100rel: NotSupported`, UAS with `Required` → UAC should
  receive `420 Bad Extension` and `CallFailed { status_code: 420, … }`.

---

## C.2 — Session-Expires / RFC 4028

### C.2.1 — `sip-core` header types

**File**: `crates/sip-core/src/types/session_expires.rs` (new)

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionExpires {
    pub delta_seconds: u32,
    pub refresher: Option<Refresher>, // refresher=uas|uac, optional
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refresher { Uac, Uas }
```

**File**: `crates/sip-core/src/types/min_se.rs` — likely already exists (grep
in sip-core for `MinSE`; the `TypedHeader::MinSE` variant is already there per
typed_header.rs line 169). Verify it has a usable `from_str` that accepts
`"<seconds>"`. If not, extend.

**Check**: `HeaderName::SessionExpires` and `HeaderName::MinSE` should already
exist in `crates/sip-core/src/types/headers/header_name.rs`. The mapped
string matching also needs to exist (`from_str` lowercase map).

**Parser**: follow the `MinExpires` pattern for parameterised duration-style
headers. The `refresher=…` parameter parsing should leverage the generic
`Param` infrastructure (grep for `;q=` handling in Contact headers).

**Testing**: unit tests for roundtrip parse/serialize including missing and
present `refresher=` parameter.

---

### C.2.2 — `dialog-core` session-timer negotiation

**Files**

- `crates/dialog-core/src/dialog/dialog_impl.rs` — new fields:
  - `session_expires: Option<Duration>`
  - `session_refresher: Option<Refresher>`
  - `session_refresh_task: Option<JoinHandle<()>>`
  - `session_min_se: Duration`

- `crates/dialog-core/src/protocol/invite_handler.rs`:
  - **Outgoing INVITE**: if `Config.session_timer_secs` is `Some(s)`, inject
    `Session-Expires: s;refresher=uac`, `Min-SE: <min>`, `Supported: timer`.
  - **Incoming INVITE**: parse `Session-Expires:` and `Min-SE:`. If peer's
    `Min-SE > our Session-Expires`, respond `422 Session Interval Too Small`
    with our `Min-SE:` header.
  - **Outgoing 200 OK (UAS)**: echo `Session-Expires:` with the negotiated
    refresher. Prefer `refresher=uac` for NAT-preserving setups (UAC is
    typically behind NAT; refreshing keeps its pinhole alive).

- `crates/dialog-core/src/manager/transaction_integration.rs`:
  - On receiving 200 OK at the UAC side: parse echo'd `Session-Expires:`
    and `refresher=`. Note who's responsible.
  - On receiving 422: parse `Min-SE:` from response; either bump our local
    setting and retry, or surface as failure up to session-core-v3.

**Testing**

- Unit test: build a Request with `session_timer_secs: Some(1800)`; assert
  the `Session-Expires:` header is present with correct seconds.
- Unit test: 422 response with `Min-SE: 300` → retry should use 300.

---

### C.2.3 — `dialog-core` refresh scheduling

**File**: `crates/dialog-core/src/manager/session_timer.rs` (new)

**Core task** spawned per dialog when session timer is negotiated:

```rust
async fn run_session_refresh_task(
    dialog_id: DialogId,
    manager: Arc<DialogManager>,
    interval: Duration,
    is_refresher: bool,
) {
    loop {
        tokio::time::sleep(interval / 2).await; // refresh at half-expiry
        let refresh_at = Instant::now() + interval / 2;

        if is_refresher {
            // Issue UPDATE (fallback to re-INVITE on 501 Not Implemented)
            match manager.send_update(&dialog_id, None).await {
                Ok(_) => { /* reschedule implicit: loop */ }
                Err(_) => {
                    // Retry once with re-INVITE before giving up
                    let _ = manager.send_reinvite(&dialog_id).await;
                }
            }
        } else {
            // We're not the refresher — just check that we heard from them
            let dialog = manager.get_dialog(&dialog_id)?;
            if dialog.last_refresh_received < refresh_at - Duration::from_secs(1) {
                // Peer failed to refresh in time; tear down
                send_bye_with_reason(&dialog_id, 408, "Session expired").await;
                break;
            }
        }
    }
}
```

**Failure path**: if a UPDATE/re-INVITE refresh fails (488, 500, timeout),
send BYE with `Reason: SIP ;cause=408;text="Session expired"` per RFC 4028 §10.

**Cancellation**: the task's JoinHandle must be cancelled/dropped whenever
the dialog terminates (BYE or other). Hook into the dialog cleanup path —
likely `DialogManager::terminate_dialog()`.

**Reuse**: `send_update` already exists (dialog-core/src/manager/unified.rs:842).
`send_reinvite` — check for it; likely a `send_request_in_dialog(dialog_id,
Method::Invite, body)` pattern.

---

### C.2.4 — `session-core-v3` config + events

**Files**

- `crates/session-core-v3/src/api/unified.rs` — `Config` struct:
  ```rust
  pub session_timer_secs: Option<u32>,  // None = disabled
  pub session_timer_min_se: u32,        // default 90
  ```
- Thread into `create_dialog_api()`.

- `crates/session-core-v3/src/api/events.rs` — new events:
  ```rust
  Event::SessionRefreshed { call_id: CallId },
  Event::SessionRefreshFailed { call_id: CallId, reason: String },
  ```
  Update `call_id()`, `is_call_event()` matchers.

- `crates/session-core-v3/src/adapters/session_event_handler.rs` — handle new
  `DialogToSessionEvent::SessionRefreshed` / `SessionRefreshFailed` variants
  and publish the app-level events. Follow the pattern of the existing
  `handle_call_failed` / `handle_call_cancelled` methods.

- `crates/infra-common/src/events/cross_crate.rs` — add the two new
  `DialogToSessionEvent` variants + exhaustive match arm updates.

---

### C.2.5 — Session-timer integration test

**File**: `crates/session-core-v3/tests/session_timer_test.rs` (new)

**Positive case**:
1. Two peers, both with `session_timer_secs: Some(10), min_se: 5`.
2. UAC calls UAS; normal setup.
3. Wait 6 seconds; assert the UAC emitted `Event::SessionRefreshed`.
4. Wait another 10 seconds; assert a second refresh.
5. Call still active at 20 s; hang up normally.

**Failure case**:
1. UAS configured to ignore UPDATE (simulate by mutating its handler to
   swallow UPDATE events). UAC `session_timer_secs: Some(10)`.
2. Wait 12 s; assert UAC sends BYE with `Reason: SIP ;cause=408`.
3. Assert UAC's session ends with `CallEnded`.

---

## Execution order (recommended)

1. **Week 1 / Session 1** — C.1 (PRACK) front to back:
   - C.1.1 ✅ already done
   - C.1.2 UAC (~1 day)
   - C.1.4 config (~1 hour)
   - C.1.5 test (~2 hours — covers UAC-only path; UAS-simulator uses a
     hand-built 18x)
   - Ship a commit here. Real-world utility: covers all carriers that require
     100rel from UAC; UAS-side coverage can follow.

2. **Week 1 / Session 2** — C.1.3 UAS retransmission (1.5 days):
   - Implement UAS reliable 18x + retransmit timer
   - Extend C.1.5 test to exercise UAS-side retransmission
   - Ship commit

3. **Week 2 / Session 3** — C.2 (session timers) front to back:
   - C.2.1 headers (~half day)
   - C.2.2 negotiation (~1 day)
   - C.2.3 refresh scheduling (~1.5 days — biggest piece)
   - C.2.4 config + events (~1 hour)
   - C.2.5 test (~2 hours)
   - Ship commit

4. **Finalize** — update `RFC_COMPLIANCE_STATUS.md` to mark PRACK and
   session timers as ✅ supported. Remove from the "Known gaps" section.

---

## Risks & unknowns

- **Transaction-core state-machine changes for C.1.3**: the reliable
  provisional retransmission logic lives below the dialog layer. If
  `transaction-core` is a separate crate (check `crates/` layout), expect
  changes to span `transaction-core` → `dialog-core` → `session-core-v3`.
  Budget extra time for cross-crate API changes.

- **Backwards compat for `TypedHeader` and `DialogToSessionEvent` enums**:
  every new variant forces an exhaustive-match update. Grep for `match event`
  / `match &header` across all workspace crates after each variant add. The
  pattern is well-understood by now.

- **Global event coordinator (session-core-v3)**: stays singleton. Any new
  `DialogToSessionEvent` variant routed via this bus must be filtered by
  session ownership in `session_event_handler.rs` like the existing handlers
  (see `is_our_session` uses).

- **Test environment**: single-process multi-peer examples work but are
  artificial. Consider running the PRACK test against a real SIP PBX
  (FreeSWITCH, Asterisk) or SIPp scripts as part of CI after the code
  change lands. There's existing SIPp infrastructure under
  `crates/call-engine/examples/e2e_test/`.

---

## File inventory

### New files this plan creates

- `crates/dialog-core/src/transaction/server/reliable_invite.rs`
- `crates/dialog-core/src/protocol/prack_handler.rs`
- `crates/dialog-core/src/manager/session_timer.rs`
- `crates/sip-core/src/types/session_expires.rs`
- `crates/session-core-v3/tests/prack_test.rs`
- `crates/session-core-v3/tests/session_timer_test.rs`

### Files this plan modifies

- `crates/sip-core/src/types/min_se.rs` (extend, or create if missing)
- `crates/sip-core/src/types/headers/typed_header.rs` (new variants + dispatch)
- `crates/sip-core/src/types/headers/header_name.rs` (verify headers present)
- `crates/sip-core/src/types/mod.rs` (re-exports)
- `crates/sip-core/src/builder/request.rs` & `response.rs` (match arms)
- `crates/dialog-core/src/dialog/dialog_impl.rs` (new fields)
- `crates/dialog-core/src/protocol/invite_handler.rs` (100rel + timer hooks)
- `crates/dialog-core/src/manager/unified.rs` (send_prack + timer start/stop)
- `crates/dialog-core/src/api/unified.rs` (send_prack public API)
- `crates/dialog-core/src/manager/transaction_integration.rs` (auto-PRACK event hook)
- `crates/dialog-core/src/manager/core.rs` (route PRACK)
- `crates/dialog-core/src/transaction/dialog/quick.rs` (prack_for_dialog)
- `crates/dialog-core/src/transaction/dialog/mod.rs` (RAck insertion)
- `crates/infra-common/src/events/cross_crate.rs` (SessionRefreshed, SessionRefreshFailed variants)
- `crates/session-core-v3/src/api/unified.rs` (Config fields)
- `crates/session-core-v3/src/api/events.rs` (new Event variants)
- `crates/session-core-v3/src/adapters/session_event_handler.rs` (handlers)
- `crates/session-core-v3/docs/RFC_COMPLIANCE_STATUS.md` (mark both ✅ when done)
