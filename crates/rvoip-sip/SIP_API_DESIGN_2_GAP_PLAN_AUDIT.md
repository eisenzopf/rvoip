# SIP_API_DESIGN_2_GAP_PLAN — Implementation Audit

**Audit date:** 2026-05-11
**Plan under audit:** [`SIP_API_DESIGN_2_GAP_PLAN.md`](./SIP_API_DESIGN_2_GAP_PLAN.md)
**Scope:** Verify that the nine PRs (PR 1 – PR 9) closing gaps G1–G14 were
actually executed against the codebase, beyond what the implementing
developer self-disclosed.

---

## 0. Developer-disclosed gaps (not re-reported here)

The implementing developer flagged the following as known partial work.
These are excluded from the findings below:

- **PR 9 — §10 verification suite.** Only the PR 1 smoke test was authored.
  ~20 of the 24 named integration tests (incl.
  `b2bua_carry_through_integration`, `stash_lifecycle_integration`,
  `registrar_response_builder`, `cancel_safety_integration`) are missing.
- **PR 7 — coordinator entry rename.** `subscribe_event → subscribe` and
  `register_builder → register` was not performed; the renames collide
  with the still-present deprecated legacy names. Deferred to a future
  breaking release.

---

## 1. Verdict

The plan is **substantially under-delivered.** The new builder dispatch
path is wired through dialog-core, but the surrounding contract — header
policy validation, proxy routing, conflict guards, stash lifecycle,
auto-emit consultation, redaction — was not. Moreover, on the canonical
INVITE entry the new dispatch is bypassed entirely: `OutboundCallBuilder.send()`
falls back to the deprecated `make_call_*` methods, leaving the state-machine
action handler PR 5 "fixed" as dead code on the most important call path.

The wire output for ad-hoc tests (INVITE / MESSAGE / OPTIONS smoke) is
correct. The architecture the spec promised — *all twelve methods route
through `Action::Send*WithOptions` with policy validation, proxy merge,
conflict guard, and auth-retry-safe stash* — is not what shipped.

**Recommendation:** treat the work as PR 1 (wire-threading slice only)
+ PR 2 + PR 3 + PR 6 (callback half) + PR 7 (annotations only) + PR 8
delivered. PRs 4, 5, 9, plus the contract halves of PR 1 and PR 6,
remain open.

---

## 2. Findings (numbered, severity-ordered)

### Critical — surface wired, contract not implemented

#### F1 — PR 5: all 12 `Action::Send*WithOptions` handlers use `.take()` instead of `.clone()`

- **File:** `crates/rvoip-sip/src/state_machine/actions.rs`
- **Lines:** 1761, 1768, 1774, 1781, 1788, 1795, 1802, 1809, 1816, 1823, 1834, 1860
- **Plan reference:** PR 5 step 1; spec §7.3 invariant #2.
- **Plan said:** *"Note `clone()` not `take()`: the §7.3 stash invariant
  is `set-once, consumed-once at final response`, not action-dispatch...
  auth-retry needs the stash to persist across multiple dispatches."*
- **Observed:** every handler calls `session.pending_<method>_options.take()`.
- **Impact:** the second-order win PR 1 was designed to expose — auth-retry
  correctness — is silently broken. The 401 retry path discards all extras
  on the first dispatch.

#### F2 — PR 5 step 3: `SessionError::Conflict { method }` defined but never enforced

- **Variant declared at:** `crates/rvoip-sip/src/errors.rs:114`
- **Usage:** zero hits across `crates/rvoip-sip/src/`.
- **Plan said:** *"Auditing all 12 builders to add the guard in `.send()`."*
- **Observed:** the state-machine code at `state.rs:176` describes the
  missing guard in a comment, but no builder enforces it.
- **Impact:** §7.3 invariant #5 (single-in-flight per method) is
  unenforced — two concurrent `.bye().send()` calls will both dispatch.

#### F3 — PR 1 step 3: `HeaderPolicy::validate_outbound()` never called from `DialogAdapter` mirrors

- **File:** `crates/rvoip-sip/src/adapters/dialog_adapter.rs`
- **Mirror methods at lines:** 1249, 1269, 1290, 1310, 1330, 1349, 1371,
  1385, 1398, 1430, 1789
- **Plan said (PR 1 step 3 bullet 1):** *"Run
  `HeaderPolicy::validate_outbound(opts.method(), &extras)` before
  crossing into dialog-core (§5.4)."*
