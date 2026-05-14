# SIP_API_DESIGN_2 — Remaining Work Plan

**Date:** 2026-05-13
**Spec under remediation:** [`SIP_API_DESIGN_2.md`](./SIP_API_DESIGN_2.md)
**Predecessor docs:**
[`SIP_API_DESIGN_2_AUDIT.md`](./SIP_API_DESIGN_2_AUDIT.md) (2026-05-10),
[`SIP_API_DESIGN_2_GAP_PLAN.md`](./SIP_API_DESIGN_2_GAP_PLAN.md) (2026-05-10),
[`SIP_API_DESIGN_2_GAP_PLAN_AUDIT.md`](./SIP_API_DESIGN_2_GAP_PLAN_AUDIT.md) (2026-05-11),
[`SIP_API_DESIGN_2_COMPLETION_AUDIT.md`](./SIP_API_DESIGN_2_COMPLETION_AUDIT.md) (2026-05-13).

---

## Context

The goal is a developer-friendly `rvoip-sip` public API that covers four
use-case classes:

1. **SIP endpoint** — softphones, IVRs, test clients.
2. **SIP gateway** — protocol translation, trunk-side concentrators.
3. **Call-center server** — agent presence, blind/attended transfer,
   subscription-driven UI.
4. **SBC / B2BUA** — topology-hiding, two-leg bridging,
   header rewrite, redaction.

`SIP_API_DESIGN_2.md` is the canonical spec. The 2026-05-11 audit
(`SIP_API_DESIGN_2_GAP_PLAN_AUDIT.md`) flagged 21 findings (F1–F21)
against the original implementation. The 2026-05-13 completion audit
(`SIP_API_DESIGN_2_COMPLETION_AUDIT.md`) and the 2026-05-13 working
session closed most of them. This plan reconciles those reports against
the current code state and lists what is genuinely still open.

---

## Ground-truth status (verified via direct code reads, 2026-05-13)

### Closed and verified

| Finding / Phase | Where to confirm |
|---|---|
| F1 narrow (REGISTER auth retry threads extras) | `state_machine/actions.rs:84-105` snapshots via `.as_ref()` before retry loop |
| F2 (`SessionError::Conflict` enforcement) | `state_machine/executor.rs:225-232` centralizes the guard in `stage_outbound_options`; `stash_lifecycle_integration` and `conflict_guard_integration` tests pass |
| F3 (`HeaderPolicy::validate_outbound` on every mirror) | `adapters/dialog_adapter.rs:2520` `apply_outbound_extras_policy` combinator; 11 call sites (lines 1318/1343/1369/1394/1419/1443/1469/1488/1507/1549/1565) |
| F4 (per-leg outbound proxy `prepend_outbound_proxy_route`) | Same combinator runs for all 12 methods; INVITE path at `dialog_adapter.rs:1015` |
| F5 (`OutboundCallBuilder.send()` routes through state machine) | `api/send/outbound_call.rs:189+` builds `OutboundCallOptionsSnapshot`, stages, dispatches `SendOutboundInvite` |
| F7 (`TraceRedactor`) | `api/trace_redactor.rs`; `dialog/transport/trace.rs:24`; `transport/mod.rs:315` |
| F12 (auto-emit on CANCEL/NOTIFY) | `actions.rs:1820` CANCEL, `:1843` NOTIFY consult `auto_extras` |
| F13 (SendBYE stash-wins precedence) | `actions.rs:1803` `pending_bye_options.take()` before auto-emit fallback |
| F14 (residual `raw_request: None` in production publish) | Fixed in 2026-05-13 session: `rvoip-sip-dialog/src/events/adapter.rs:282`, `events/event_hub.rs:250` now use `Message::Request(...).to_bytes()` (`Request::Display` was lossy for empty-body messages) |
| F16 (smoke tests for 6 builders) | `tests/outbound_request_builders_integration.rs` now covers 7 methods |
| F17 (multipart helpers) | `api/headers/convenience.rs:111-201` |
| F18 (`with_target_dialog` RFC 4538 formatting) | `api/send/refer.rs:56-75` |
| F19 (stale `unified.rs` comment block) | Deleted in 2026-05-12 sweep |
| F20 (`IncomingRequest::with_request` dead-code) | Used by `stream_peer.rs:802`, `session_event_handler.rs:444` |
| F21 (deprecation-table CI guard) | `tests/deprecation_table.rs` strengthened |
| Phase 12 (`subscribe`/`register` rename) | `unified.rs:1240`/`:1324` canonical; `subscribe_event`/`register_builder` deprecated aliases |

