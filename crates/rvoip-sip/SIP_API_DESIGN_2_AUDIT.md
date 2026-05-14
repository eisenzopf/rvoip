# SIP_API_DESIGN_2 — Implementation Audit

**Audit date:** 2026-05-11
**Auditor:** independent review against `SIP_API_DESIGN_2.md`
**Scope:** completeness, quality, conformance to spec, performance, developer API surface

---

## Verdict

**⚠️ NOT complete.** The structural skeleton is in place but the load-bearing
feature — application-supplied headers reaching the wire — is broken on the
outbound side, and several builders return `NotImplemented`. The developers
shipped surface-area compilation gates but skipped most of the actual
plumbing. This should not be accepted as "finished" against the design spec.

`cargo build -p rvoip-sip` and `cargo build -p rvoip-sip-dialog` both pass.
`rvoip-sip` emits 52 warnings — several flag dead-code (unused constructors
on `IncomingRegister` and `RegisterRefreshBuilder`) that maps directly to
the gaps below.

---

## At a glance

| Phase | Surface | Wire / Behavior | Verdict |
|---|---|---|---|
| **A — Inbound inspection** | ✅ Complete | ⚠️ Bytes-threading partial | **~90% done** |
| **B — Dialog-core options** | ✅ Structs + method signatures exist | ❌ `extra_headers` ignored on every method; OPTIONS returns `NotImplemented`; CANCEL ignores opts entirely | **~30% done** |
| **C — Send builders** | ✅ All 12 builders + trait + policy + state-machine variants | ❌ Headers don't reach wire (Phase B gap); INVITE/REGISTER action handlers are stubs; `RegisterRefreshBuilder` returns `NotImplemented`; convenience body module missing; legacy methods not deprecated | **~55% done** |
| **D — Response builders** | ✅ All 7 + entry points | ⚠️ `GenericResponseBuilder::method()` hardcoded to `Method::Invite`; `RegisterResponseBuilder` reachable only via `IncomingRegister` which is never constructed | **~80% done** |
| **E — In-dialog requests** | ✅ Events + cross-crate variants + OPTIONS bridge | ❌ `Event::ReferReceived`/`NotifyReceived` enrichment present but dispatch still calls legacy `on_transfer_request`/`on_notify`; `on_refer_received`/`on_notify_received` are dead code; `TransferRequest` missing `raw_request` | **~60% done** |
| **§10 verification tests** | — | ❌ **0 of 24 named tests exist** | **~0% done** |

---

## Highest-severity findings

### 🔴 1. Outbound `extra_headers` never reach the wire

**File:** `crates/rvoip-sip-dialog/src/api/unified.rs:1764-1977`

Every `*_with_options` method either:

- Forwards to the legacy `send_*` manager method without passing
  `extra_headers` (REFER, NOTIFY, INFO, BYE, UPDATE, REINVITE, MESSAGE,
  SUBSCRIBE), OR
- Ignores the entire options struct (`_opts: CancelRequestOptions` at
  line 1870), OR
- Returns `ApiError::Internal { "staged for Phase B follow-up — surface
  exists; transaction authorship pending" }` (OPTIONS at line 1971).

The comment at line 1769-1775 explicitly admits the gap:

> *"the `extra_headers` field on each options struct is staged but not
> yet stamped on the wire."*

**Impact.** This breaks the **primary goal of the design** (§1: B2BUA
carry-through, custom headers, REFER `Referred-By`/`Replaces`, NOTIFY
custom headers). The rvoip-sip builders stage the headers and pass them
into dialog-core, but dialog-core throws them away. `with_raw_header`,
`with_headers_from`, `as_session_timer_refresh`, `with_replaces`,
`with_referred_by`, `with_target_dialog`, `with_reason`, and the entire
`BuilderStrictness` machinery are decorative.

### 🔴 2. `RegisterRefreshBuilder::send()` returns `NotImplemented`

**File:** `crates/rvoip-sip/src/api/send/register.rs:153-161`

```rust
pub async fn send(self) -> Result<()> {
    let _ = (self.handle, self.expires);
    Err(crate::errors::SessionError::NotImplemented(
        "RegisterRefreshBuilder.send() — manual refresh wiring lands \
         in Phase C follow-up; ..."
            .to_string(),
    ))
}
```

The refresh API exposed on `RegistrationHandle::refresh()` is dead.

### 🔴 3. `IncomingRegister` is type-shaped but never constructed

