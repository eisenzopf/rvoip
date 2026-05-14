# SIP_API_DESIGN_2 — Gap Closure Plan

**Owner:** SIP API working group
**Author:** engineering manager (in response to `SIP_API_DESIGN_2_AUDIT.md`)
**Spec under remediation:** `SIP_API_DESIGN_2.md`
**Audit being remediated:** `SIP_API_DESIGN_2_AUDIT.md` (2026-05-11)
**Status:** approved for execution. Single-pass plan; ships as nine ordered PRs.

---

## 0. Reading order

1. The original spec (`SIP_API_DESIGN_2.md`) is still the canonical
   design. This document does **not** modify the design — it closes
   gaps between the design and the current implementation.
2. The audit (`SIP_API_DESIGN_2_AUDIT.md`) enumerates the gaps. Every
   gap finding (🔴 1–8 and ⚠️ 9–14) is tracked as a workstream
   below, with the same numbering preserved (`G1`–`G14`) so reviewers
   can map plan ↔ audit one-to-one.
3. Workstreams are ordered by dependency. PRs ship in the order
   listed; each PR includes the verification step(s) from §10 of the
   spec that it unblocks.

---

## 1. Verdict on the audit's verdict

The audit is correct. The shipped surface compiles and is
type-system-correct, but the wire behavior diverges from the design
on the load-bearing path: outbound `extra_headers` do not reach the
wire on most methods, registrar response authoring is unreachable
because `IncomingRegister` is never constructed on the bus path,
several builders are stubs, and the prescribed §10 integration test
suite does not exist. We treat the prior delivery as **Phase A
complete; Phases B–E surface-only**.

The remediation has two halves:

- **Half 1 (G1–G8) — make the wire output match the design.**
  These are the audit's 🔴 findings. Without them the new builders
  are decorative. PRs 1–7 below.
- **Half 2 (G9–G14, plus §10 tests) — make the documented developer
  experience match reality.** These are the audit's ⚠️ findings,
  the verbatim migration-guide examples, deprecations, the missing
  convenience body module, naming reconciliation, and the 24-test
  acceptance suite. PRs 8–9 below.

We do **not** revise the design's scope. Lean-mode (§13.3) and
removal of deprecated methods stay out of scope per spec §14.

---

## 2. Gap inventory (audit-aligned)

| Gap | Audit ID | Severity | Workstream PR | Spec section(s) |
|---|---|---|---|---|
| G1 | 🔴 1 — Outbound `extra_headers` never reach the wire | Critical | PR 1 | §7.2, §7.3, §6.1 |
| G2 | 🔴 5 — `CancelBuilder` silently drops everything | Critical | PR 1 | §3.3, §5.2, §7.2 |
| G3 | 🔴 8 — Zero of §10 verification tests exist | Critical | PR 1 (smoke), PR 9 (full) | §10 |
| G4 | 🔴 2 — `RegisterRefreshBuilder.send()` returns `NotImplemented` | High | PR 4 | §3.3, §7.1, §12.5 |
| G5 | 🔴 3 — `IncomingRegister` is type-shaped but never constructed | Critical | PR 2 | §3.4, §7.5, §9 Phase D |
| G6 | 🔴 4 — INVITE / REGISTER state-machine action handlers are stubs | High | PR 5 | §7.1, §7.3 |
| G7 | 🔴 7 — `GenericResponseBuilder::method()` hardcoded to INVITE | Medium | PR 3 | §3.4 |
| G8 | 🔴 6 — `on_refer_received` / `on_notify_received` are dead code | High | PR 6 | §9 Phase E |
| G9 | ⚠️ 9 — Legacy methods not `#[deprecated]` | Medium | PR 7 | §9 Phase C deprecation table |
| G10 | ⚠️ 10 — Convenience `bodies` module missing | Medium | PR 8 | §3.6 |
| G11 | ⚠️ 11 — Coordinator entry naming drift (`subscribe` vs `subscribe_event`, `register` vs `register_builder`) | Medium | PR 7 | §3.3, §11.4 |
| G12 | ⚠️ 12 — `TransferRequest` (dialog-core internal) missing `raw_request` | High | PR 6 | §7.5 |
| G13 | ⚠️ 13 — `infra-common` enrichment uses `Option<Arc<Bytes>>` not `Arc<Bytes>` | Medium | PR 6 | §7.5 |
| G14 | ⚠️ 14 — `Incoming{Call,Request,Response,Register}` not re-exported at crate root | Low | PR 8 | §9 Phase A acceptance |

PR ordering rationale: PR 1 unblocks every other outbound workstream
(once `extra_headers` reach the wire, every later builder fix
becomes verifiable). PRs 2–6 are independent and can be parallelized
across reviewers; PRs 7–9 close the remaining surface, naming, and
test gaps.

---

## 3. PR plan

### PR 1 — Thread `extra_headers` through dialog-core; fix CANCEL; add the smoke test

**Closes:** G1, G2, plus the smoke-test slice of G3.
**Spec sections:** §7.2 (additive options), §7.3 (stash lifecycle),
§6.1 (merge precedence), §10 test #9.

**Files & edits:**