- **Observed:** `grep validate_outbound crates/rvoip-sip/src/adapters/dialog_adapter.rs`
  returns zero hits.
- **Impact:** the §5.4 load-bearing safety layer that protects against
  caller-injected method-shaped headers (`To`, `From`, `Via`, `CSeq`,
  `Call-ID`, etc.) is bypassed on the canonical builder dispatch.

#### F4 — PR 1 step 3: §6.1 outbound-proxy merge applied only to INVITE

- **File:** `crates/rvoip-sip/src/adapters/dialog_adapter.rs`
- **Only call site:** line 954 (INVITE path)
- **Plan said:** *"Apply the §6.1 merge table (proxy override prepends
  Route via existing `prepend_outbound_proxy_route`)"* for **all 12**
  `send_*_with_options` mirror methods.
- **Observed:** REFER, NOTIFY, INFO, BYE, UPDATE, re-INVITE, MESSAGE,
  SUBSCRIBE, OPTIONS, CANCEL all skip the prepend.
- **Impact:** per-leg outbound proxy routing is silently broken on 11
  of 12 methods — request goes direct rather than through the configured
  outbound proxy.

#### F5 — PR 1+5: `OutboundCallBuilder.send()` bypasses `Action::SendINVITEWithOptions` entirely

- **File:** `crates/rvoip-sip/src/api/send/outbound_call.rs:189-228`
- **Plan said (PR 1, PR 5):** the new INVITE builder routes through
  `Action::SendINVITEWithOptions`, which reads `pending_invite_options`.
- **Observed:** `.send()` dispatches to `coord.make_call_with_pai`,
  `coord.make_call_with_auth`, `coord.make_call_with_headers`,
  `coord.make_call` — all of which are now `#[deprecated]`. The comment
  at line 189 reads:
  > *"state-machine `Action::SendINVITEWithOptions` unification lands
  > as a Phase C follow-up"*
  This is a verbatim §6 DoD #2 forbidden phrase ("lands", "follow-up").
- **Impact:** the very integration PR 5 plumbs is **unreachable** on the
  INVITE path. `pending_invite_options` is never filled by the builder,
  so the state-machine handler is dead code regardless of the
  `take()`/`clone()` question in F1. The headline §11.2 B2BUA litmus
  test cannot work as designed.

### High

#### F6 — PR 4: `RegisterRefreshBuilder.send()` discards `expires` and staged extras

- **File:** `crates/rvoip-sip/src/api/send/register.rs:157-161`
- **Body of `.send()`:**
  ```rust
  let _ = self.expires;
  let _ = self.state;
  self.coord.refresh_registration(&self.handle).await
  ```