**Files:**
- `crates/rvoip-sip/src/api/incoming.rs:1298-1450` — type definition
- `crates/rvoip-sip/src/adapters/session_event_handler.rs:383`

`IncomingRegister` is defined with all the `SipHeaderView` plumbing,
constructors `synthetic`, `with_request`, `with_request_and_coordinator`,
and Phase D's `RegisterResponseBuilder` attached. But:

```rust
DialogToSessionEvent::IncomingRegister { .. } => {
    debug!("IncomingRegister is handled by dialog registration paths");
    Ok(())
}
```

The cross-crate bus event is received and discarded. No application ever
sees an `IncomingRegister`. The constructors are the compiler-flagged
dead code.

**Impact.** Registrar use cases (Service-Route, Path echo,
P-Associated-URI, contact-from-binding) are non-functional through the
new API. Phase A's `IncomingRegister.raw_request` plumbing and Phase D's
entire `register_response.rs` are unused.

### 🔴 4. State-machine action handlers for INVITE and REGISTER are stubs

**File:** `crates/rvoip-sip/src/state_machine/actions.rs:1833-1856`

`SendINVITEWithOptions` and `SendREGISTERWithOptions` log "migration
pending" instead of dispatching. The 10 other handlers do forward to
dialog-core, but per Finding #1 they cannot deliver headers anyway.

### 🔴 5. `CancelBuilder` silently drops everything

**File:** `crates/rvoip-sip/src/api/send/cancel.rs:40-46`

```rust
pub async fn send(self) -> Result<()> {
    let _ = self.reason;
    self.coord.dialog_adapter().send_cancel(&self.session_id).await
}
```

No `take_staged()` call. `with_reason`, `with_header`, `with_raw_header`,
`with_headers_from` — all silently dropped before the dialog-core call.

### 🔴 6. `CallHandler::on_refer_received` / `on_notify_received` are dead code

**File:** `crates/rvoip-sip/src/api/callback_peer.rs:1000, 1008`

The new typed callbacks are defined with `{}` default bodies, but the
dispatch sites at lines 1886 (ReferReceived) and 2004 (NotifyReceived)
still call the legacy positional methods:

- `on_transfer_request(handle, target: String)`
- `on_notify(handle, event, sub_state, content_type, body)`

The new methods are never invoked. Phase E §9.5 prescribed that the
legacy methods become deprecated default-impl adapters that decode the
typed `IncomingRequest` into legacy positional fields — that work was
not done. Only `on_notify` is actually marked `#[deprecated]`.

### 🔴 7. `GenericResponseBuilder::method()` is wrong

**File:** `crates/rvoip-sip/src/api/respond/generic.rs:87-89`

```rust
impl SipRequestOptions for GenericResponseBuilder {
    fn method(&self) -> Method { Method::Invite }
    ...
}
```

Per §3.4 this builder is reachable from `IncomingRequest` (for REFER /
NOTIFY / INFO / UPDATE / MESSAGE / OPTIONS) and must return the
underlying request's method so `HeaderPolicy::classify` picks the right
matrix column. The `new()` constructor does not accept a method, so this
cannot be fixed without changing the constructor signature.

### 🔴 8. None of §10's 24 named integration tests exist

`grep` for the prescribed names (`header_policy_unit`,
`b2bua_carry_through_integration`, `stash_lifecycle_integration`,
`registrar_response_builder`, `cancel_safety_integration`, etc.) in
`crates/rvoip-sip/tests/` returns zero hits. Pre-existing tests
(`extra_headers_integration.rs`, `pai_integration.rs`,
`cancel_integration.rs`, etc.) cover legacy paths only and do not
exercise the new builders — `grep` for `\.invite(.*)\.send()` and
`\.refer(.*)\.send()` in `tests/` returns zero hits.

**Impact.** There is no test evidence that any of the new builders work
end-to-end. Given findings 1–7, they likely do not.

---

## Medium-severity findings

### ⚠️ 9. Legacy methods not `#[deprecated]`

Design §9 (Phase C) prescribed `#[deprecated(since = "0.3.0", note = "...")]`
on ~22 legacy methods across `UnifiedCoordinator`, `PeerControl`,
`CallbackPeerControl`, and `Endpoint`. 27 deprecation annotations exist
in the crate, but the specific legacy methods named in the design
(`make_call`, `make_call_with_auth`, `make_call_with_pai`,
`make_call_with_headers`, `register`, `register_with`, `send_refer`,
`send_refer_with_replaces`, `send_notify`, `send_info`,
`hangup_with_reason`, `reject_call`, `redirect_call`,
`subscribe_dialogs`, and the peer-surface `call*` methods) do not appear
to be marked.