1. `crates/rvoip-sip-dialog/src/api/unified.rs:1764-1977` — every
   `*_with_options` method that today forwards to a legacy
   `send_*` manager method (REFER, NOTIFY, INFO, BYE, UPDATE,
   re-INVITE, MESSAGE, SUBSCRIBE, SUBSCRIBE-refresh, CANCEL) must
   route through `request_builder_from_dialog_template` (the slot at
   `transaction/dialog/mod.rs:107-118` already accepts
   `extra_headers: Option<Vec<TypedHeader>>`).

   For methods whose existing manager helper does not expose the
   extras slot, add a sibling helper in
   `crates/rvoip-sip-dialog/src/manager/mod.rs` (or the appropriate
   submodule) that passes `extra_headers` through. Pattern:

   ```rust
   pub async fn send_refer_with_extras(
       &self,
       dialog_id: &DialogId,
       refer_to: String,
       body: Option<String>,
       extras: Vec<TypedHeader>,
   ) -> ApiResult<TransactionKey>;
   ```

   The legacy `send_refer(...)` becomes a one-line forwarder
   (`send_refer_with_extras(..., Vec::new())`) so external callers
   (`rvoip-sip-registrar`) keep compiling per §7.2.

2. **CANCEL (G2):** `send_cancel_with_options` at line 1867 must:
   - Call `take_staged` on the `CancelBuilder.state` before queuing
     (this lives in `crates/rvoip-sip/src/api/send/cancel.rs:40-46`).
   - Pass both `opts.reason` (RFC 3326) and `opts.extra_headers`
     into a new `send_cancel_with_extras(dialog_id, reason, extras)`
     manager helper that builds the CANCEL via
     `transaction/utils/request_builders.rs::create_cancel_from_invite`
     and appends the extras after stack-managed cloning per §5.2.

3. **DialogAdapter (rvoip-sip):** at
   `crates/rvoip-sip/src/adapters/dialog_adapter.rs`, the 12
   `send_*_with_options` mirror methods (per §9 Phase C file list)
   must:
   - Run `HeaderPolicy::validate_outbound(opts.method(), &extras)`
     before crossing into dialog-core (§5.4).
   - Apply the §6.1 merge table (proxy override prepends Route via
     existing `prepend_outbound_proxy_route` at
     `dialog_adapter.rs:2086`).
   - Forward extras to dialog-core's new
     `send_*_with_options` / `send_*_with_extras` entries.

4. **Audit-comment removal:** delete the explanatory comment block
   at `crates/rvoip-sip-dialog/src/api/unified.rs:1764-1775` once the
   threading is real. Comments that document a broken state become
   stale lies the moment the state is fixed.

5. **Smoke test (slice of G3):** add
   `crates/rvoip-sip/tests/outbound_request_builders_integration.rs`
   covering at minimum INVITE, REFER, NOTIFY, INFO, BYE, MESSAGE
   each emitting an asserted-on-wire `X-Test: smoke` header. The
   remaining 6 builders + full §10 #9 coverage land in PR 9. This
   slice is the gate that proves G1 actually closed.

**Acceptance:**
- `cargo test -p rvoip-sip --test outbound_request_builders_integration` passes.
- Manual wire-trace inspection on each of the 6 covered methods
  shows `X-Test: smoke` after stack-managed headers, before body.
- Rerun the audit's grep for `"follow-up\|staged for\|wiring lands\|pending"`
  in `crates/rvoip-sip-dialog/src/api/unified.rs` returns zero hits
  in the `*_with_options` block.

**Estimated effort:** 1.5 engineer-weeks (the audit's 1 day estimate
is the dialog-core threading; the rvoip-sip-side adapter work and
the smoke test add the rest).

---

### PR 2 — Wire `IncomingRegister` construction on the bus path

**Closes:** G5.
**Spec sections:** §3.4 (`RegisterResponseBuilder` reachability),
§7.5 (cross-crate `IncomingRegister.raw_request` enrichment), §9
Phase A acceptance, §9 Phase D acceptance, §10 test #27.

**Files & edits:**

1. `crates/rvoip-sip-dialog/src/protocol/register_handler.rs:92-108`
   — at the point where the inbound REGISTER is parsed and dispatched,
   bridge `SessionCoordinationEvent::IncomingRegister` (or the
   equivalent internal variant) to the cross-crate
   `DialogToSessionEvent::IncomingRegister { ..., raw_request: Some(Arc::new(bytes)) }`.
   Today this publish path does not run for the new builder
   surface — it bypasses session-core because legacy
   `rvoip-sip-registrar` reads REGISTER directly off the dialog-core
   side. The bridge is additive: the old path stays for the
   registrar crate; the new bus event is what `IncomingRegister`
   reads.

2. `crates/rvoip-sip/src/adapters/session_event_handler.rs:383` —
   replace the discard branch:
   ```rust
   DialogToSessionEvent::IncomingRegister { .. } => {
       debug!("IncomingRegister is handled by dialog registration paths");
       Ok(())
   }
   ```
   with construction of an `IncomingRegister` via the existing
   `with_request_and_coordinator(...)` constructor at
   `crates/rvoip-sip/src/api/incoming.rs:1397-1420`. Re-parse the
   `raw_request: Arc<Bytes>` once via
   `rvoip_sip_core::parse_message` (mirror the pattern used for
   `InfoReceived` / `MessageReceived` / `OptionsReceived` already in
   the same file at lines 397-409).