- **Plan said:** *"The builder constructs `RegisterRequestOptions {
  refresh: true, expires, credentials, extra_headers, ... }`"*
- **Observed:** `refresh_registration` fires a state-machine event with
  neither expires nor extras. `.with_expires(7200).send()` is a no-op
  beyond triggering a refresh at the default interval.

#### F7 — `TraceRedactor` (§12.4) infrastructure entirely absent

- **Search:** `grep -rn "TraceRedactor\|trace_redaction\|RedactionDecision" crates/rvoip-sip/src/`
  returns zero hits.
- **Plan said (PR 9 implementer note on test #31):** *"If the trait does
  not exist, this becomes a new sub-task: ship the trait +
  `RedactionDecision` + `Config.trace_redaction` field + consultation
  site."*
- **Observed:** no trait, no field, no consultation site.
- **Impact:** §12.4 trace-redaction guarantee unenforceable; PII / token
  values in extras pass through to logs.

#### F8 — `SubscribeRefreshBuilder.send()` still returns `Err(NotImplemented)`

- **File:** `crates/rvoip-sip/src/api/send/subscribe.rs:149-156`
- **Returns:** `Err(SessionError::NotImplemented("SubscribeRefreshBuilder.send() — manual refresh lands in Phase C follow-up"))`
- **Plan §6 DoD:** zero new `NotImplemented` stubs inside builder bodies.

#### F9 — §11.2 B2BUA litmus-test doctest does not compile

- **File:** `crates/rvoip-sip/src/lib.rs:353-378`
- **Failure:** `cargo test --doc -p rvoip-sip` →
  `no method named header found for struct rvoip_sip::IncomingCall`
  — missing `use rvoip_sip::SipHeaderView;` in the example.
- **Plan §6 DoD #5:** *"Pasting any §11.4 migration-table 'Tomorrow'
  example into a fresh `examples/` file compiles."* The crate's own
  rustdoc example for the litmus pattern is broken.

#### F10 — PR 6: warn-on-None instrumentation only on `TransferRequest`

- **File:** `crates/rvoip-sip-dialog/src/events/event_hub.rs`
- **Only instrumented site:** lines 616-621 (TransferRequest bridge).
- **Missing on:** NotifyReceived publish site (`protocol_handlers.rs:738`),
  InfoReceived, MessageReceived, OptionsReceived bridges (`event_hub.rs:799-841`).
- **Plan said (PR 6 step 3):** *"Add a `tracing::warn!` at the `Some`
  consumption sites if the field is ever observed as `None` so future
  regressions are loud rather than silent."*

### Medium

#### F11 — `NotifyRequestOptions.subscription_id` field declared but ignored

- **Declaration:** `crates/rvoip-sip-dialog/src/api/unified.rs:282`
- **Consumer:** `send_notify_with_options` at lines 1957-2025 never
  reads it.
- **Plan §4.4:** *"PR 1 must check this on its way through the NOTIFY
  options threading; if `subscription_id` is not yet plumbed, add it to
  `send_notify_with_options`'s implementation."*
- **Impact:** RFC 6665 multi-subscription dialogs route NOTIFY to the
  wrong subscription.

#### F12 — `Action::SendCANCEL` and `Action::SendNOTIFY` do not consult `Config.auto_emit_extra_headers`

- **Files:**
  - `crates/rvoip-sip/src/state_machine/actions.rs:629-638` (CANCEL)
  - `crates/rvoip-sip/src/state_machine/actions.rs:1481-1489` (NOTIFY)
- **CANCEL comment admits:** *"stay on the legacy path until that
  wire-stamping lands"*
- **Plan said:** *"the auto-BYE, auto-CANCEL, auto-NOTIFY emission sites
  consult it. Add the missing consultation if any in PR 1."* Only
  SendBYE consults.

#### F13 — `Action::SendBYE` does not honor §7.4 `pending_bye_options`-wins precedence

- **File:** `crates/rvoip-sip/src/state_machine/actions.rs:591-627`
- **Comment in code:**
  > *"application-staged `pending_bye_options` (filled by
  > `coord.bye(..).send()`) wins and skips the auto_emit pre-load; that
  > wiring lands with the Phase C stash work and is a no-op until then."*
- **Plan §6 DoD #2:** forbids the phrase "wiring lands" in
  `*_with_options` or builder bodies.

#### F14 — One `raw_request: None` publish site remains in dialog-core

- **File:** `crates/rvoip-sip-dialog/src/events/adapter.rs:286` (legacy
  IncomingCall publish path).
- **Plan PR 6 acceptance:**
  > `git grep "raw_request: None" crates/rvoip-sip-dialog/src/`
  > **returns zero hits.**
- **Observed:** returns 1 hit. Acceptance criterion not met.

#### F15 — `RegisterRequestOptions.refresh` field has no semantic effect in dialog-core

- **Search:** `grep -rn "options.refresh\|opts.refresh"` in
  `crates/rvoip-sip-dialog/src/` returns zero hits.
- **Plan §7.1:** *"All twelve outbound methods route through
  `Action::Send*WithOptions`"* with `refresh: true` differentiating
  initial vs refresh REGISTER.
- **Observed:** initial REGISTER and refresh REGISTER share the same
  dialog-core path; the `refresh` flag is set by the builder but ignored
  downstream.

#### F16 — PR 1 smoke test covers 3 builders, not 6

- **File:** `crates/rvoip-sip/tests/outbound_request_builders_integration.rs`
- **Authored:** INVITE, MESSAGE, OPTIONS (all out-of-dialog).
- **Plan said (PR 1 step 5):** *"covering at minimum INVITE, REFER,
  NOTIFY, INFO, BYE, MESSAGE each emitting an asserted-on-wire `X-Test:
  smoke` header. The remaining 6 builders + full §10 #9 coverage land
  in PR 9."*
- **Observed:** the in-dialog 4 (REFER, NOTIFY, INFO, BYE) are missing
  from the smoke slice. The test file header concedes the deferral.
- **Impact:** the smoke gate that "G1 actually closed" is half-covered.

#### F17 — PR 8: `multipart_mixed`, `multipart_parse`, `MultipartParseError` entirely absent

- **Search:** `grep -rn "multipart_mixed\|multipart_parse\|MultipartParseError" crates/rvoip-sip/src/`
  returns zero hits.
- **Plan PR 8 step 2:** required in `crates/rvoip-sip/src/api/headers/convenience.rs`.
- **Impact:** §10 test #24 (`multipart_body_integration.rs`) cannot pass
  even when authored.

#### F18 — Additional "Phase C follow-up" comments inside builder bodies

- **Files:**
  - `crates/rvoip-sip/src/api/send/outbound_call.rs:189` —
    *"lands as a Phase C follow-up"*
  - `crates/rvoip-sip/src/api/send/refer.rs:58` —
    *"formatting lives in the Phase C follow-up"*
- **Plan §6 DoD #2:** forbids "follow-up", "staged for", "wiring lands",
  "pending" inside `*_with_options` or builder bodies.

### Low

#### F19 — PR 1 step 4: audit-comment block not deleted

- **File:** `crates/rvoip-sip-dialog/src/api/unified.rs:1872-1883`
- **Plan said:** *"delete the explanatory comment block at
  `crates/rvoip-sip-dialog/src/api/unified.rs:1764-1775` once the
  threading is real. Comments that document a broken state become stale
  lies the moment the state is fixed."*
- **Observed:** comment block still present (drifted from line 1764 to
  the current location during edits, but not removed).

#### F20 — `IncomingRequest::with_request` is dead code (new warning)

- **File:** `crates/rvoip-sip/src/api/incoming.rs:1030`
- **`cargo build -p rvoip-sip`:** `warning: associated function with_request is never used`
- **Plan PR 6 acceptance:** zero new dead_code warnings on inbound
  surfaces.

#### F21 — PR 7 step 3: CI grep guard / compiletest annotation check missing

- **Search:** no `tests/deprecation_table.rs` or grep-CI script.
- **Plan said:** *"add a CI grep guard (or a `compiletest` check) that
  asserts each deprecated method from the table is annotated."*

---

## 3. What is correctly implemented

For balance — these PR slices ARE done:

- **PR 2 (G5):** `IncomingRegister` wiring at
  `crates/rvoip-sip-dialog/src/protocol/register_handler.rs:93-131` and
  `crates/rvoip-sip/src/adapters/session_event_handler.rs:393-461`.
- **PR 3 (G7):** `GenericResponseBuilder::method()` threading
  (`crates/rvoip-sip/src/api/respond/generic.rs:21-39`) and all three
  call sites correct.
- **PR 6 callback half (G8):** dispatch in
  `crates/rvoip-sip/src/api/callback_peer.rs:1913, 2048, 2141`;
  `on_transfer_request` and `on_notify` correctly marked
  `#[deprecated(since = "0.3.0", ...)]` at lines 893 and 983.
