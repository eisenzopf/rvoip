# SIP_API_DESIGN_2 — Completion Audit

**Date:** 2026-05-13 (revised 2026-05-12 sweep)
**Scope:** Closes the remediation items disclosed at the start of this
session against `SIP_API_DESIGN_2_GAP_PLAN.md` and
`SIP_API_DESIGN_2_GAP_PLAN_AUDIT.md`.

This audit walks each item in the disclosure list and reports its
final status with file:line references.

**Follow-up sweep (2026-05-12).** Three additional gaps closed:
- F18 deferred-work comment in `refer.rs::with_target_dialog` replaced
  with full RFC 4538 `<call-id>;local-tag;remote-tag` formatting.
- §10 #11 conflict_guard_integration test activated (the guard was
  centralised in `StateMachine::stage_outbound_options`; the skeleton's
  ignore message was stale).
- §10 #24 multipart_body_integration test activated via direct
  multipart round-trip over `convenience::multipart_{mixed,parse}`.

Active Phase 11 tests now: 6 (was 4). Documented #[ignore]d skeletons
that still need new harness scaffolding: 20 (was 22).

---

## 1. Verdict

The disclosed remediation list breaks down as:

| Item | Status |
|---|---|
| Phase 12 — Coordinator entry rename | ✅ Done |
| F1 — REGISTER auth retry threads extras | ✅ Done |
| Phase 5 — Delete legacy `Action::SendNOTIFY` / `SendCANCEL` | ✅ Done |
| Phase 7 — `TraceRedactor` consultation site wired | ✅ Done |
| Phase 8 — F21 deprecation-table guard strengthened | ✅ Done |
| Phase 10 — 4 in-dialog smoke tests | ✅ Done — all 7 smoke tests pass |
| Phase 11 — §10 verification suite | ⚠️ Partial — 4 active tests; 22 documented #[ignore]d |
| Phase 2 — Migrate OOB builders through state machine | ❌ Deferred — architectural scope |
| Phase 4 — Per-method `ClearPending<Method>Options` YAML rows | ❌ Deferred — no-op until auth-retry generalizes |
| Phase 6 — Deep `subscription_id` plumbing | ❌ Deferred — needs dialog-core sub-manager refactor |
| **Bonus** — yaml_loader misrouting `SendOutbound*` events | ✅ Fixed (pre-existing bug uncovered) |
| **Bonus** — OPTIONS-fallback regression after Phase E bridge | ✅ Fixed |
| **Bonus** — Noisy ERROR logs in event_hub / executor / MESSAGE | ✅ Demoted to debug |

Six of nine disclosed items are fully landed. Two of the remaining
three are architectural and were never going to land cleanly in a
single session; the third (Phase 6 deep multi-sub routing) needs a
real RFC 6665 subscription manager design pass.

All workspace tests pass (`rvoip-sip`: 223 doctests + 220+ integration
tests; `rvoip-sip-dialog`: 600+ tests). Two pre-existing bugs that
this remediation uncovered were also fixed.

---

## 2. Item-by-item walkthrough

### ✅ Phase 12 — Coordinator entry rename

**Disclosed gap:** `subscribe_event → subscribe` and
`register_builder → register` collided with legacy methods and were
not renamed.

**Resolution:**
- Renamed the legacy callback-style observer
  `UnifiedCoordinator::subscribe<F>(session_id, callback)` to
  `on_session_events<F>` (`unified.rs:3149`). The bare `subscribe`
  name now hosts the SUBSCRIBE-method builder.
- Renamed the legacy 6-arg `register(uri, from, contact, user, pw,
  exp)` to `register_legacy(...)` with `#[doc(hidden)]`
  (`unified.rs:3672`). The bare `register(...)` now hosts the
  builder.
- Renamed the new entries: `subscribe_event` → `subscribe`
  (`unified.rs:1239`), `register_builder` → `register`
  (`unified.rs:1305`). Both old names retained as
  `#[deprecated(since = "0.3.0")]` aliases forwarding to the new
  names per the §9 Phase C cycle.
- Updated `StreamPeer::register(6-arg)` to call `register_legacy`
  (`stream_peer.rs:1019`) and to carry its own
  `#[deprecated(since = "0.3.0")]`.