3. Publish a new `Event::IncomingRegister(IncomingRegister)` to the
   app-event channel via `publish_api_event` so subscribers (and
   `CallHandler::on_register_received` — to be added in PR 6 as
   part of the Phase E callback work) see it.

4. **Carry the dead-code annotations away.** Once construction is
   wired, the compiler warnings on
   `IncomingRegister::synthetic / with_request / with_request_and_coordinator`
   disappear. If any constructor remains genuinely unused after this
   PR, delete it rather than leave it `#[allow(dead_code)]`.

**Acceptance:**
- `cargo build -p rvoip-sip` emits zero `dead_code` warnings on
  `IncomingRegister`, `RegisterResponseBuilder::new`, and the
  related private fields the audit flagged.
- New integration test
  `crates/rvoip-sip/tests/registrar_response_builder.rs` (§10 test
  #27) — REGISTER → `IncomingRegister` → `accept_builder()`
  with `with_expires(3600)`, `with_service_route(routes)`,
  `with_path_echo()` → wire 200 OK contains all three headers in
  RFC-correct positions.

**Estimated effort:** 0.5 engineer-week.

---

### PR 3 — Fix `GenericResponseBuilder::method()` (carry inbound method through constructor)

**Closes:** G7.
**Spec sections:** §3.4 ("Response builder `method()` semantics"),
§5.1 (HeaderPolicy classification picks the right column).

**Files & edits:**

1. `crates/rvoip-sip/src/api/respond/generic.rs:21-39` — change the
   constructor signature to accept the request method:

   ```rust
   pub(crate) fn new(
       coord: Arc<UnifiedCoordinator>,
       call_id: CallId,
       method: Method,
       status: u16,
   ) -> Result<Self> { ... }
   ```

   Store `method` on the struct; return it from
   `SipRequestOptions::method()` instead of the hardcoded
   `Method::Invite`.

2. Update all call sites — the `respond_builder(status)` entry
   points on `IncomingCall` (passes `Method::Invite`),
   `IncomingRequest` (passes `request.method()`), and
   `IncomingRegister` (passes `Method::Register`).

   Per §3.4: each call site already knows the underlying request's
   method; the caller of `new` is the right place to thread it.

3. Verify the policy classification kicks in: add a doctest on
   `IncomingRequest::respond_builder` showing that
   `with_header(TypedHeader::Authorization(...))` on a NOTIFY-shaped
   `respond_builder(401)` returns `Err(UseDedicatedSetter("with_credentials"))`
   per the §5.1 NOTIFY column.

**Acceptance:**
- `cargo test -p rvoip-sip --test generic_response_integration`
  (§10 test #20) passes for INVITE / NOTIFY / REFER / OPTIONS
  inbound-request paths.
- Doctest on `respond_builder` compiles and asserts the policy
  classification outcome.

**Estimated effort:** 0.25 engineer-week.

---

### PR 4 — Implement `RegisterRefreshBuilder.send()` end-to-end

**Closes:** G4.
**Spec sections:** §3.3 (`RegisterRefreshBuilder`), §7.1 (refresh
flag on `RegisterRequestOptions`), §12.5 (refresh dispatch routes
through the parent action with `refresh: true`).

**Files & edits:**

1. `crates/rvoip-sip/src/api/send/register.rs:153-161` — replace
   the `NotImplemented` body with a real dispatch path. The builder
   constructs `RegisterRequestOptions { refresh: true, expires,
   credentials, extra_headers, ... }` and:

   - For now, dispatches via the dialog-adapter mirror method
     `send_register_refresh_with_options(self.handle.aor_or_id(), opts)`
     which forwards to dialog-core's existing
     `send_register_refresh_with_options` (added in Phase B).
   - Once PR 5 lands, this can route through
     `Action::SendREGISTERWithOptions` with `refresh: true` per
     §7.1; switching the dispatch path is a one-line edit at that
     point.

2. Confirm the existing `RegistrationHandle::refresh()` returns the
   builder (no signature change) so external callers see no
   breakage.

3. Add a doctest on `RegisterRefreshBuilder::send` showing the
   typical usage:
   ```rust
   let reg = coord.register(registrar, user, pw).send().await?;
   reg.refresh().with_expires(3600).send().await?;
   ```

**Acceptance:**
- `cargo test -p rvoip-sip --test register_refresh_integration`
  (new test, slice of §10) — initial REGISTER establishes; refresh
  sends a REGISTER with the same Call-ID (per RFC 3261 §10.2.4),
  CSeq incremented, `Expires: 3600`.
- The dead-code warning on `RegisterRefreshBuilder::new` clears.

**Estimated effort:** 0.5 engineer-week.

---

### PR 5 — Replace stubbed INVITE / REGISTER state-machine action handlers

**Closes:** G6.
**Spec sections:** §7.1 ("All twelve outbound methods route through
`Action::Send*WithOptions`"), §7.3 (auth-retry re-reads the stash).

**Files & edits:**

1. `crates/rvoip-sip/src/state_machine/actions.rs:1833-1856` —
   replace the two `debug!("... stub on session ...")` arms:

   ```rust
   Action::SendINVITEWithOptions => {
       if let Some(opts) = session.pending_invite_options.clone() {
           dialog_adapter
               .send_invite_with_options(&session.session_id, (*opts).clone())
               .await?;
       }
   }
   Action::SendREGISTERWithOptions => {
       if let Some(opts) = session.pending_register_options.clone() {
           if opts.refresh {
               dialog_adapter
                   .send_register_refresh_with_options(&session.session_id, (*opts).clone())
                   .await?;
           } else {
               dialog_adapter
                   .send_register_with_options(&session.session_id, (*opts).clone())
                   .await?;
           }
       }
   }
   ```

   Note `clone()` not `take()`: the §7.3 stash invariant is
   "set-once, consumed-once at *final* response," not
   action-dispatch. The clear happens at response-resolution per the
   spec — auth-retry needs the stash to persist across multiple
   dispatches.

2. Audit the other 10 `Action::Send*WithOptions` handlers (the same
   `actions.rs` file, lines preceding 1820) for the same `take()`
   bug if present — per §7.3 invariant #2 they must `clone()` and
   only the response-resolution path may clear.

3. Add the §7.3 invariant #5 conflict guard. In the rvoip-sip
   builders' `.send()` methods, before stashing, check whether
   `pending_<method>_options.is_some()` and return
   `Err(SessionError::Conflict { method })` if so. This requires:
   - Adding `SessionError::Conflict { method: Method }` to
     `crates/rvoip-sip/src/errors.rs` (§8 already prescribes this
     variant).
   - Auditing all 12 builders to add the guard in `.send()`.

**Acceptance:**
- `cargo test -p rvoip-sip --test stash_lifecycle_integration`
  (§10 test #23) passes the (a)/(b)/(c) clauses:
  (a) successful send leaves `pending_invite_options = None`,
  (b) concurrent `.bye().send()` returns `Err(Conflict)`,
  (c) simultaneous `.info()` + `.notify()` both succeed.
- `cargo test -p rvoip-sip --test builder_auth_retry_preserves_headers`
  (§10 test #21) — the X-Trace header survives the 401 retry on
  both INVITE and REGISTER paths.

**Estimated effort:** 1 engineer-week (handler rewrite is small;
auth-retry and cancel-safety verification eat the time).

---

### PR 6 — Phase E callback dispatch fix; cross-crate `raw_request` enrichment fully populated

**Closes:** G8, G12, G13.
**Spec sections:** §9 Phase E (callback rename strategy, line 1440
of spec), §7.5 (preserve original inbound bytes through the bus).

**Files & edits:**

1. **G8 — callback dispatch.**
   `crates/rvoip-sip/src/api/callback_peer.rs:1886-1890` (REFER) and
   `:2004-2014` (NOTIFY): replace the legacy positional calls with
   typed dispatch. Pattern:

   ```rust
   Event::ReferReceived { call_id, request, .. } => {
       let handle = SessionHandle::new(call_id, coordinator);
       handler.on_refer_received(request).await;          // typed (canonical)
       handler.on_transfer_request(handle, refer_to).await; // legacy adapter, deprecated
   }
   ```

   Then in the trait definitions at lines 894 / 984, mark the
   positional methods `#[deprecated(since = "0.3.0", note = "use
   on_refer_received(IncomingRequest) — see SIP_API_DESIGN_2.md
   Phase E")]` and rewrite their default impls to no-op (since the
   typed method is now what fires for new code, and the deprecated
   forms exist only to keep external `impl CallHandler for ...`
   blocks compiling). Per spec §9 Phase E line 1454-1458 the
   deprecated forms are scheduled for removal in the next breaking
   release — this PR adds the deprecation, not the removal.

   Apply the same pattern for `on_notify` ↔ `on_notify_received`,
   adding a typed `on_register_received(IncomingRegister)` method
   while we're in this file (PR 2 publishes the event; PR 6 wires
   the dispatch).

2. **G12 — internal dialog-core enrichment.**
   `crates/rvoip-sip-dialog/src/events/session_coordination.rs:228-243`
   — extend `SessionCoordinationEvent::TransferRequest` with
   `raw_request: Option<Arc<Bytes>>`. Update the publish site at
   `crates/rvoip-sip-dialog/src/manager/protocol_handlers.rs:426` to
   thread the inbound REFER bytes (already preserved on the parse
   path).
   Then in `crates/rvoip-sip-dialog/src/events/event_hub.rs` extend
   `convert_session_coordination_to_cross_crate` to populate
   `raw_request: Some(Arc::clone(&bytes))` on the cross-crate
   `TransferRequested` variant.

3. **G13 — `Option<Arc<Bytes>>` vs `Arc<Bytes>` decision.** The
   spec (§7.5) prescribes non-optional. The current
   `Option<Arc<Bytes>>` pattern was an interim hedge. We **keep**
   `Option<Arc<Bytes>>` on the bus (for serde-skip safety and lean
   mode per §13.3), but make it **non-`None` at every publish
   site** in this PR. Update `protocol_handlers.rs:722` and any
   other publish site grep finds with `raw_request: None`. Add a
   `tracing::warn!` at the `Some` consumption sites if the field is
   ever observed as `None` so future regressions are loud rather
   than silent. Update the `Option<...>` doc comment on each variant
   to read: *"`None` indicates a synthesized-or-test publish path;
   production publish sites must populate this."*

   Document this decision in spec §7.5 via a follow-up doc PR
   (out of scope for code; tracked as a doc-update note in PR 9).

4. **Carry-through bridge (Phase E acceptance line 1467-1469):**
   verify in `event_hub.rs::convert_session_coordination_to_cross_crate`
   that all six inbound mid-dialog methods (REFER, NOTIFY, INFO,
   MESSAGE, OPTIONS, UPDATE) are bridged. Add the missing arms.

**Acceptance:**
- `cargo build -p rvoip-sip` reports zero `dead_code` warnings on
  `on_refer_received`, `on_notify_received`, `on_register_received`.
- New integration test asserts that an inbound REFER triggers
  `on_refer_received(IncomingRequest)` AND that
  `request.header(&HeaderName::ReferredBy)` returns the inbound
  Referred-By value (proves bytes survived end-to-end).
- `git grep "raw_request: None"` in `crates/rvoip-sip-dialog/src/`
  returns zero hits.

**Estimated effort:** 1.5 engineer-weeks (the deprecation +
dispatch rewrite is mechanical; the bytes-preservation audit
across 6 inbound methods × 3 publish sites is what consumes the
time).

---

### PR 7 — Deprecate legacy methods; reconcile coordinator entry naming

**Closes:** G9, G11.
**Spec sections:** §9 Phase C deprecation table (line 1334-1340),
§3.3, §11.4 migration table.

**Files & edits:**

1. **G9 — apply the §9 Phase C deprecation matrix.** Add
   `#[deprecated(since = "0.3.0", note = "use <surface>.<verb>(...).send().await — see SIP_API_DESIGN_2.md")]`
   to each method in the spec's table:

   | Surface | File | Methods |
   |---|---|---|
   | `UnifiedCoordinator` | `src/api/unified.rs` | `make_call`, `make_call_with_auth`, `make_call_with_pai`, `make_call_with_headers`, `register`, `register_with`, `send_refer`, `send_refer_with_replaces`, `send_notify`, `send_info`, `hangup_with_reason`, `reject_call`, `redirect_call`, `subscribe_dialogs` |
   | `PeerControl` | `src/api/stream_peer.rs` | `call`, `call_with_auth`, `call_with_headers` |
   | `CallbackPeerControl` | `src/api/callback_peer.rs` | `call`, `call_with_auth`, `call_with_headers` |
   | `Endpoint` / `EndpointControl` | `src/api/endpoint.rs` | `call`, `call_with_headers` |

   Per spec §9 Phase C line 1342-1347, `make_transfer_leg` is **not**
   deprecated — keep it.

   The workspace already sets `deprecated = "allow"` at the lint
   level (`Cargo.toml:54-67` per spec), so internal callers compile
   without warnings; external callers see standard warnings. Verify
   this lint level is still in place before flipping the
   annotations on; if not, the PR is split (lint level first as a
   trivial PR, deprecations second).

2. **G11 — coordinator entry naming reconciliation.** The current
   code exposes `subscribe_event` and `register_builder` to avoid
   colliding with legacy `subscribe_dialogs` / `register_with`. Per
   spec §3.3 the canonical entries are `subscribe(target,
   event_package)` and `register(registrar, user, pw)`. We
   reconcile in **this** PR (rather than at the spec level)
   because:
   - Once PR 7 deprecates the legacy methods, the names free up.
   - The migration-guide examples in spec §11.4 match the spec's
     canonical names — keeping the implementation drift would
     break the examples by name.

   **Migration:** rename
   `UnifiedCoordinator::subscribe_event` → `subscribe`,
   `UnifiedCoordinator::register_builder` → `register`. Old names
   stay as `#[deprecated]` aliases for one cycle. Same rename on
   the other three surfaces.

3. **Verify deprecation reachability:** add a CI grep guard
   (or a `compiletest` check) that asserts each deprecated method
   from the table is annotated. The audit caught this gap by
   counting annotations vs the prescribed list — we automate the
   check.

**Acceptance:**
- `cargo build -p rvoip-sip` clean (workspace allow level holds).
- Manual check: pasting any §11.4 migration-table "Tomorrow"
  example into a fresh source file compiles. The spec was
  previously not compilable verbatim due to the naming drift.
- The legacy `pai_integration.rs`, `extra_headers_integration.rs`
  tests continue to pass (deprecation warnings allowed) per §10
  test #14.

**Estimated effort:** 0.5 engineer-week.

---

### PR 8 — Convenience body module; crate-root re-exports

**Closes:** G10, G14.
**Spec sections:** §3.6 (`api::bodies` and `multipart_*`), §9
Phase A acceptance for crate-root re-exports.

**Files & edits:**

1. **G10 — `api::bodies` module.** Add `crates/rvoip-sip/src/api/bodies.rs`
   exporting:

   ```rust
   pub fn sdp(s: impl Into<String>) -> (String, Bytes);                  // application/sdp
   pub fn dtmf_relay(signal: char, duration_ms: u32) -> (String, Bytes); // application/dtmf-relay
   pub fn pidf_xml(presence: &Presence) -> (String, Bytes);              // application/pidf+xml — RFC 3863
   pub fn simple_message_summary(/* ... */) -> (String, Bytes);          // application/simple-message-summary — RFC 3842
   pub fn isup_l3(bytes: impl Into<Bytes>) -> (String, Bytes);           // application/isup — RFC 3204
   ```

   Each function is a doc-tested one-liner. `Presence` is the
   minimal pidf+xml model — keep it small and `#[non_exhaustive]`
   per §3.7 forward-compatibility hygiene.

2. **G10 — `multipart_mixed` / `multipart_parse` /
   `MultipartParseError`** in
   `crates/rvoip-sip/src/api/headers/convenience.rs` per §3.6.

3. **G14 — crate-root re-exports.** Add to
   `crates/rvoip-sip/src/lib.rs`:

   ```rust
   pub use api::incoming::{IncomingCall, IncomingRequest, IncomingResponse, IncomingRegister};
   pub use api::headers::view::SipHeaderView;
   pub use api::headers::options::{
       SipRequestOptions, BuilderHeaderState, BuilderStrictness,
       HeaderPolicyViolation, ViolationReason, HeaderCarryThroughReport,
   };
   pub mod bodies { pub use crate::api::bodies::*; }
   ```

4. **Crate-level doc audit (§9 Phase A item 5):** verify the five
   sub-sections are present in `lib.rs`'s `//!` block — decision
   chart, B2BUA example, three trust-boundary patterns,
   classification reference, cross-links. Spec §10 test #32 is the
   manual gate; add the missing pieces if any.

**Acceptance:**
- `use rvoip_sip::IncomingResponse;` compiles in a fresh
  downstream crate.
- `cargo test -p rvoip-sip --test multipart_body_integration`
  (§10 test #24) passes.
- `cargo doc -p rvoip-sip --no-deps` clean — the
  `#![deny(rustdoc::broken_intra_doc_links)]` lint already in the
  spec acceptance does not regress.

**Estimated effort:** 0.5 engineer-week.

---

### PR 9 — The §10 verification suite (24 named integration tests)

**Closes:** the remainder of G3.
**Spec sections:** §10 (verification, lines 1473-1587).

**Files & edits:**

Add the 24 named integration tests under `crates/rvoip-sip/tests/`,
in this order (each line lists the test file + which spec test #
it implements + the gap PR(s) it gates):

| # | File | Gates |
|---|---|---|
| 6 | `header_policy_unit.rs` | covers every `TypedHeader` × every method matrix cell |
| 7 | `header_inspection_integration.rs` | PR 1 inbound enrichment |
| 8 | `forbidden_header_guard_integration.rs` | PR 1 policy enforcement |
| 9 | `outbound_request_builders_integration.rs` (full 12-builder version) | PR 1 (smoke version covers 6) |
| 10 | `response_builders_integration.rs` | PR 3 |
| 11 | `b2bua_carry_through_integration.rs` | PR 1 + PR 6 (the litmus test) |
| 12 | `builder_strictness_integration.rs` | PR 1 |
| 13 | `config_builder_coexistence.rs` | PR 1 + PR 7 |
| 16 | `b2bua_contact_rewrite_integration.rs` | PR 1 |
| 17 | `per_leg_outbound_proxy_integration.rs` | PR 1 |
| 18 | `provisional_carry_through_integration.rs` | PR 6 (response enrichment) |
| 19 | `third_party_register_integration.rs` | PR 2 + PR 4 |
| 20 | `generic_response_integration.rs` | PR 3 |
| 21 | `builder_auth_retry_preserves_headers.rs` | PR 5 |
| 22 | `header_case_insensitive_lookup.rs` | PR 1 |
| 23 | `stash_lifecycle_integration.rs` | PR 5 |
| 24 | `multipart_body_integration.rs` | PR 8 |
| 25 | `reliable_provisional_bridge.rs` | PR 6 |
| 26 | `topology_hiding_guarantee.rs` | PR 1 + PR 6 |
| 27 | `registrar_response_builder.rs` | PR 2 |
| 28 | `options_timeout.rs` | (see §3.1 below — depends on OPTIONS authorship completion) |
| 29 | `cancel_safety_integration.rs` | PR 1 + PR 5 |
| 30 | `auto_emit_headers.rs` | PR 1 (and §7.4 wiring; see §3.2 below) |
| 31 | `trace_redaction.rs` | (see §3.2 below — depends on §12.4 redactor wiring) |

**A note on tests #28, #30, #31.** These three exercise capabilities
the audit did not separately call out as missing because the audit
was scoped to surface vs wire conformance, but the spec §10 list
them as gating. They depend on:

- **#28 OPTIONS authorship.** `send_options_out_of_dialog_with_options`
  in dialog-core today returns `NotImplemented` (audit Finding #1
  cites this). PR 1 closes the threading gap, but the *initial*
  OPTIONS authorship in
  `crates/rvoip-sip-dialog/src/transaction/utils/request_builders.rs`
  (parallel to `send_message_out_of_dialog`) is a brand-new helper
  per spec §9 Phase B. PR 1 must include this helper for the
  threading to have anything to deliver.
- **#30 auto-emit headers.** `Config.auto_emit_extra_headers` per
  spec §7.4 — the field exists on `Config`; verify the auto-BYE,
  auto-CANCEL, auto-NOTIFY emission sites consult it. Add the
  missing consultation if any in PR 1.
- **#31 trace redaction.** The `TraceRedactor` trait per spec
  §12.4. If the trait exists but the consultation site in
  `DialogAdapter` is missing, add it in PR 6 (alongside the
  cross-crate plumbing work that touches the same area). If the
  trait does not exist, this becomes a new sub-task: ship the
  trait + `RedactionDecision` + `Config.trace_redaction` field +
  consultation site. Check at the start of PR 6 and split if
  needed.

**Acceptance:**
- `cargo test -p rvoip-sip` runs all 24 tests; all pass.
- `cargo test --doc -p rvoip-sip` — the spec §10 acceptance for
  ~60 new doctests is met (each new setter has a doctest; this is
  asserted by counting `#[doc]` blocks per setter).
- Test #32 (manual): open `target/doc/rvoip_sip/index.html` and
  confirm the Gateway / B2BUA / SBC Authoring section has all 5
  sub-sections (decision chart, B2BUA example, three
  trust-boundary patterns, classification reference, cross-links)
  per §9 Phase A acceptance.

**Estimated effort:** 2 engineer-weeks.

---

## 4. Cross-cutting work captured during PR sequencing

These items surface during the PR work above; they are listed once
here so reviewers can see the full surface.

### 4.1 `SessionError::Conflict { method }` — new variant

Spec §8 prescribes the variant. PR 5 ships it (used by the
single-in-flight guard per §7.3 invariant #5). Mark
`SessionError` `#[non_exhaustive]` if it isn't already (spec §3.7).

### 4.2 `Action::Send*WithOptions` `clone()` vs `take()` audit

Spec §7.3 invariant #2: stash persists across auth retry; only the
response-resolution path may clear. PR 5 audits all 12 handlers in
`actions.rs` for compliance. Where today's handler uses `take()`,
switch to `clone()` and wire a clear at the response-resolution
path (in `state_machine/event_handlers.rs` or the equivalent —
identify in PR 5).

### 4.3 Spec §3.4 response-builder method semantics — beyond G7

Spec §3.4 lists three response-builder method-resolution rules.
PR 3 fixes `GenericResponseBuilder`. Audit the other six response
builders (`AcceptBuilder`, `RejectBuilder`, `RedirectBuilder`,
`ProvisionalBuilder`, `AuthChallengeBuilder`, `RegisterResponseBuilder`)
for the same hardcoded-method bug; if any is wrong, fold the fix
into PR 3.

### 4.4 Subscription multiplex (§12.5 implementer verification)

Spec §12.5 leaves an implementer-verification flag: does
dialog-core's subscription manager already accept a
`SubscriptionId` on the NOTIFY path? Today's
`UnifiedDialogApi::send_notify(unified.rs:1449)` takes a
`DialogId`. PR 1 must check this on its way through the NOTIFY
options threading; if `subscription_id` is not yet plumbed, add it
to `send_notify_with_options`'s implementation. If the
sub-manager-aware NOTIFY path is a larger lift than PR 1 absorbs,
split it as PR 1.5 (between PR 1 and PR 2 in the order).

### 4.5 Spec doc-update follow-ups

Per the gap analysis, three small spec-side updates are warranted
once the implementation lands. These ship as a single doc-only PR
after PR 9:

1. §7.5 — clarify `Option<Arc<Bytes>>` on the bus (lean-mode hedge
   plus serde-skip), with the publish-site invariant "production
   sites must populate `Some`."
2. §11.4 — once PR 7 reconciles names, the migration table compiles
   verbatim; remove the implementer-note to the contrary.
3. §3.3 — add the renamed `subscribe` / `register` entries with a
   one-line note that the prior `subscribe_event` /
   `register_builder` names are deprecated aliases.

The doc PR is **after** PR 9 deliberately — the spec is updated to
reflect the world we shipped, not the world we wished for.

---

## 5. PR sequencing and parallelism

Sequencing summary (× = blocks; → = depends on):

```
PR 1 (extras + CANCEL + smoke test)  ←  blocks PR 4, PR 5, PR 9 partially
PR 2 (IncomingRegister wiring)        ←  independent of PR 1
PR 3 (GenericResponseBuilder.method)  ←  independent
PR 4 (RegisterRefreshBuilder.send)    →  PR 1 (needs threading) + PR 2 (uses bus path)
PR 5 (state-machine handlers)         →  PR 1
PR 6 (callback dispatch + bytes)      →  PR 2 (IncomingRegister surface ready)
PR 7 (deprecations + naming)          →  PR 1 through PR 6 (everything must land first; legacy callers point at the new path)
PR 8 (bodies + re-exports)            →  PR 6 (re-export surface is stable post-Phase-E rename)
PR 9 (full §10 suite)                 →  PR 1–PR 8
```

**Parallelism:** PR 2, PR 3 can be worked in parallel with PR 1.
PR 5 and PR 6 can be worked in parallel after PR 1 lands. PR 7
sequences last among the code PRs because it depends on the
canonical builder path being trustworthy.

**Estimated total effort.** Sum of the per-PR estimates: **8.25
engineer-weeks**, which aligns with the audit's bottom-line
estimate of "2–3 engineer-weeks for code in remediation steps 1–9
plus a similar amount for the test suite in step 10," scaled up
modestly for the dependent rework in PRs 4–7. With two engineers
in parallel after PR 1 unblocks the tree, calendar time is
roughly **5–6 weeks** end-to-end.

---

## 6. Definition of done

The work is done when **every one** of these holds simultaneously:

1. `cargo build -p rvoip-sip` and `cargo build -p rvoip-sip-dialog`
   compile with **zero `dead_code` warnings** in the new surfaces
   (the audit's 52-warning baseline drops to the pre-existing
   floor; new dead-code is rejected at PR review).
2. `git grep -i "follow-up\|staged for\|wiring lands\|pending"` in
   `crates/rvoip-sip/src/` and `crates/rvoip-sip-dialog/src/`
   returns zero hits inside any `*_with_options` or builder body.
   Comments may reference "future PR work" (e.g., lean-mode) only
   in code that the spec explicitly defers.
3. All 24 named §10 integration tests exist and pass on `main`.
4. The 60+ doc-tests (one per new setter) pass via
   `cargo test --doc -p rvoip-sip`.
5. Pasting any §11.4 migration-table "Tomorrow" example into a
   fresh `examples/` file compiles.
6. Manual gate (§10 test #32): the crate-level rustdoc has the
   five-section "Gateway / B2BUA / SBC Authoring" block.
7. The audit's eight 🔴 findings + six ⚠️ findings each have a
   linked PR in the merge log marked `Closes: G<n>`.

---

## 7. Risks and mitigations

| Risk | Likelihood | Mitigation |
|---|---|---|
| PR 1 (`extra_headers` threading) discovers a deeper invariant violation in `request_builder_from_dialog_template` (e.g., extras stamped before stack-managed headers in some path) | Medium | The audit reports the slot is in place at `transaction/dialog/mod.rs:107-118` and "appends them after all stack-managed headers." Read the helper end-to-end before splitting work. If the slot is honored only on some code paths, expand PR 1's scope to make it uniform. |
| §12.5 NOTIFY subscription-id multiplex turns out to require dialog-core sub-manager surgery | Medium | Split as PR 1.5 (estimated 0.5–1 week) per §4.4 above. Do not block PR 1 on it — single-subscription NOTIFY (the common case) does not need the sub-manager change. |
| `TraceRedactor` infrastructure (§12.4) is missing entirely, not just unwired | Low | Identify at the start of PR 6. If absent, ship as a new sub-PR (PR 6.5, ~0.5 week). The trait + Config field + consultation site is small; the cost is the test (#31). |
| Stash `take()` vs `clone()` audit (§4.2) reveals existing handlers that mutate the stash incorrectly today, masking auth-retry bugs that are hidden because `extra_headers` were never on the wire | High | This is the second-order win from PR 1 — once headers reach the wire, auth-retry behaviour becomes observable. PR 5's job is to make it correct. Allocate a half-week buffer in PR 5's estimate (already in the 1-week figure). |
| External crates (notably `rvoip-sip-registrar`) break under the new bus path or rename | Low | Spec §7.2 explicitly preserves legacy dialog-core methods. PR 2 adds the `IncomingRegister` bus event additively — the registrar crate's existing direct-read path is untouched. PR 7's renames keep deprecated aliases for one cycle. Run the workspace test suite at every PR boundary. |
| §10 test #11 (`b2bua_carry_through_integration`) — the litmus test — fails after PR 1 because Strict mode rejects something the spec example uses | Medium | Spec §11.2 example uses `with_strictness(BuilderStrictness::Lenient)` only in the §6.2 Path 3 case; the §11.2 litmus example does not. Walk every header in the litmus example through the §5.1 matrix during PR 1 review and confirm classification. If a header turns out to be `MethodShaped` and the example uses `with_raw_header`, fix the example in the same PR (§4.5 doc PR). |

---

## 8. Out of scope (unchanged from spec §14)

For clarity to reviewers — these explicitly remain out of scope and
are **not** addressed by this gap plan:

- Removing deprecated methods (stays through ≥0.3.0).
- Migrating in-tree examples and tests onto the builder API
  (separate sweep PR).
- Migrating `rvoip-sip-registrar` onto `IncomingRegister` /
  `RegisterResponseBuilder` (additive bus event leaves the
  registrar's direct-read path working).
- New SIP methods beyond the design's twelve.
- Stateless SIP proxy mode.
- Lean-mode feature flag (§13.3) — `Option<Arc<Request>>` field
  shape preserved so this can land later.
- Changes to `rvoip-sip-core` or `rvoip-sip-transport`.
- Changes to dialog-core's dialog state machine, route-set logic,
  transaction core, or CSeq management (additive only).

---

## 9. Bottom line

The audit's framing — *"the API of the design without the behavior
of the design"* — is the gap. Nine ordered PRs over ~8.25
engineer-weeks (5–6 calendar weeks with two engineers) close it.
PR 1 is the single highest-leverage piece of work: once
`extra_headers` reach the wire, the rest of the gaps become
testable and most of the §10 suite becomes tractable.

The success criterion is unambiguous: §6 above lists seven gates,
all of which must hold simultaneously. Anything short of that
remains "Phase A complete; Phases B–E surface-only" and is not
accepted as finishing the design.