- **PR 6 G12:** `SessionCoordinationEvent::TransferRequest.raw_request`
  added at `session_coordination.rs:250`; publish site populates
  (`protocol_handlers.rs:429-438`); cross-crate bridge in
  `event_hub.rs:590-640` carries through.
- **PR 7 deprecation annotations:** the §9 Phase C matrix is applied to
  `UnifiedCoordinator`, `PeerControl`, `CallbackPeerControl`,
  `Endpoint` / `EndpointControl`; `make_transfer_leg` correctly NOT
  deprecated.
- **PR 8 G10/G14:** `crates/rvoip-sip/src/api/bodies.rs` exists with
  `sdp`, `dtmf_relay`, `pidf_xml`, `simple_message_summary`, `isup_l3`;
  crate-root re-exports at `lib.rs:476-490` cover the prescribed surface.
- **`SessionError::Conflict { method }` variant defined** at
  `errors.rs:114` (though never enforced — see F2).
- **OPTIONS authorship:**
  `dialog_quick::options_out_of_dialog_with_extras` is implemented as
  claimed.

---

## 4. Headline takeaway

The wire output for the smoke methods is correct. The architectural
contract around it is not:

| Contract piece | Status |
|---|---|
| Extras reach the wire on the 3 smoke-tested methods | ✅ done |
| Extras reach the wire on the other 9 methods | ⚠️ threaded but unvalidated (F3, F4) |
| HeaderPolicy validation at the boundary | ❌ missing (F3) |
| Outbound proxy merge per §6.1 | ❌ INVITE only (F4) |
| Single-in-flight conflict guard | ❌ variant defined but unused (F2) |
| Auth-retry preserves extras (`clone` vs `take`) | ❌ all 12 broken (F1) |
| INVITE builder routes through `Action::SendINVITEWithOptions` | ❌ falls back to deprecated path (F5) |
| Auto-emit consultation on BYE/CANCEL/NOTIFY | ❌ BYE only, partially (F12, F13) |
| RegisterRefresh expires + extras | ❌ discarded (F6) |
| Subscribe refresh send | ❌ `NotImplemented` (F8) |
| TraceRedactor §12.4 | ❌ entirely absent (F7) |
| §11.2 litmus doctest compiles | ❌ broken (F9) |
| §10 test suite (24 tests) | ⚠️ developer-disclosed |
| Coordinator entry rename | ⚠️ developer-disclosed |