### ⚠️ 10. Convenience body module missing

`api::bodies` (`sdp`, `dtmf_relay`, `pidf_xml`, `simple_message_summary`,
`isup_l3`) and `convenience::{multipart_mixed, multipart_parse,
MultipartParseError}` are not present in
`src/api/headers/convenience.rs`. Only the typed header constructors
(`diversion`, `history_info`, `privacy`, `replaces`, `target_dialog`,
`session_expires`, `min_se`, `p_charging_vector`,
`p_called_party_id`) shipped.

### ⚠️ 11. Coordinator entry naming drift

Per §3.3 the entry points are `coord.subscribe(target, event_package)`
and `coord.register(registrar, user, pw)`. Implementation in
`src/api/unified.rs` exposes them as `subscribe_event` and
`register_builder` to avoid name collisions with legacy methods. That is
a pragmatic call, but the migration-guide examples in §11.4 will not
compile verbatim — the docs and code disagree. Either rename legacy
methods or rename builder entries and update the design doc.

### ⚠️ 12. `TransferRequest` cross-crate variant missing `raw_request`

**Files:**
- `crates/rvoip-sip-dialog/src/events/session_coordination.rs:228`
- `crates/rvoip-sip-dialog/src/manager/protocol_handlers.rs:426`

The REFER bytes are not passed to the cross-crate event, so even if the
rvoip-sip side could deliver REFER through `on_refer_received(IncomingRequest)`,
the typed request would have no body to parse from. Phase E §9.5
prescribed this enrichment.

### ⚠️ 13. `infra-common` enrichment uses `Option<Arc<Bytes>>` not `Arc<Bytes>`

Per design §7.5 the fields should be `raw_request: Arc<Bytes>` /
`raw_response: Arc<Bytes>` (non-optional, since the bytes always exist
on the inbound path). Implementation uses `Option<Arc<Bytes>>` and
`protocol_handlers.rs:722` hardcodes `raw_request: None` at one publish
site, defeating the Phase A acceptance criterion. The transition would
have been fine if all publish sites were filled in — they are not.

### ⚠️ 14. `IncomingRegister`, `IncomingRequest`, `IncomingResponse` not re-exported at crate root

The Phase A acceptance asked for crate-level re-exports so gateway
developers can write `use rvoip_sip::IncomingResponse`. Today they live
under `rvoip_sip::api::incoming::*`. Workable, but not the documented
ergonomic.

---

## Quality / API-surface observations

### What is genuinely well done

- `SipRequestOptions` trait and `HeaderPolicy` classifier are cleanly
  designed and would work if the wire layer existed. Forward-compatibility
  hygiene (`#[non_exhaustive]`) is followed for `ViolationReason`,
  `BuilderStrictness`, `HeaderRole`, `MultipartParseError`.
- `SipHeaderView` trait shape is correct (object-safe with boxed iter,
  plus inherent unboxed `headers_named_iter()` for hot paths per §13.4).
- `SurfaceBuilder<B, S>` generic adapter follows the design and avoids
  the 12 × 4 wrapper explosion.
- The crate-level `//!` "Gateway / B2BUA / SBC Authoring" doc section
  exists and includes the decision chart, B2BUA example, trust-boundary
  patterns, classification reference, and cross-links.
- All 12 outbound `Action::Send*WithOptions` variants are defined in
  `state_table/types.rs`.
- Phase A inbound inspection (`IncomingCall` typed access, deprecated
  `HashMap` populated correctly, all four `Incoming*` types implementing
  `SipHeaderView`) is genuinely working.
- Phase E OPTIONS end-to-end bridge (transport → dialog-core
  `CapabilityQuery` → bus `OptionsReceived` → rvoip-sip
  `Event::OptionsReceived` → `CallHandler::on_options_received`) is
  fully wired.
- `IncomingRequest` / `IncomingResponse` re-parsing from `Arc<Bytes>` is
  implemented per design (`session_event_handler::build_incoming_request_from_bytes`)
  — single re-parse, no serialize-and-re-parse round trip.

### What is concerning

- Pervasive "Phase X follow-up" comments suggest the developers planned
  to come back to about half the work and never did. Greppable count:
  `git grep -i "follow-up\|staged for\|wiring lands\|pending"` in the
  new code shows the pattern is systemic.
- The 52 build warnings include `dead_code` on `IncomingRegister`
  constructors, `RegisterRefreshBuilder::new`, and unused private
  fields — these are smoke signals of unwired paths.