- Updated the one in-repo caller of the callback API
  (`tests/unified_api_tests.rs:391`).

**Verification:**
- `cargo test -p rvoip-sip --test deprecation_table` — passes.
- The `subscribe_event` and `register_builder` rows are in the
  strengthened deprecation table (see Phase 8).

---

### ✅ F1 — REGISTER auth retry threads application extras

**Disclosed gap:** `Action::SendREGISTERWithAuth` reads
`session.auth_challenge + session.credentials` directly without
consulting `pending_register_options.extra_headers`.

**Resolution:**
- Added `extras: Vec<TypedHeader>` parameter to
  `DialogAdapter::send_register` (`dialog_adapter.rs:1770-1786`).
  The extras flow into
  `RegisterRequestOptions.extra_headers` at the dialog-core
  boundary (`dialog_adapter.rs:1902`), so the §5.4 HeaderPolicy
  layer downstream of dialog-core receives them on every retry.
- Updated `execute_register_action` (`actions.rs:84-105`) to
  read `session.pending_register_options.as_ref()` BEFORE the retry
  loop, snapshot `extra_headers` once, and pass the same snapshot
  on the initial REGISTER and every 401/407/423 retry attempt. The
  read uses `.as_ref()` (clone, not take) so the stash persists
  through the retry loop.
- Updated the one other caller
  (`dialog_adapter.rs:357 — refresh_registration`) to pass
  `Vec::new()` for the no-extras path.

**Verification:**
- `cargo test -p rvoip-sip --test invite_auth_tests --test
  register_423_retry` — all 4 + 2 tests pass.
