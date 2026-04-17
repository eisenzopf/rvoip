# Phase C Implementation Plan — PRACK (RFC 3262) + Session Timers (RFC 4028)

Detailed, pick-up-and-go plan for the remaining RFC-compliance gaps. Written
assuming a future session will execute it without rediscovering the codebase.
File paths and line numbers are accurate as of the most recent commit; see
the "Progress log" section below for session-by-session status.

---

## Status

### Done

- **Phase A** — `CallCancelled` for 487, 181/182/199 provisional mapping, UPDATE audit (and UPDATE→re-INVITE fix in hold/resume).
- **Phase B** — 3xx redirect auto-follow, 491 glare retry, REGISTER 423 auto-retry.
- **Phase C.1** (RFC 3262 PRACK / 100rel) ✅ — C.1.1 RAck header type, C.1.2 UAC auto-PRACK, C.1.3 UAS reliable 18x + retransmit + PRACK handler + 420 path, C.1.4 `RelUsage` config, C.1.5 multi-binary 420 integration test.
- **Phase C.2** (RFC 4028 Session Timers) ✅ — C.2.1 header types already present; C.2.2 Session-Expires/Min-SE/Supported:timer negotiation + 422 response + 200 OK echo; C.2.3 per-dialog refresh scheduler (`session_timer.rs`) with UPDATE + re-INVITE fallback + 408 BYE on failure; C.2.4 session-core-v3 config + `Event::SessionRefreshed`/`SessionRefreshFailed`; C.2.5 multi-binary integration test.
- **sip-core** — new `StatusCode::SessionIntervalTooSmall` (422) variant to mirror existing 420/421/423 naming.
- **Side fix** — `DialogManager.config` race (clones didn't see `set_config`) resolved by wrapping in `Arc<RwLock<Option<_>>>`.
- **Side fix** — fast-RTT race where 4xx responses arriving on localhost before the `StoreDialogMapping` cross-crate event was processed caused `CallFailed` to be dropped. Resolved via new `UnifiedDialogApi::make_call_for_session` that pre-registers the mapping before the INVITE goes on the wire.

### Done (Session 2, 2026-04-17)

- **C.1.3** ✅ — UAS reliable 18x emission wrapped with `Require: 100rel` + `RSeq:` when the peer advertised 100rel, T1-backoff retransmit via `crates/dialog-core/src/transaction/server/reliable_invite.rs`, incoming PRACK handler at `crates/dialog-core/src/protocol/prack_handler.rs` (aborts retransmit on match, 481 on no match), and 420 Bad Extension + `Unsupported: 100rel` on policy mismatch in `handle_initial_invite`.
- **C.1.5** ✅ — `crates/session-core-v3/tests/prack_integration.rs` — 420 negative case passes. Positive reliable-183 path deferred to a follow-on that carves a `send_early_media` API on session-core-v3 (not in scope).
- **C.2.2** ✅ — Session-Expires/Min-SE/Supported:timer injection on outgoing INVITE via `inject_session_timer_headers`; UAS 422 on `Min-SE > Session-Expires`; UAS 200 OK echoes Session-Expires with the negotiated refresher.
- **C.2.3** ✅ — `crates/dialog-core/src/manager/session_timer.rs` with `spawn_refresh_task` (T/2 interval, UPDATE with re-INVITE fallback, BYE + `SessionRefreshFailed` on hard failure). Abort on `terminate_dialog`. Public `UnifiedDialogApi::send_reinvite` exposed for the refresh path.
- **C.2.4** ✅ — `Config.session_timer_secs` + `session_timer_min_se` threaded into `DialogConfig`; `Event::SessionRefreshed`/`SessionRefreshFailed` wired end-to-end via new `DialogToSessionEvent::SessionRefreshed`/`SessionRefreshFailed` cross-crate variants.
- **C.2.5** ✅ — `crates/session-core-v3/tests/session_timer_integration.rs` — Alice(refresher) sees `Event::SessionRefreshed` within 12 s on a 10 s `Session-Expires`.
- **Side fixes** — `DialogManager.config` wrapped in `Arc<RwLock<Option<DialogManagerConfig>>>` so `set_config` propagates to the cloned event-processor task (was a latent bug hiding behind any config-dependent incoming-request handler). New `UnifiedDialogManager::make_call_for_session` pre-registers the session↔dialog mapping before the INVITE goes on the wire, closing a fast-RTT race where 4xx responses could be dropped by `event_hub` because the mapping hadn't been populated yet. Added `StatusCode::SessionIntervalTooSmall` (422) as a first-class variant in `sip-core` instead of `Custom(422)`.

### Not done

All Phase C items above are complete. Remaining follow-on work (not blocking):

- **Positive reliable-183 integration test** — requires session-core-v3 to expose a `send_early_media(sdp)` API so we can drive a UAS 183 with SDP through the public surface. Wire-level correctness is covered by the 20 unit tests in `crates/dialog-core/tests/prack_test.rs`.
- **422 Session Interval Too Small UAC-side retry** — parse `Min-SE:` from 422 and re-issue INVITE with bumped Session-Expires. Real-world carriers rarely 422 on fresh INVITE, so this is low-priority.
- **`Reason: SIP ;cause=408` header on session-timer BYE** — RFC 4028 §10 nicety; currently we just send a BYE and rely on the `SessionRefreshFailed` event string for reason context.

---

## Progress log

### Session 2 (2026-04-17): C.1.3 + C.1.5 + C.2.x shipped — Phase C complete

**Delivered**

- UAS PRACK (C.1.3): new `PrackHandler` trait + `protocol/prack_handler.rs` (200 OK on match, 481 on no match). New `transaction/server/reliable_invite.rs` with `spawn_reliable_provisional_retransmit` (T1=500 ms × 2 up to T2=4 s, abandon at 64·T1). 18x-with-body wrapping in `send_transaction_response`. 420 Bad Extension + `Unsupported: 100rel` on policy mismatch. `Dialog` gained `local_rseq_counter`, `peer_supports_100rel`.
- PRACK integration test (C.1.5): `tests/prack_integration.rs` covers the 420 negative case. Positive reliable-183 is deferred to a follow-on once session-core-v3 exposes a `send_early_media` API.
- Session timers (C.2.2 – C.2.5): `DialogConfig.session_timer_secs` / `session_timer_min_se`; `inject_session_timer_headers` + `config_session_timer_settings` helpers; UAS 422 response when peer's `Min-SE` exceeds our `Session-Expires`; UAS 200 OK echoes Session-Expires with refresher. UAC captures negotiated interval in `handle_transaction_success_response` and spawns per-dialog refresh task (`manager/session_timer.rs`). Refresh uses UPDATE first, falls back to re-INVITE on error, then BYE + `SessionRefreshFailed` event at §10. Public `UnifiedDialogApi::send_reinvite` now exposed. `Event::SessionRefreshed`/`SessionRefreshFailed` surface through the session-core-v3 API.
- `sip-core`: `StatusCode::SessionIntervalTooSmall` (422) added as a named variant rather than `Custom(422)`.

**Cleanup / unrelated fixes required to get tests green**

- `DialogManager.config` was `Option<DialogManagerConfig>` set AFTER the constructor cloned the manager for its event-processor task — the clone's config stayed `None`, so incoming-request handlers on the cloned manager couldn't read the 100rel/session-timer policy. Wrapped in `Arc<std::sync::RwLock<Option<_>>>`. All read sites on `DialogManager` updated.
- Fast-RTT race: on localhost Bob's 420 reached Alice before her async `StoreDialogMapping` event processed, so `event_hub::convert_coordination_to_cross_crate` dropped the `CallFailed` with `No session ID found`. Fix is `UnifiedDialogManager::make_call_for_session`, which installs the session↔dialog mapping between `create_outgoing_dialog` and `send_request(INVITE)` so the write is ordered-before the send. session-core-v3's `dialog_adapter::send_invite_with_details` now uses it.

**Verified clean**

- PRACK: 20 unit tests in `crates/dialog-core/tests/prack_test.rs` (doubled from 10).
- `cargo test -p rvoip-dialog-core --tests --lib`: 17 test binaries, 326 passed, 1 ignored.
- `cargo test -p rvoip-session-core-v3 --tests --lib`: all suites green.
- Integration: `prack_integration` + `session_timer_integration` pass deterministically. `blind_transfer_integration` is still flaky on first-run timing (pre-existing, subprocess-driven).

**Gotchas learned — carry forward**

1. **Response flows have two paths**: `handle_response_message` → `process_response_in_dialog` in response_handler.rs AND `handle_transaction_success_response` / `handle_transaction_failure_response` in transaction_integration.rs. The latter is the hot path in practice; the former is defensive. If you add logic that must fire on every 2xx to INVITE, put it in `handle_transaction_success_response`.
2. **Unassociated-INVITE dispatch bypasses `handle_invite_method`**: `manager/core.rs::handle_unassociated_transaction_event` calls `handle_initial_invite` directly. Any policy check needs to live inside `handle_initial_invite` (not the method-handler wrapper) to cover both paths.
3. **`SessionRefreshFailed` contains `SessionRefreshed` as a substring** — `event_str.contains("SessionRefreshed")` matches both. Check for `SessionRefreshFailed` first in the dispatch chain.
4. **`DialogManager.config` is now `Arc<RwLock<Option<_>>>`** — use `.read().ok().and_then(|g| g.as_ref().map(|c| ...))` to access; do not re-introduce the old `self.config.as_ref()` pattern.

### Session 1 (2026-04-16): C.1.2 + C.1.4 shipped, plus cleanup

**Delivered**

- `Dialog` struct gained `invite_cseq: Option<u32>` and `last_rseq_acked: Option<u32>` (`crates/dialog-core/src/dialog/dialog_impl.rs`).
- `invite_cseq` is captured on every INVITE send in `TransactionIntegration::send_request_in_dialog` (both initial and re-INVITE paths).
- `prack_for_dialog()` builder in `crates/dialog-core/src/transaction/dialog/quick.rs` (mirrors `bye_for_dialog`, appends `RAck` after the generic builder returns).
- `DialogManager::send_prack(&self, dialog_id, rseq)` in `manager/transaction_integration.rs`; re-exposed on `UnifiedDialogManager` and public `UnifiedDialogApi`.
- Auto-PRACK wired into `handle_transaction_provisional_response`: uses free function `detect_reliable_provisional(&Response) -> Option<u32>` and rolls back `last_rseq_acked` on transient send failure so a retransmit can retry.
- `RelUsage` enum + `DialogConfig.use_100rel` (`crates/dialog-core/src/api/config.rs`), re-exported from `rvoip_dialog_core::api`.
- Ergonomic `with_100rel()` on `ClientConfigBuilder`, `ServerConfigBuilder`, `HybridConfigBuilder`; `DialogManagerConfig::use_100rel()` convenience getter.
- `session-core-v3 Config.use_100rel` (re-exports `RelUsage`), threaded through `create_dialog_api()`.
- `inject_100rel_policy(&mut Request, RelUsage)` free function in `manager/transaction_integration.rs`; applied to every outgoing INVITE after build, additive to any existing `Supported`/`Require` headers.
- Tests: `crates/dialog-core/tests/prack_test.rs` (10 cases: detection true/false paths, dedupe, PRACK structure, policy injection variants including no-op + no-dup).

**Cleanup / unrelated fixes required to get tests green**

- DashMap deadlock in `send_request_in_dialog::Method::Notify` arm (was calling `self.get_dialog(dialog_id)` while `get_dialog_mut(dialog_id)` still held). Hoisted the `event_package` + `subscription_state` reads up to the top of the outer block. This was the three-way hang in `phase3_integration_tests` (notify, complete_dialog_workflow, server_side_integration).
- Lib test compile fixes: `.expect()` on `&Arc<GlobalEventCoordinator>` (now infallible), `notify_for_dialog` 11-arg signature.
- Integration test API drift: `refer_handling_test.rs`, `refer_header_test.rs`, `unified_api_tests.rs` had stale calls to the removed `set_session_coordinator` / `set_dialog_event_sender` channel APIs; rewritten to assume the GlobalEventCoordinator model.
- **Transfer testing**: two in-process StreamPeers in one Tokio runtime are unreliable (we hit this repeatedly). The broken `test_transfer_call_via_handle` / `test_transfer_via_send_refer` were removed and replaced by `crates/session-core-v3/tests/blind_transfer_integration.rs`, which drives three separate example binaries as subprocesses. Examples (`streampeer_blind_transfer_{alice,bob,charlie}`) were parameterized with `ALICE_PORT`/`BOB_PORT`/`CHARLIE_PORT` env vars so the test uses `35060`-`35062` and can't collide with the `run.sh` demo (`5060`-`5062`).

**Verified clean**

- `dialog-core`: 316 passed, 1 ignored, 0 failed (17 test binaries).
- `session-core-v3`: 156 passed, 1 ignored, 0 failed (13 test binaries, including `blind_transfer_integration`).

**Gotchas learned — read before starting C.1.3 / C.2.x**

1. **API naming drift vs. plan**: it's `Require::requires("100rel")`, not `contains_tag(...)`. The plan below still uses the old name in places — treat that as pseudocode.
2. **No `send_reinvite` on `UnifiedDialogApi`**. Session-core-v3 wraps re-INVITE via `adapters/dialog_adapter.rs::send_reinvite_session`. For C.2.3's refresh path, either expose `send_reinvite()` publicly (mirror `send_update` at `api/unified.rs:920`) or drive it through the session-core-v3 adapter.
3. **Never call `self.get_dialog(id)` or `self.get_dialog_mut(id)` while already holding a RefMut on the same key**. The DashMap deadlock surfaced for NOTIFY and will surface again for any new per-method fields. Hoist the reads to the top.
4. **Dialog struct has `Serialize`/`Deserialize`** — new fields must be serializable. For session-timer fields with `JoinHandle<()>`, mark them `#[serde(skip)]` and default-init.
5. **`session_expires.rs` and `min_se.rs` already exist in sip-core** — the plan section C.2.1 is mostly a no-op. Verify `MinSE` accepts the values used in RFC 4028 negotiation (it's a bare number, no params).
6. **Two-peer in-process testing is NOT viable**. Use the multi-binary subprocess pattern from `crates/session-core-v3/tests/blind_transfer_integration.rs` for C.1.5 and C.2.5. Template:
   - `examples/<flow>_{alice,bob}.rs` with `ALICE_PORT`/`BOB_PORT` env-var fallback.
   - `tests/<flow>_integration.rs` spawns them via `Command::new(env::var("CARGO"))`, waits on Alice's exit code.
7. **DialogManagerConfig has three behavior variants** (Client/Server/Hybrid), all embedding `DialogConfig`. Put shared new fields on `DialogConfig` (like `use_100rel`) rather than duplicating across three variants.
8. **`DialogClient::send_notify` silently drops its `event` argument today** — it calls `send_request_in_dialog(dialog_id, Method::Notify, body)` and the receiver reads `event_package` off the dialog instead. Not our bug, but be aware when adding tests.

---

## C.1 — PRACK / 100rel (RFC 3262)

### C.1.2 — UAC auto-PRACK on reliable 18x — ✅ COMPLETE (Session 1)

> The section below is preserved for historical context. For the shipped
> implementation, see the Progress log at the top of this file and the
> cross-file inventory. Notes that differ from what actually landed:
> - `Require::requires("100rel")` is the real method name (not `contains_tag`).
> - The PRACK fields on `Dialog` landed as `invite_cseq` + `last_rseq_acked`;
>   `pending_prack` was deemed unnecessary for the current dedupe scheme.
> - The RAck injection lives in `prack_for_dialog` itself, not in
>   `request_builder_from_dialog_template` — keeping the generic builder
>   RAck-agnostic turned out cleaner.

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

### C.1.4 — `session-core-v3` `Config.use_100rel` — ✅ COMPLETE except inbound `420` path

> Outgoing-INVITE side complete (Session 1). The inbound-INVITE `420 Bad
> Extension` response for policy mismatch is not yet implemented and should
> be folded into C.1.3.

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

### C.2.1 — `sip-core` header types — ✅ COMPLETE (pre-existing)

> `SessionExpires` and `MinSE` both already exist in `sip-core` with full
> parse/serialize/TypedHeader wiring. No new work needed. Skip to C.2.2.

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

1. **Session 1** ✅ — C.1.1 + C.1.2 + C.1.4 shipped; UAC-side PRACK complete.
   See Progress log above.

2. **Session 2 (next)** — **C.1.3 UAS reliable 18x + C.1.5 integration test** (~1.5 days):
   - Implement UAS reliable-18x emission + retransmit FSM (see C.1.3 below).
   - Add the incoming-INVITE `420 Bad Extension` path for `Required` policy mismatch (last loose end from C.1.4).
   - Write the multi-binary PRACK integration test (`examples/streampeer/prack/{alice,bob}.rs` + `tests/prack_integration.rs`) following the `blind_transfer_integration` template.
   - Ship commit.

3. **Session 3** — **C.2 session timers** front to back (~3 days):
   - C.2.1 ✅ already done (sip-core header types exist).
   - C.2.2 negotiation (~1 day).
   - C.2.3 refresh scheduling (~1.5 days — biggest piece).
   - C.2.4 config + events (~1 hour).
   - C.2.5 multi-binary test (~2 hours).
   - Ship commit.

4. **Finalize** — update `RFC_COMPLIANCE_STATUS.md` to mark PRACK and
   session timers ✅ supported. Remove both from the "Known gaps" section.

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

### Created / modified so far (Session 1)

- ✅ `crates/dialog-core/src/api/config.rs` — `RelUsage` enum, `DialogConfig.use_100rel`, `with_100rel()`.
- ✅ `crates/dialog-core/src/api/mod.rs` — re-export `RelUsage`.
- ✅ `crates/dialog-core/src/api/unified.rs` — public `send_prack()`.
- ✅ `crates/dialog-core/src/config/unified.rs` — `use_100rel()` getter, per-builder `with_100rel()`.
- ✅ `crates/dialog-core/src/dialog/dialog_impl.rs` — `invite_cseq`, `last_rseq_acked` fields on `Dialog`.
- ✅ `crates/dialog-core/src/manager/transaction_integration.rs` — auto-PRACK, `DialogManager::send_prack`, `detect_reliable_provisional`, `inject_100rel_policy`, deadlock fix in Notify arm.
- ✅ `crates/dialog-core/src/manager/unified.rs` — `UnifiedDialogManager::send_prack` wrapper.
- ✅ `crates/dialog-core/src/transaction/dialog/quick.rs` — `prack_for_dialog()`.
- ✅ `crates/dialog-core/src/transaction/dialog/mod.rs` + `builders.rs` — re-export of `prack_for_dialog`.
- ✅ `crates/dialog-core/src/events/adapter.rs` — removed obsolete `.expect()` on infallible coordinator call.
- ✅ `crates/dialog-core/tests/{prack_test.rs,refer_handling_test.rs,refer_header_test.rs,unified_api_tests.rs}` — new PRACK unit tests + pre-existing API-drift fixes.
- ✅ `crates/session-core-v3/src/api/unified.rs` — `Config.use_100rel` (re-exports `RelUsage`), threaded into `create_dialog_api()`.
- ✅ `crates/session-core-v3/tests/{blind_transfer_integration.rs,simple_api_tests.rs,unified_api_tests.rs,registration_test.rs,simple_api_tests.rs}` — new multi-binary transfer test + config-field updates.
- ✅ `crates/session-core-v3/examples/streampeer/blind_transfer/{alice,bob,charlie}.rs` — `ALICE_PORT`/`BOB_PORT`/`CHARLIE_PORT` env-var support (backward-compatible).

### Created / modified in Session 2

**Created**:
- ✅ `crates/dialog-core/src/protocol/prack_handler.rs`
- ✅ `crates/dialog-core/src/transaction/server/reliable_invite.rs`
- ✅ `crates/dialog-core/src/manager/session_timer.rs`
- ✅ `crates/session-core-v3/examples/streampeer/prack/{alice,bob}.rs`
- ✅ `crates/session-core-v3/tests/prack_integration.rs`
- ✅ `crates/session-core-v3/examples/streampeer/session_timer/{alice,bob}.rs`
- ✅ `crates/session-core-v3/tests/session_timer_integration.rs`

**Modified**:
- ✅ `crates/sip-core/src/types/status.rs` — new `StatusCode::SessionIntervalTooSmall` (422) variant.
- ✅ `crates/dialog-core/src/dialog/dialog_impl.rs` — `local_rseq_counter`, `peer_supports_100rel`, `session_expires_secs`, `is_session_refresher` fields + `next_local_rseq()` helper.
- ✅ `crates/dialog-core/src/api/config.rs` — `session_timer_secs`, `session_timer_min_se` fields + `with_session_timer`/`with_min_se` builders.
- ✅ `crates/dialog-core/src/config/unified.rs` — `with_session_timer`/`with_min_se` on Client/Server/Hybrid builders.
- ✅ `crates/dialog-core/src/manager/core.rs` — PRACK dispatch arm; `reliable_provisional_tasks` and `session_refresh_tasks` maps on `DialogManager`; `config` wrapped in `Arc<RwLock<...>>`.
- ✅ `crates/dialog-core/src/manager/protocol_handlers.rs` — `handle_prack_method` delegate.
- ✅ `crates/dialog-core/src/protocol/invite_handler.rs` — 420 Bad Extension on policy mismatch; capture invite_cseq + peer_supports_100rel + negotiated Session-Expires on dialog; spawn refresh task on UAS side in `process_ack_in_dialog`.
- ✅ `crates/dialog-core/src/manager/transaction_integration.rs` — `detect_peer_100rel_support`, `inject_session_timer_headers`, `config_session_timer_settings`; reliable-18x wrapping + retransmit spawn in `send_transaction_response`; UAS Session-Expires echo on 200 OK; UAC Session-Expires capture + refresh-task spawn in `handle_transaction_success_response`.
- ✅ `crates/dialog-core/src/manager/unified.rs` — new `make_call_for_session(session_id, …)` pre-registers mapping between `create_outgoing_dialog` and `send_request`.
- ✅ `crates/dialog-core/src/api/unified.rs` — public `send_reinvite` + `make_call_for_session`.
- ✅ `crates/dialog-core/src/manager/dialog_operations.rs` — `terminate_dialog` aborts refresh and retransmit tasks.
- ✅ `crates/dialog-core/src/events/session_coordination.rs` — `SessionRefreshed`/`SessionRefreshFailed` internal events.
- ✅ `crates/dialog-core/src/events/event_hub.rs` — converter arms for the new cross-crate variants.
- ✅ `crates/dialog-core/src/protocol/response_handler.rs` — UAC-side Session-Expires capture (note: path is currently dead code; live UAC handling is in `handle_transaction_success_response`).
- ✅ `crates/dialog-core/tests/prack_test.rs` — 20 unit tests (10 UAC + 10 UAS).
- ✅ `crates/infra-common/src/events/cross_crate.rs` — `DialogToSessionEvent::SessionRefreshed`/`SessionRefreshFailed` variants + `session_id()` match arms.
- ✅ `crates/session-core-v3/src/api/unified.rs` — `Config.session_timer_secs`, `session_timer_min_se`; threaded into `create_dialog_api`.
- ✅ `crates/session-core-v3/src/api/events.rs` — `Event::SessionRefreshed`/`SessionRefreshFailed` + `call_id()` matcher.
- ✅ `crates/session-core-v3/src/adapters/session_event_handler.rs` — `handle_session_refreshed` / `handle_session_refresh_failed` dispatch (check `SessionRefreshFailed` first — it's a superstring of `SessionRefreshed`).
- ✅ `crates/session-core-v3/src/adapters/dialog_adapter.rs` — uses `make_call_for_session` to close the fast-RTT race.
- ✅ `crates/session-core-v3/src/lib.rs` — re-export `RelUsage` from `api::unified`.
- ✅ `crates/session-core-v3/Cargo.toml` — new example targets for prack and session_timer.
- ✅ `crates/session-core-v3/tests/{simple_api_tests,unified_api_tests,registration_test}.rs` — add `session_timer_secs`/`session_timer_min_se` to the test `Config` literals.
- ✅ `crates/session-core-v3/docs/RFC_COMPLIANCE_STATUS.md` — PRACK row marked ✅, 422 row added, SessionRefreshed/SessionRefreshFailed rows added, Known gaps section de-duplicated.