### §10 tests added in the 2026-05-13 session

- `tests/b2bua_carry_through_integration.rs` (§10 #11 litmus — uses a
  synthetic `SipHeaderView` until the two-coordinator harness lands)
- `tests/stash_lifecycle_integration.rs` (§10 #23 sub-cases (a) and (c))

Both pass; verified against Asterisk 20.9.3 and FreeSWITCH 1.10.12 PBX
matrix (registration / basic_call / blind_transfer, UDP + TLS) with
zero rvoip errors and zero PBX errors.

---

## What is still open

### A. Architecturally deferred

Documented as deferred in the completion audit with load-bearing
reasons. These are not bugs in shipped code; they are spec-level
scope decisions.

1. **Phase 2 — OOB builders through the state machine.**
   OPTIONS, MESSAGE, initial REGISTER, initial SUBSCRIBE bypass
   `Action::Send*WithOptions` and call `DialogAdapter::send_*_oob_with_options`
   directly. Architectural uniformity gap; wire output is correct.
   - Files: `api/send/options.rs`, `message.rs`, `register.rs::send`,
     `subscribe.rs::send`.
   - Use-case impact: low. The wire is right; what's missing is the
     observability hook that "every outbound is tracked by the state
     machine."

2. **Phase 4 — Per-method `ClearPending<Method>Options` YAML rows.**
   The 10 non-INVITE/non-REGISTER `Send*WithOptions` handlers use
   `.take()` (`actions.rs:1803/1814/1835/1846/1875/1882/1889/1896/1903/1910`),
   which clears the stash on dispatch. Explicit YAML clear rows would
   be no-ops today.
   - Becomes load-bearing only once auth-retry generalizes beyond
     INVITE/REGISTER (Phase R2 below).

3. **Phase 6 — Deep `subscription_id` routing (RFC 6665 §4.5.2).**
   Today's plumbing is shallow: `NotifyRequestOptions.subscription_id`
   rides through to the `Event: pkg;id=<sid>` wire parameter
   (`rvoip-sip-dialog/src/api/unified.rs:1985-1988`), but the internal
   subscription manager still keys on `(dialog_id, event_package)`,
   not the triple `(dialog_id, event_package, subscription_id)`.
   - Use-case impact: call-center features that ride multiple
     subscriptions on a single dialog (e.g., simultaneous `dialog` +
     `message-summary` event packages) route inbound NOTIFYs to the
     wrong subscription. Single-subscription dialogs (the common
     case) work correctly.
   - This becomes **Phase R5** below.

### B. §10 verification suite — 19 of 24 still `#[ignore]`d

`tests/sip_api_design_2_section_10_skeletons.rs` and adjacent files
have 7 active + 19 ignored §10 tests, plus the 2 added in the
2026-05-13 session as dedicated files. The 19 ignored tests fall into
three groups:

- **Cross-references (3)** — covered by other test files, just left
  as numbered breadcrumbs. No work needed.
- **Established-call harness pending (~10)** — need a two-coordinator
  INVITE → 200 → ACK → … pattern for re-INVITE, UPDATE, auto-emit
  CANCEL/NOTIFY, BYE stash-wins, etc. Each test is ~50–100 lines once
  the harness exists; the harness itself is ~150–200 lines.
- **Specialized harness needed (~6)** — mock registrar, auth-challenge
  UAS, redirect-follow loop, two-coordinator B2BUA. Each harness is
  ~200–400 lines.

### C. Auth-retry generalization (broader F1)

The 10 non-INVITE/non-REGISTER `Action::Send*WithOptions` handlers
consume the stash via `.take()`. If/when authentication is needed on
those methods (SBC / gateway scenarios where every outbound may be
challenged), extras and per-call overrides will be lost on the retry.

This is an architectural choice today, not a bug. Generalizing
requires switching all 10 handlers to `.clone()` and wiring the
clear at the response-resolution path (executor's terminal-response
hook).

### D. Auxiliary / non-blocking items

- **F10 (warn-on-None instrumentation)** at non-TransferRequest bridge
  sites in `event_hub.rs` — partial; only flagged on one site today.
  Low-priority diagnostic hygiene.
- **DoD #7 (per-finding `Closes: G<n>` PR markers)** — n/a since the
  remediation landed as a single sweep, not a PR sequence.
- **Sample / example crates** for the four target use-cases (endpoint,
  gateway, call-center, SBC) — none exists today. Crate-level rustdoc
  (`lib.rs:310-419`) has the 5-section developer block, but no
  `examples/sbc/` or `examples/call_center/` walkthrough.
- **Stateless SIP proxy mode** — explicitly out of scope per spec §14
  (this round confirms stateful B2BUA covers the SBC use-cases).

---

## Use-case relevance matrix

| Gap | SIP endpoint | Gateway | Call-center | SBC / B2BUA |
|---|---|---|---|---|
| A1 (OOB through state machine) | low | low | low | low (observability only) |
| A2 (Clear YAML rows) | none | none | none | none |
| A3 (subscription_id deep routing) | low | low | **high** (BLF, MWI) | medium |
| B (§10 test gaps) | medium | medium | medium | **high** (B2BUA carry-through is litmus) |
| C (auth-retry on 10 methods) | low | **high** (challenged trunks) | medium | **high** (authenticated egress) |
| D (example crates) | medium | medium | medium | **high** (cookbook value) |

---

## Committed scope this round: R1 + R2 + R3 + R4

User-confirmed: full in-scope close (~7 engineer-weeks total),
followed immediately by **R5** as the next round.

### Phase R1 — Test scaffolding (~1 engineer-week)

Build the shared harness so half the ignored §10 tests become
reachable:

1. **`tests/support/mod.rs`** — shared two-coordinator established-call
   helper (boot alice + bob with `CallbackPeer` `AutoAccept`, INVITE,
   wait `CallAnswered`). Reusable across tests. Pattern lifted from
   `tests/outbound_request_builders_integration.rs:230-263` and
   `tests/stash_lifecycle_integration.rs:71-89`.
2. Author the established-call-dependent tests against the harness:
   - `auto_emit_cancel_carries_headers` (§10 #16)
   - `auto_emit_notify_carries_headers` (§10 #17)
   - `bye_stash_wins_over_auto_emit` (§10 #18)
   - `in_dialog_update_smoke` (§10 #8)
   - `in_dialog_reinvite_smoke` (§10 #9)
   - Tighten the 3 cross-references that point at this suite.

**Output:** §10 active tests 7 → ~17, ignored 19 → ~9.

### Phase R2 — Auth-retry generalization (~3–5 engineer-days)

Switch the 10 non-INVITE/non-REGISTER `Send*WithOptions` handlers from
`.take()` to `.clone()` and wire stash clearing at response-resolution
(in `state_machine/executor.rs` post-final-response).

**Files:**
- `state_machine/actions.rs:1803` (BYE)
- `state_machine/actions.rs:1814` (CANCEL)
- `state_machine/actions.rs:1835` (REFER)
- `state_machine/actions.rs:1846` (NOTIFY)
- `state_machine/actions.rs:1875` (INFO)
- `state_machine/actions.rs:1882` (UPDATE)
- `state_machine/actions.rs:1889` (re-INVITE)
- `state_machine/actions.rs:1896` (MESSAGE)
- `state_machine/actions.rs:1903` (OPTIONS)
- `state_machine/actions.rs:1910` (SUBSCRIBE)
- Plus executor terminal-state hook (mirror the existing INVITE
  clear at `actions.rs:2073-2106`).

**Verification:** new `tests/builder_auth_retry_preserves_headers.rs`
cases for SUBSCRIBE, MESSAGE, OPTIONS, re-INVITE.

This is the most user-visible architectural close because it makes
the SBC/gateway "authenticated-everything" pattern correct.

### Phase R3 — Specialized harnesses (~2 engineer-weeks)

Build:

- **Mock-registrar harness** → unblocks `third_party_register_integration`,
  `registrar_response_builder` (§10 #19, #27).
- **Auth-challenge UAS harness** → unblocks any `auth_challenge_*`
  scenarios.
- **Two-coordinator B2BUA harness** → unblocks
  `b2bua_carry_through_integration` end-to-end. Today's version uses
  a synthetic `SipHeaderView`; with the real harness, drive
  `with_headers_from(&incoming_call, ...)` end-to-end now that
  `IncomingCall::raw_request()` round-trips correctly (fixed in the
  2026-05-13 session).

**Output:** §10 active tests ~17 → 24. DoD gate 3 closes.

### Phase R4 — Sample crates for the four use-cases (~1 engineer-week each)

Add `examples/` walkthroughs that exercise the API end-to-end against
the local Asterisk container:

- **`examples/endpoint_softphone/`** — register, place call,
  hold/resume, send DTMF, blind transfer.
- **`examples/gateway_pstn/`** — receive INVITE on UDP trunk,
  originate TLS leg with carry-through, bridge media, BYE both legs.
- **`examples/call_center_agent/`** — register, accept call via
  `CallbackPeer`, present custom hold music via SDP renegotiation,
  blind-transfer to colleague.
- **`examples/sbc_topology_hiding/`** — receive INVITE, strip Privacy,
  rewrite PAI per trust-boundary pattern (§11.3), forward with
  trace-redactor scrubbing Authorization.

Each example should:
- Use only the canonical builder API (no `#[deprecated]` calls).
- Have a README mapping it to the §11.x pattern(s) it demonstrates.
- Be runnable via `cargo run --example <name>` against the local
  Asterisk container under `~/Developer/asterisk/`.

### Phase R6 — Migrate in-tree examples to the builder API (~1 engineer-week)

Today's audit found **112 deprecation warnings** when building
`cargo build -p rvoip-sip --tests --examples --features dev-insecure-tls`.
The breakdown:

- **Examples (~28 files):** every `stream_peer/*`, `callback_peer/*`,
  `regression/*`, `unified/*`, `sip_client/*`, and the older
  `endpoint/02_*` still use `coord.make_call(...)`, `peer.call(...)`,
  `coord.send_refer(...)`, `coord.reject_call(...)`, etc.
- **Tests (15 files):** intentionally left as legacy callers so the
  deprecated surface keeps regression coverage. **Stays as-is.**
- **Library internals (19 hits):** `adapter.rs`, `server/transfer.rs`,
  `server/b2bua.rs` still call deprecated methods from internal
  back-compat plumbing; `api/incoming.rs`, `api/handle.rs`,
  `api/callback_peer.rs`, `api/stream_peer.rs`, `api/endpoint.rs`,
  `api/simple.rs`, `api/respond/{generic,redirect}.rs`,
  `api/unified.rs` are mostly the deprecated wrappers
  forwarding into the new builder API (correct by spec §10).

**Scope (Phase R6):**

1. **In-tree examples (~28 files).** Walk each example and replace
   the spec §11.4 migration-table "Today" call sites with the
   "Tomorrow" builder chain:
   - `coord.make_call(target)` → `coord.invite(None, target).send().await`
   - `coord.make_call_with_auth(target, creds)` → `coord.invite(None, target).with_credentials(creds).send().await`
   - `coord.make_call_with_pai(target, pai)` → `coord.invite(None, target).with_pai(pai).send().await`
   - `coord.make_call_with_headers(target, hdrs)` → `coord.invite(None, target).with_headers(hdrs)?.send().await`
   - `peer.call(target)` → `peer.invite(target).send().await`
   - `peer.call_with_headers(target, hdrs)` → `peer.invite(target).with_headers(hdrs)?.send().await`
   - `endpoint.call(target)` → `endpoint.invite(target).send().await`
   - `coord.send_refer(&session, target)` → `coord.refer(&session, target).send().await`
   - `coord.send_refer_with_replaces(&session, target, replaces)` → `coord.refer(&session, target).with_replaces(replaces).send().await`
   - `coord.send_notify(&session, event, body, state)` → `coord.notify(&session, event).with_body(body).with_subscription_state(state).send().await`
   - `coord.send_info(&session, ctype, body)` → `coord.info(&session, ctype).with_body(body).send().await`
   - `coord.hangup_with_reason(&session, reason)` → `coord.bye(&session).with_reason(reason).send().await`
   - `coord.reject_call(&session, code, reason)` → `coord.reject(&session).with_status(code).with_reason(reason).send().await`
   - `coord.redirect_call(&session, code, contacts)` → `coord.redirect(&session).with_status(code).with_contacts(contacts).send().await`
   - `coord.register_with(reg)` → `coord.register(reg.registrar, reg.user, reg.pw).with_expires(reg.expires).send().await`

2. **PBX runner `examples/pbx/common.rs` (9 hits).** Migrate the
   shared scenario helpers so the matrix runner stops emitting
   the deprecation warning spew we see in every PBX test run.

3. **Internal callers (`adapter.rs:217/250/290`, `server/transfer.rs:34/51`,
   `server/b2bua.rs`).** These are internal forwarders, not part of
   the deprecated surface itself. Migrate them to call the new
   builders directly so the deprecated methods become true leaf
   forwarders with zero internal callers (easier to remove in the
   next breaking release).

**Out of scope for R6:**
- The 15 test files that exercise the deprecated surface
  intentionally. They keep validating that legacy callers still
  compile and run; that is correct regression coverage for external
  apps that haven't migrated.
- Removing the `#[deprecated]` methods themselves — that is a
  separate breaking-release cleanup, governed by the deprecation
  cycle promised in spec §9 Phase C.

**Verification:**
- `cargo build -p rvoip-sip --tests --examples --features dev-insecure-tls 2>&1 | grep -c '^warning: use of deprecated'`
  drops from **112 → ~15** (just the legacy-surface regression
  tests). The 15 remaining hits are auditable line-by-line.
- Every migrated example still runs against the local Asterisk
  container without behavior change.
- PBX matrix
  (`crates/rvoip-sip/examples/pbx/run.sh --pbx asterisk --api all --scenario all`)
  passes with no deprecation warnings.

**Use-case impact:** high for adoption. The 28 example files are
what new developers read and copy. Today they teach the legacy
shape; after R6 they teach the canonical shape.

---

## Phase R5 — RFC 6665 multi-subscription routing (next round)

User-confirmed as the immediate next round after R1–R4 land. Needed
for call-center BLF / voicemail-MWI / any shared-dialog event-package
multiplex.

**Scope:**
- Refactor `rvoip-sip-dialog`'s subscription manager to key on the
  triple `(dialog_id, event_package, subscription_id)` instead of the
  pair `(dialog_id, event_package)`.
- Thread `subscription_id` through inbound NOTIFY routing so the
  right subscription record receives each NOTIFY.
- Update `send_notify_with_options` consumers in
  `rvoip-sip-dialog/src/api/unified.rs:1985+` to look up subscription
  state via the triple.
- Author dedicated test `tests/notify_subscription_id_routing.rs`
  (§10 #19) replacing today's `#[ignore]` skeleton.

**Out-of-scope decisions confirmed:**
- **Stateless SIP proxy mode** stays explicitly out of scope per
  spec §14. Stateful B2BUA (every leg = a session) covers the SBC
  use-cases this design targets.
- **Phase 2 (OOB through state machine)** and **Phase 4 (Clear YAML
  rows)** remain deferred. They are observability / cleanup items
  without user-visible behavior gaps; revisit only when a concrete
  driver emerges.

**Estimate:** separate ~2 engineer-week design + implementation pass;
plan as its own document once R1–R4 ship.

---

## Verification

After each phase:

- `cargo test -p rvoip-sip` — all suites green.
- `cargo test -p rvoip-sip-dialog --lib` — 280/280 still pass.
- `cargo test --doc -p rvoip-sip` — doctest count holds (currently 223).
- After R1/R3: `cargo test -p rvoip-sip --test sip_api_design_2_section_10_skeletons`
  shows the expected active/ignored split.
- After R4: each new example builds
  (`cargo build --examples -p rvoip-sip --features dev-insecure-tls`)
  and runs end-to-end against the asterisk container
  (`~/Developer/asterisk/scripts/up.sh`).
- PBX matrix sanity:
  `crates/rvoip-sip/examples/pbx/run.sh --pbx both --api callback --scenario all`
  — every cell green (proven 2026-05-13 against Asterisk 20.9.3 and
  FreeSWITCH 1.10.12 for the registration / basic_call /
  blind_transfer subset).

---

## Bottom line

After **R1–R4 + R6** land, all seven DoD gates of `SIP_API_DESIGN_2_GAP_PLAN.md`
§6 pass simultaneously, the four target use-cases have working
documented examples, the auth-retry gap that affects gateway / SBC
operators is closed, and every in-tree example teaches the canonical
builder API (deprecation warnings drop from 112 → ~15, all in
intentional regression tests). **R5** (RFC 6665 multi-subscription
routing) is the immediate follow-on round. Phase 2, Phase 4, and
stateless proxy mode stay out of scope.