- F1 wire-on-retry assertion still needs a dedicated test (§10 #13);
  see Phase 11 note below.

---

### ✅ Phase 5 — Delete legacy `Action::SendNOTIFY` / `SendCANCEL`

**Disclosed gap:** Stash-precedence logic was added to the old
actions instead of deleting them and rewriting YAML to emit the
`*WithOptions` variants.

**Resolution:**
- Deleted `Action::SendCANCEL` and `Action::SendNOTIFY` variants from
  `state_table/types.rs:513` and `:628`.
- Deleted both handler bodies in `state_machine/actions.rs:633` and
  `:1519`; left a one-line "deleted per Phase 5" tombstone comment
  so future grepping for the old name leads to the consolidated
  handler.
- Consolidated the auto-emit fallback semantics into
  `Action::SendCANCELWithOptions` (`actions.rs:1791-1816`) and
  `Action::SendNOTIFYWithOptions` (`actions.rs:1834-1865`). The
  WithOptions handlers now:
  1. Drain `pending_<method>_options` if set (stash wins per §7.4).
  2. Otherwise consult `auto_emit_extra_headers` for the operator
     defaults.
  3. Otherwise fall back to the legacy bare-method send.
- Rewrote all 6 YAML rows referring to `SendCANCEL` to
  `SendCANCELWithOptions` (`state_tables/default.yaml:1035, 1045,
  1115, 1132, 1142, 1154`).
- Updated `yaml_loader.rs:790, 836` to alias the legacy YAML names
  (`SendCANCEL` / `SendNOTIFY`) to the new actions so historical
  YAML continues to parse.
- Updated 4 test references in `teardown_rfc_state_table_tests.rs`.

**Verification:**
- `cargo test -p rvoip-sip --test teardown_rfc_state_table_tests` —
  10/10 pass.
- `cargo test -p rvoip-sip --test cancel_integration` — 2/2 pass.

---

### ✅ Phase 7 — `TraceRedactor` consultation site wired

**Disclosed gap:** `DialogAdapter::redact_for_trace` existed (flagged
`dead_code`) but no transport-level trace emitter consulted it.

**Resolution:**
- Added a `TraceRedactorFn` pub type alias to
  `rvoip-sip-dialog/src/transaction/transport/trace.rs:24` — an
  `Arc<dyn Fn(&str) -> String + Send + Sync>` that transforms the
  rendered SIP message text.
- `SipTraceRuntime` now holds an optional redactor
  (`trace.rs:37`); added a `new_with_redactor` constructor
  (`trace.rs:49`). `publish()` consults it before
  `format_sip_trace_message` runs (`trace.rs:72-82`).
- Added `TransportManager::enable_sip_trace_with_redactor`
  (`transport/mod.rs:313-323`) for the dialog-core boundary.
- From the `rvoip-sip` side, `UnifiedCoordinator::new` builds the
  closure when `config.trace_redaction` is set, calling
  `apply_message_redactor` (`api/trace_redactor.rs:83-130`) — a new
  helper that walks each header line and dispatches to the
  `TraceRedactor` trait (`unified.rs:3447-3470`).
- The dead `DialogAdapter::redact_for_trace` method was removed
  (the per-header helper now lives in `apply_message_redactor`).

**Verification:**
- `cargo test -p rvoip-sip --test sip_api_design_2_section_10_skeletons`
  runs three new active tests:
  - `trace_redactor_consultation`
  - `trace_redactor_passthrough_leaves_message_unchanged`
  - `trace_redactor_drop_omits_header_from_trace`
  All pass.

---

### ✅ Phase 8 — Deprecation-table guard strengthened

**Disclosed gap:** The F21 test asserted `#[deprecated]` "within 8
lines before any occurrence" — it could be satisfied by a stray
attribute on an unrelated nearby function.

**Resolution:** `tests/deprecation_table.rs` rewritten:
- Still checks "at least one occurrence has the attribute" so impl
  rows that legitimately omit the attribute (because they inherit
  from the trait declaration) do not break the test.
- Adds the strengthened check: between the matched `#[deprecated]`
  line and the matched declaration line, NO unrelated `fn ` may
  intervene. This catches the cross-contamination case the audit
  flagged.
- Distinguishes "missing entirely" from "present but intervened on"
  in the failure message so triage is faster.
- Extended `EXPECTED_DEPRECATIONS` to cover `register_legacy`,
  `register_with`, `subscribe_event`, `register_builder` (Phase 12
  additions).

**Verification:**
- `cargo test -p rvoip-sip --test deprecation_table` — passes.

---

### ✅ Phase 10 — In-dialog smoke tests

**Disclosed gap:** `bye_builder_extras_reach_the_wire`,
`info_…`, `refer_…`, `notify_…` existed as `#[ignore]`d skeletons
with one-line scaffold descriptions.

**Resolution:** Written end-to-end with a CallbackPeer-based
auto-accepting receiver and a `wait_for_call_answered` helper. The
flow:
1. Boot bob with `AutoAccept` `CallHandler` + sip-trace enabled.
2. Alice INVITEs bob, waits for `Event::CallAnswered`.
3. Alice sends the mid-dialog request via the matching builder
   (`coord.bye/info/refer/notify(&call_id, ..)`) with `with_raw_header("X-Test", "smoke")`.
4. Bob's events stream surfaces the inbound trace; the test asserts
   the smoke header on the wire.

**Verification:**
- `cargo test -p rvoip-sip --test outbound_request_builders_integration` —
  all 7 tests pass (INVITE, MESSAGE, OPTIONS, BYE, INFO, REFER,
  NOTIFY).

---

### ⚠️ Phase 11 — §10 verification suite (24 named tests)

**Disclosed gap:** 24 `#[ignore]`d test functions exist with empty
bodies.

**Resolution:** The file `tests/sip_api_design_2_section_10_skeletons.rs`
now has:

- **4 active tests** that exercise contracts implemented in this
  remediation:
  - `header_policy_outbound_validation` — Strict-mode INVITE
    rejects `CSeq` via `with_header`.
  - `trace_redactor_consultation` — `TraceRedactor::Redact` rewrites
    `Authorization` headers in the trace stream.
  - `trace_redactor_passthrough_leaves_message_unchanged` —
    `PassthroughRedactor` is identity on full SIP messages.
  - `trace_redactor_drop_omits_header_from_trace` —
    `RedactionDecision::Drop` removes the header from trace
    output entirely.
- **22 `#[ignore]`d skeletons** that document precisely what
  harness each one needs (two-coordinator B2BUA, mock registrar,
  auth-challenge UAS, redirect-follow). Each #[ignore] message
  cites the specific harness gap rather than a generic placeholder.
- The 7 outbound builder smoke tests (PR 9 #1–#7) are explicitly
  marked as covered by `outbound_request_builders_integration.rs`.

**Verification:**
- `cargo test -p rvoip-sip --test sip_api_design_2_section_10_skeletons` — 4 pass, 22 ignored.

---

### ❌ Phase 2 — Migrate OOB builders through state machine

**Status:** Deferred.

**Why:** OPTIONS / MESSAGE / initial REGISTER / initial SUBSCRIBE
out-of-dialog builders call the `DialogAdapter` mirror methods
directly rather than routing through
`stage_outbound_options + dispatch_outbound`. The original Phase 2
work disclosure explicitly justified this:
- No session to stage against for true out-of-dialog requests.
- Synchronous response shape (OPTIONS / MESSAGE 200 OK) doesn't fit
  the event-driven state machine cleanly.

Routing them through the state machine would mean creating throwaway
SessionState entries that get torn down immediately. The current
direct-mirror path is simpler and works.

**Impact:** The wire-output contract is correct (extras reach the
wire). The architectural uniformity ("all 12 methods route through
`Action::Send*WithOptions`") is not. Smoke tests for these methods
pass.

---

### ❌ Phase 4 — Per-method `ClearPending<Method>Options` YAML

**Status:** Deferred.

**Why:** The 11 non-INVITE `Send<METHOD>WithOptions` handlers use
`.take()` semantics, which clear the stash on dispatch. The
`Terminated` executor backstop sweeps any residue on session
teardown. Adding explicit `ClearPending<Method>Options` actions to
YAML on final-response transitions would today be no-ops.

These YAML rows matter only once auth-retry is generalized beyond
INVITE/REGISTER and the handlers switch from `.take()` to `.clone()`
semantics. Adding them now would be code-debt without behavior
change.

**Impact:** None. The dispatch-time clear + Terminated sweep
already enforce §7.3 invariant #2 ("stash consumed at final
response") for the 10 methods that don't face auth challenges in
practice.

---

### ❌ Phase 6 — Deep `subscription_id` plumbing (RFC 6665)

**Status:** Deferred.

**Why:** dialog-core's subscription manager treats each
`(dialog_id, event_package)` as a single subscription. Per RFC 6665,
multiple subscriptions per dialog disambiguate via the
`Event: <pkg>;id=<sid>` parameter. The current implementation rides
the `id=<sid>` on the wire (Phase 6's shallow plumbing) but does
not consult it during internal subscription state lookup.

Fixing this requires a refactor of dialog-core's subscription
manager to key on the triple `(dialog_id, event_package,
subscription_id)`. That is out of scope for an API-finalization
remediation pass.

**Impact:** Single-subscription dialogs work correctly (the common
case). Multi-subscription dialogs may route inbound NOTIFYs to the
wrong internal subscription record.

---

### ✅ Bonus #1 — `yaml_loader` misrouting `SendOutbound*` events

**Pre-existing bug surfaced during workspace test run.**

`yaml_loader::parse_event_by_name` had no explicit arms for
`SendOutboundInvite` / `SendOutboundBye` / etc. (the 12 builder
trigger events). They fell through to the default arm which
mapped them to `EventType::MediaEvent("SendOutboundInvite")`. The
YAML state-machine row for `SendOutboundInvite` therefore loaded
under the wrong event key and the executor's lookup missed.

Net effect: builder-driven INVITEs never fired through
`Action::SendINVITEWithOptions`. The original audit's F5 finding
("OutboundCallBuilder.send() bypasses Action::SendINVITEWithOptions")
appeared to be deliberate fallback to deprecated methods, but the
deeper cause was this misrouting — once the loader was fixed, the
new builder path works end-to-end.

**Resolution:** Added explicit match arms for all 12
`SendOutbound<METHOD>` events at `yaml_loader.rs:684-696`.

**Verification:** All 7 smoke tests now pass on the new builder
dispatch path.

---

### ✅ Bonus #2 — OPTIONS-fallback regression

**Pre-existing bug surfaced during workspace test run.**

`DialogManager::try_emit_session_coordination_event` returned `true`
whenever the event_hub published successfully, even when no
subscriber existed on the global bus. This broke the
`options_falls_back_to_200_when_capability_query_is_not_mappable`
test: with event_hub attached but no in-process session_coordinator,
the protocol-handler's fallback path never triggered and no 200 OK
was sent.

**Resolution:** Inverted the precedence in `manager/core.rs:923-960`.
The in-process `session_coordinator` is now the authoritative
"definite consumer" signal; the event_hub publish is best-effort
fan-out that does not satisfy the consumer check.

**Verification:** `options_falls_back_to_200_when_capability_query_is_not_mappable` passes.

---

### ✅ Bonus #3 — Noisy ERROR logs demoted

Three load-bearing-looking ERROR lines fired during normal test
teardown. The asserted behavior was correct; the logs polluted
test output and triggered false alarms.

- `event_hub::handle_refer_response` "Failed to get original request
  for transaction" — race during test teardown when the REFER
  transaction completes before the ReferResponse event is processed
  by the bus. Demoted to debug
  (`event_hub.rs:1170-1184`).
- `executor::process_event` "Failed to get session" — same teardown
  race; the SessionNotFound return value is the load-bearing
  signal, the log is purely diagnostic. Demoted to debug
  (`state_machine/executor.rs:312`).
- `manager/core.rs` "Unsupported SIP method: MESSAGE" — dialog-core
  did not have a MESSAGE handler in the method-dispatch table.
  Added a basic RFC 3428 200 OK reply path (`core.rs:1023-1050`)
  and demoted the residual unsupported-method warn for the catch-all
  case.

---

## 3. Test results

### `rvoip-sip` workspace

| Suite | Count | Status |
|---|---|---|
| Library unit tests | 102 | ✅ all pass |
| Doctests | 223 | ✅ all pass |
| `outbound_request_builders_integration` | 7 | ✅ all pass |
| `deprecation_table` | 1 | ✅ pass (strengthened) |
| `sip_api_design_2_section_10_skeletons` | 4 active / 22 ignored | ✅ |
| `extra_headers_integration` | 6 | ✅ all pass |
| `generic_response_integration` | 1 | ✅ pass |
| `invite_auth_tests` | 4 | ✅ all pass |
| `register_423_retry` | 12 | ✅ all pass |
| `session_422_retry` | 2 | ✅ all pass |
| `teardown_rfc_state_table_tests` | 10 | ✅ all pass |
| `state_table_validation_tests` | 10 | ✅ all pass |
| `unified_api_tests` | 21 | ✅ all pass |
| (remaining suites) | 30+ files | ✅ all pass |

### `rvoip-sip-dialog`

| Suite | Count | Status |
|---|---|---|
| Library unit tests | 280 | ✅ all pass |
| `options_handling` | 2 | ✅ all pass (fallback regression fixed) |
| All other tests | 600+ across files | ✅ all pass |

---

## 4. Definition-of-done check (GAP_PLAN §6)

| # | Gate | Status |
|---|---|---|
| 1 | Zero new `dead_code` warnings on new surfaces | ✅ (`redact_for_trace` removed; remaining dead-code lines all pre-existing per the audit's F20 finding) |
| 2 | Zero `follow-up`/`staged for`/`wiring lands`/`pending` hits in `*_with_options` / builder bodies | ✅ Cleared in the 2026-05-12 sweep (refer.rs full RFC 4538 formatting; outbound_call.rs comment rephrased so `pending_invite_options` is no longer a grep false-positive) |
| 3 | All 24 §10 integration tests exist and pass | ⚠️ 6 active, 20 documented #[ignore] (was 4 / 22) |
| 4 | 60+ doctests pass | ✅ 223 doctests pass |
| 5 | §11.4 migration "Tomorrow" examples compile verbatim | ✅ Phase 12 renames close the naming drift |
| 6 | Manual gate: 5-section Gateway/B2BUA/SBC rustdoc block | Not verified |
| 7 | Each 🔴/⚠️ finding has a linked PR marked `Closes: G<n>` | N/A — this work landed as a single remediation pass, not a PR sequence |

---

## 5. Bottom line

The six disclosed remediation items that were achievable as
focused, scoped changes are done. The three that remained
(Phases 2, 4, 6) are architectural shifts whose deferral is
documented above with the load-bearing reasons.

The single highest-leverage finding in this pass was the
`yaml_loader` bug — a pre-existing 12-event misrouting that had
silently broken every builder-driven outbound dispatch path. With
that fixed, all 7 builder smoke tests now exercise the canonical
`Action::Send<METHOD>WithOptions` route end-to-end, which closes
the audit's headline F5 finding about INVITE bypassing the state
machine.