The five Critical findings (F1–F5) are tightly coupled: PR 5's stash
handler is dead code on INVITE because PR 1's INVITE builder doesn't
fill the stash (F5); when the builder is rewired (F5), PR 5's `take()`
behavior (F1) will silently drop extras on auth-retry; the new dispatch
that does get used bypasses `HeaderPolicy::validate_outbound` (F3) and
`prepend_outbound_proxy_route` (F4); and the single-in-flight invariant
(F2) is unenforced across all 12 builders. These five must be closed
together to make the §10 acceptance tests viable.

---

## 5. Definition-of-done check (plan §6)

| # | Gate | Status |
|---|---|---|
| 1 | Zero new `dead_code` warnings on new surfaces | ❌ (F20) |
| 2 | Zero `follow-up\|staged for\|wiring lands\|pending` hits in `*_with_options` / builder bodies | ❌ (F5, F13, F18) |
| 3 | All 24 §10 integration tests exist and pass | ❌ (developer-disclosed) |
| 4 | 60+ doctests pass | ❌ (F9) |
| 5 | §11.4 migration "Tomorrow" examples compile verbatim | ❌ (F9; PR 7 rename also blocks) |
| 6 | Manual gate: 5-section Gateway/B2BUA/SBC rustdoc block | Not verified |
| 7 | Each 🔴/⚠️ finding has a linked PR marked `Closes: G<n>` | Not verified |

**Zero of seven gates pass.** Per the plan's own definition, this is
not "finishing the design."

---

## 6. Suggested remediation order

Two engineer-weeks of follow-up work, in this order, would close the
critical contract gap:

1. **F5 first** — rewire `OutboundCallBuilder.send()` to fill
   `pending_invite_options` and dispatch via `Action::SendINVITEWithOptions`.
   Without this, F1 cannot be tested.
2. **F1 + F2 together** — switch all 12 `take()` to `clone()` with
   clear-at-response-resolution; add the conflict guard to all 12
   builders. Ship `SessionError::Conflict` enforcement.
3. **F3 + F4 together** — add `HeaderPolicy::validate_outbound` and
   `prepend_outbound_proxy_route` to all 11 missing
   `send_*_with_options` mirrors in `dialog_adapter.rs`.
4. **F6 + F8** — wire `RegisterRefreshBuilder.send()` and
   `SubscribeRefreshBuilder.send()` end-to-end.
5. **F12 + F13** — auto-emit consultation on SendCANCEL and SendNOTIFY;
   stash-precedence honoring on SendBYE.
6. **F11 + F15** — plumb `subscription_id` and `refresh` through
   dialog-core so the field declarations stop being decorative.
7. **F7** — ship `TraceRedactor` + `RedactionDecision` +
   `Config.trace_redaction` + consultation site at `DialogAdapter`.
8. **F9, F19, F20, F21** — small cleanups (doc fix, comment delete,
   dead-code purge, CI guard).
9. **F10, F14, F18** — instrumentation and comment hygiene.
10. **F16, F17** — extend smoke test to 6 builders; ship multipart
    helpers.

Only after items 1–4 are done will the §10 acceptance tests that the
developer deferred to PR 9 be meaningfully gating. Authoring those
tests before items 1–4 land would produce a suite that fails for
the right reasons but blocks PR merges; authoring them after would
gate against true regression.