- Error-message strategy on broken builders is inconsistent: some
  return `NotImplemented`, some silently drop, some log nothing.
  Production code calling these will fail or misbehave in different
  ways.

### Performance / scalability

- No correctness issues observed. Per-session retention is consistent
  with the design budget (§13.1).
- The new options structs and `pending_*_options` stash fields on
  `SessionState` are correctly typed as `Option<Arc<...>>` and would
  satisfy §7.3's set-once / consumed-once invariants — but since the
  state-machine handlers either are stubs (INVITE/REGISTER) or call
  dialog-core methods that ignore the contents, the invariants are
  untested in practice.
- `with_headers_from`'s `HeaderCarryThroughReport` allocates per call;
  per §13.4 this is acknowledged. No regression versus baseline.

### Developer API surface

The intended ergonomics (one shape across surfaces, one builder per
method, `?` chaining, carry-through) **do work at the type-system
level** — code compiles, the trait is uniform, IDE completion shows the
right setters. But at runtime, behavior diverges from documentation:

- A developer writing `coord.invite(from, to).with_raw_header("X-Foo",
  "bar")?.send().await?` will get a session back with no `X-Foo` on
  the wire. There is no error. The header just disappears. This is
  the worst possible failure mode.
- A developer writing `coord.cancel(&session).with_reason("user")` to
  attach RFC 3326 will see the cancel succeed but no `Reason:` header
  emitted.
- A developer writing `registration.refresh().send().await?` gets
  `Err(NotImplemented)`, which at least is a loud failure.

---

## Recommended remediation order

1. **Thread `extra_headers` through dialog-core.** Each `send_*_with_options`
   in `crates/rvoip-sip-dialog/src/api/unified.rs` should call
   `request_builder_from_dialog_template` with the staged headers (the
   slot already exists at `transaction/dialog/mod.rs:107`). This single
   change unblocks every outbound use case. Estimated effort: ~1 day.
2. **Implement OPTIONS authorship** in
   `transaction/utils/request_builders.rs` and wire
   `send_options_out_of_dialog_with_options`. Estimated: ~1 day.
3. **Wire `IncomingRegister` construction** in
   `session_event_handler.rs:383` (and call sites in
   `registration_adapter.rs`). Then `RegisterResponseBuilder` becomes
   reachable. Estimated: ~½ day.
4. **Implement `RegisterRefreshBuilder::send()`** by dispatching through
   `Action::SendREGISTERWithOptions` with `refresh: true`.
5. **Fix `GenericResponseBuilder::method()`** — pass the inbound method
   through the constructor.
6. **Fix Phase E callback dispatch** — invoke `on_refer_received` /
   `on_notify_received` from the dispatch loop; mark the positional-arg
   variants `#[deprecated]` with default-impl adapters that forward to
   the typed methods.
7. **Fix `CancelBuilder`** to use `send_cancel_with_options` and pass
   staged headers + reason.
8. **Replace stubbed INVITE/REGISTER action handlers** with real
   dispatch through `send_invite_with_options` /
   `send_register_with_options`.
9. **Enrich `TransferRequest` cross-crate variant with `raw_request`.**
10. **Add the §10 integration tests.** At minimum:
    - #9 `outbound_request_builders_integration` (each of 12 builders
       emits X-Test on the wire — this catches Finding #1)
    - #11 `b2bua_carry_through_integration`
    - #23 `stash_lifecycle_integration`
    - #27 `registrar_response_builder` (catches Finding #3)
    - #29 `cancel_safety_integration`
    - #28 `options_timeout` (catches OPTIONS authorship)
    Without these, regression risk is high.
11. **Fill in remaining items** — `api::bodies` module, multipart
    helpers, complete deprecations of legacy methods, decide
    `Option<Arc<Bytes>>` vs `Arc<Bytes>` (or document the lean-mode
    reason), reconcile entry-point naming (`subscribe_event`
    vs `subscribe`).

---

## Bottom line

What was shipped is approximately *the API of the design without the
behavior of the design*. The skeleton compiles and the type system
makes a B2BUA developer happy at IDE-completion time, but the wire
output for any non-default header attached via the new builders is
identical to today's legacy methods.

The "developers said they finished" claim is not supportable against
the design spec; treat this as **Phase A complete, Phases B–E
surface-only**. Estimated effort to actually complete: 2–3 engineer-weeks
for the code in remediation steps 1–9, plus a similar amount for the
test suite in step 10.
