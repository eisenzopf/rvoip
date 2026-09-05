# SIP Signaling Single-Authority Gap Analysis and Implementation Plan

- **Status:** Complete; source cleanup shipped and protected qualification
  passed in 0.3.9 (authorized 2026-07-20, qualified 2026-09-05)
- **Historical implementation baseline inspected:** 2026-07-20
- **Historical baseline commit:** `0df3e5ba7b29ce4dc0c641b36381aefcd4b66925`
- **Companion architecture decision:** [SIP_SIGNALING_SINGLE_AUTHORITY_CLEANUP_PLAN.md](SIP_SIGNALING_SINGLE_AUTHORITY_CLEANUP_PLAN.md)

> **Evidence status:** **PASS.** Source symbols, deletion fences, and test
> sources were reconciled on 2026-07-21. Protected run
> [`33969263241`](https://github.com/eisenzopf/rvoip/actions/runs/33969263241)
> completed the final exact-candidate Cargo, performance/soak,
> interoperability, source-integrity, and attestation scopes. Section 21 is
> retained as the qualification procedure and historical decision record.

## 1. Purpose and Required Outcome

This is a cleanup plan, not a rewrite plan. The existing architecture is close
enough to the intended design and is retained:

- runtime YAML loading through **YamlTableLoader**;
- **StateMachine** and its per-session execution lane;
- **SessionStore** and the exact-lifetime **SessionRegistry**;
- typed **DialogToSessionEvent** delivery;
- dialog and transaction ownership of SIP wire mechanics;
- transport ownership of DNS, flows, and I/O;
- media ownership of SDP/RTP resources; and
- observational event publication.

The cleanup succeeds when each application-visible call or registration
lifecycle mutation has one exact-session writer, each SIP request has one wire
implementation, delayed work is generation-fenced, and reporting can be
removed, blocked, or saturated without changing signaling results.

Every implementation slice must delete or make unreachable at least one
duplicate writer, map, handler, routing hop, or compensation path. A slice that
requires replacing the state machine, registry, store, dialog transaction
layer, or public API must stop and be escalated for architectural review.

## 2. Baseline Findings

### 2.1 Retained architecture is reusable

The current tree already contains the necessary authority primitives:

| Existing primitive | Current location | Finding |
|---|---|---|
| Runtime state-table source | **state_tables/default.yaml**, **state_table/yaml_loader.rs** | Retained. Runtime selection, compilation, validation, reachability, and selected-source hashing remain the authority; this document does not freeze incidental row counts. |
| Per-session transition lane | **session_store/state.rs: SessionStateCell::state_machine_lane** | Retain. The lane is attached to the exact session cell, so reused public session IDs receive a different lane. |
| Exact identity | **session_registry.rs: SessionRegistryHandle** | Retain and carry through every callback, timer, and cleanup task. |
| Transition executor | **state_machine/executor.rs: StateMachine::process_event_inner** | Retain. It already serializes queued internal events and accepts a private event-state input. |
| Typed dialog ingress | **adapters/session_event_handler.rs: SessionCrossCrateEventHandler** | Retain. It already receives typed cross-crate variants. |
| Bounded keyed ingress | **adapters/session_event_handler.rs: DialogToSessionDirectRouter** | Retain. It already shards by session and provides authoritative acknowledgements for response-bearing events. |
| Dialog request tracking | **adapters/outbound_request_tracker.rs: OutboundInDialogRequestTracker** | Retain. It is the exact request/transaction correlation authority for in-dialog retries and completion. |
| Event-plane separation primitives | **infra-common/src/events/coordinator.rs** | Retain **dispatch_authoritative_handler**, **publish_authoritative**, and **publish_observational**; remove ambiguous use of **publish** from causal paths. |
| SIP ownership manifest | **state_table/wiring_manifest.rs** | Retain and strengthen as an executable architecture manifest. |

### 2.2 Source cleanup closure

The 2026-07-21 source reconciliation closes the duplicate-authority indicators
from the historical baseline:

- **EventStateInput** now carries typed SDP, authentication, transaction,
  transfer, and outbound-option data, while **ResponseStateInput** carries the
  complete status/SDP/header response envelope under the exact-session lane.
- **process_one_event** keeps one lane-owned working snapshot across actions
  and publishes it through **commit_lane_state** once on the success branch or
  once on the action-failure branch. Only narrow wire-required identity/resource
  publication remains outside that final publication. Public
  **SessionStore::update_session**, **update_session_with**, and
  **update_session_snapshot_with** retain their signatures but acquire the
  exact state-machine lane before mutation; there is no competing-writer
  reconciliation path.
- **merge_tracked_request_staging**, **preserve_auth_coordination**,
  **RegistrationStateProjection**, and **sync_registration_state** are gone.
  REGISTER's four public/runtime action shapes delegate to one typed attempt,
  result, and post-commit-effect implementation.
- Response APIs and incoming-call/response builders carry exact lifecycle
  handles and typed response envelopes. Queued dialog events, request tracker
  entries, deferred completions, timers, and retained tasks revalidate the
  captured generation after waits.
- re-INVITE glare and REFER grace work run through retained exact scheduling;
  no state-machine action sleeps while holding the transition lane.
- session-affecting dialog ingress uses acknowledged typed delivery through
  **DialogToSessionDirectRouter**. The 28 debug-string wrappers and four
  extractors are deleted, and observational publication is neither causal nor
  release-blocking.
- adapter forward/reverse dialog and media compatibility maps are deleted.
  **SessionRegistry** owns exact cross-layer associations, dialog-core owns
  protocol routing, and media owns exact managed resources.
- initial INVITE delegates to **send_initial_invite_staged**; standalone
  MESSAGE, OPTIONS, and SUBSCRIBE delegate to one typed standalone auth and
  transaction driver. The exact in-dialog tracker remains the intentional
  owner for in-dialog request correlation.

The remaining boundaries are intentional resource boundaries, not alternate
SIP lifecycle implementations: protocol-owned dialog and transaction indexes
and the retained active-media deadline scheduler. Public SessionStore writes
are exact-lane serialized, and the non-owning bridge metadata facades no
longer fabricate success.

### 2.3 Beta evidence baseline

The archived 2026-07-20 interop and security modes passed, including local
Asterisk, local FreeSWITCH, SIPp, strict-UA, dependency audit, and parser fuzz
smokes. They were recorded at revision 85b932e4 from a dirty tree and therefore
do not attest the current baseline.

The archived monolithic performance soak failed after 5,016 offered calls:

- 4 call failures;
- 3 media setup failures;
- 1 teardown failure;
- 27 retained objects after drain; and
- 1 active Bob audio receiver after drain.

The archived log reported zero active transaction managers/runners after drain,
which narrows the retained-resource investigation toward exact session,
dialog, media, and receiver ownership rather than a still-running transaction
engine. The mutable file **target/perf-results/perf_soak_30min.json** must not be
treated as evidence for that archived run unless its hash appears in the same
run's attestation.

These results motivate the cleanup and its release tests; they do not prove
that any single listed code path caused every observed failure.

### 2.4 Current evidence limits

The existing evidence is useful but narrower than a complete SIP profile:

- the PBX matrices prove the scenarios they execute, but do not constitute a
  complete MESSAGE, OPTIONS, SUBSCRIBE/NOTIFY, UPDATE, PRACK, and session-timer
  interoperability matrix;
- the current SIPp load scenario primarily exercises INVITE/ACK/BYE behavior
  and must not be cited as functional evidence for unrelated methods;
- the baresip run provides one strict-UA baseline, not broad multi-UA
  coverage;
- the archived `0.2.5` Kamailio/OpenSIPS result was only a de-scope audit and
  is not evidence for the new mandatory `0.3.4` real-peer matrix; and
- ignored resilience stubs, message-construction tests, and parser tests do
  not by themselves prove an end-to-end lifecycle claim.

TE-703 must either attach executable evidence at the strength claimed or
downgrade the corresponding documentation. Adding a new product feature or a
new topology is not implied by this cleanup.

### 2.5 Reconciled work-item status

The `Current` and `Gap` fields in sections 6–13 preserve the historical reason
for each slice. This table is the current source disposition and supersedes
future-tense wording in those fields.

| Work items | Source disposition on 2026-07-21 | Qualification status |
|---|---|---|
| FZ-001–FZ-004 | Implemented: public-API snapshots/fixtures, architecture fences, baseline manifest/attestation tooling, and release metadata alignment are present. | Final tool-backed API/downstream execution **PENDING**. |
| IN-101–IN-105 | Implemented: typed transition/response input, atomic private dispatch, exact ingress handles, and generation-fenced retained work are present. The documented active-media scheduler boundary is retained. | Full deterministic race execution **PENDING**. |
| EX-201–EX-206 | Implemented: one working state/final commit, typed action results, REGISTER ownership, merge/projection deletion, exact-lane public-store serialization, off-lane delays, and release/observer separation are present. Non-owning bridge facades fail closed while `BridgeHandle` remains the real RTP lifetime owner. | Cargo/concurrency/drain execution **PENDING**. |
| RT-301–RT-306 | Implemented: typed acknowledged ingress, startup ordering fences, observational reporting, debug-string deletion, direct response operations, and exact in-dialog tracking are present. | Observer/routing matrix execution **PENDING**. |
| PR-401–PR-405 | Implemented for the approved contexts: canonical initial INVITE, REGISTER, and MESSAGE paths; immutable auth snapshots; one standalone MESSAGE/OPTIONS/SUBSCRIBE driver; canonical registry mappings; exact timer ownership. Intentionally distinct dialog contexts remain distinct. | RFC wire, PBX, strict-UA, timer, and performance execution **PENDING**. |
| YA-501–YA-504 | Implemented: bidirectional wiring/reachability, lifecycle decision audit, orphan deletion, exact selected-YAML evidence, and 24 explicitly owned compatibility shapes. Runtime grammar is unchanged. | Runtime configured-YAML and full gate execution **PENDING**. |
| AP-601–AP-603 | Implemented in source: post-commit compatibility projection, downstream/API gate inventory, and public-behavior fixtures are present. | Full downstream and behavior matrix execution **PENDING**. |
| TE-701–TE-704 | Test sources, observer isolation fixtures, RFC evidence mapping, and owner-specific drain diagnostics are present. | Required test runs and release results **PENDING**. |
| TE-705–TE-706 | Performance recipes, mode-specific reporting, and attestation generation/verification tooling are present. | Canonical 2K, full beta/perf/soak/PBX runs and final attestation **PENDING**. |

## 3. Non-Negotiable Invariants

1. YAML remains the authority for legal application-visible call and
   registration lifecycle transitions, guards, ordered actions, retry
   decisions, and cleanup decisions.
2. Dialog and transaction code remains the authority for tags, route sets,
   CSeq, transaction correlation, retransmission timers, ACK, CANCEL, PRACK,
   response matching, and standalone transaction-oriented requests.
3. **SessionRegistryHandle** is the identity of a session lifetime. A raw
   **SessionId** is only a lookup key and must not authorize delayed mutation.
4. The state-machine lane protects one complete transition, including the
   canonical commit. No transition participant may publish a competing
   session snapshot.
5. A wire request is materialized once. Authentication retry adds the
   appropriate authorization header to the retained immutable request options;
   it does not reconstruct semantically different request state.
6. Reporting and observation occur only after causal handling or committed
   state. Observation failure cannot suppress a SIP response, transition,
   terminal release, or timer.
7. Dialog-core owns its internal dialog maps; **SessionRegistry** owns the
   cross-layer exact-session association. Adapter indexes may be resource
   stores or temporary compatibility views, never independent authorities.
8. Standards compatibility is retained. Legacy wire behavior required for
   RFC-compliant peers is not classified as legacy code merely because it
   supports older algorithms or peer behavior.
9. There is no dual-path release. Once a replacement cleanup path passes its
   tests, the superseded path is deleted in the same slice or immediately
   following deletion-only slice.

## 4. Public API Compatibility Contract

### 4.1 Frozen surface

Before internal cleanup, record and continuously compare:

- every public item under **src/api**, including structures, fields, enums,
  variants, traits, builders, methods, arguments, return types, trait bounds,
  feature gates, and error types;
- crate-root and **prelude** reexports in **src/lib.rs**;
- public event variants and their delivery semantics;
- **Config**, including **Config::state_table_path**;
- **Endpoint**, **StreamPeer**, **CallbackPeer**, **SessionHandle**, and
  **UnifiedCoordinator** behavior;
- public **UnifiedCoordinator::stage_outbound_options** and
  **UnifiedCoordinator::dispatch_outbound**; and
- examples and downstream crates that compile against those surfaces.

No item in this frozen surface may be renamed, removed, moved, narrowed, made
more restrictive, or changed semantically in this cleanup.

The crate's doc-hidden modules are not an excuse to accidentally change a type
appearing in a frozen signature. An internal public item may be removed only
when the API snapshot, downstream fixtures, and repository-wide caller scan
all prove it is outside the frozen surface. Otherwise it requires a separate
versioned API decision.

### 4.2 Compatibility facade rule

The public two-call staging flow remains source- and behavior-compatible.
Internally:

- ordinary builders call one private atomic stage-and-dispatch operation;
- public **stage_outbound_options** retains the current conflict and lifetime
  semantics;
- public **dispatch_outbound** claims only the exact staged value and returns
  the same public error classes; and
- neither public method may reintroduce a writer outside the lane.

Existing compatibility event variants, such as detailed and non-detailed call
events, remain. They become projections of one committed canonical outcome
rather than independent inputs.

## 5. Work-Item Rules

Each work item below is independently reviewable. A pull request description
must repeat:

- the current writer/route being removed;
- the retained authority;
- the proof that deletion is safe;
- the public API diff result;
- the new or migrated tests; and
- the rollback boundary.

A deletion precondition is mandatory. “No known users” is not sufficient:
there must be a repository-wide zero-caller scan, API snapshot result, and
relevant runtime test.

## 6. Phase 0 — Freeze Compatibility and Add Architecture Fences

### FZ-001 — Record the supported public API

- **Current:** **src/api**, root reexports, prelude reexports, and examples are
  the de facto contract; no checked API baseline is stored.
- **Gap:** internal cleanup can accidentally alter a public generic bound,
  variant, builder return type, feature-gated method, or reexport.
- **Cleanup:** generate a checked public API baseline with
  **cargo-public-api** and run **cargo-semver-checks** against it. Add external
  compile fixtures that import only published paths. Include all supported
  feature combinations used by docs and release gates.
- **Dependencies:** none; this is the first implementation change.
- **Deletion precondition:** no public or doc-hidden item is deleted until this
  baseline and the external fixtures pass.
- **API impact:** none; this item detects changes.
- **Tests:** API snapshot equality, semver check, docs.rs feature build, crate
  root/prelude compile fixture.
- **Complete when:** CI fails on an intentional test mutation to every category
  above and passes again after the mutation is reverted.
- **Risk/rollback:** tooling instability only; pin tool versions and keep the
  previous CI checks while introducing the new job.

**Completion finding (2026-07-20):** the compiler-only external fixture is a
mandatory local/full beta gate through **scripts/check_public_api.sh**. The
checked default and documentation-feature snapshots use a pinned
**cargo-public-api**/nightly pairing, and **cargo-semver-checks** compares the
approved baseline revision when those optional tools are present.
**RVOIP_REQUIRE_API_TOOLS=1** makes both optional tools mandatory without
installing or silently accepting a different toolchain.

### FZ-002 — Add a single-authority static guard

- **Current:** direct store writes, general event publication, string parsing,
  and routing maps can be added anywhere without an architecture failure.
- **Gap:** duplicate paths can return during later feature work.
- **Cleanup:** add **tests/single_authority_architecture.rs** with source-level
  inventories and explicit, shrinking allowlists for:
  **update_session_with**, **update_session_exact_with**, full snapshot
  replacement, **GlobalEventCoordinator::publish** on causal event types,
  **event_str** parsing, raw-ID delayed tasks, and adapter routing-map writes.
- **Dependencies:** FZ-001.
- **Deletion precondition:** none; initial allowlists describe the current tree.
- **API impact:** none.
- **Tests:** the guard itself plus mutation tests in which one forbidden call
  is temporarily introduced.
- **Complete when:** every current exception has an owner work-item ID and CI
  requires the exception count to stay equal or decrease.
- **Risk/rollback:** text matching can be brittle; match stable symbol names and
  scope paths, and pair it with behavioral tests rather than treating it as a
  semantic compiler.

### FZ-003 — Freeze failure and performance baselines

- **Current:** archived evidence belongs to revision 85b932e4, while the
  implementation baseline is 0df3e5ba; the target JSON path is mutable.
- **Gap:** later comparisons can silently mix revisions or run modes.
- **Cleanup:** store a read-only baseline manifest containing commit, source
  fingerprint, YAML hash, effective config hash, executable hash, report/log
  hashes, and the archived metrics listed in section 2.3. Mark dirty-tree
  evidence diagnostic-only.
- **Dependencies:** none.
- **Deletion precondition:** no beta claim is updated from an unattested file.
- **API impact:** none.
- **Tests:** manifest-schema test and hash verification test.
- **Complete when:** moving a result JSON from another run causes verification
  to fail.
- **Risk/rollback:** no runtime risk; preserve the original external evidence
  directory unchanged.

### FZ-004 — Reconcile release-train metadata

- **Current:** the workspace package version is **0.2.5**, while comments in
  the root and **rvoip-sip** manifests and the beta release checklist still
  contain **0.2.2**. Release notes also distinguish cleanup from feature work
  planned for a later minor release.
- **Gap:** reports and reviewers can infer different release trains from
  source-controlled metadata.
- **Cleanup:** make release tooling derive the runtime crate version from
  Cargo metadata, update stale source-controlled comments/checklists when the
  cleanup is implemented, and keep feature-release statements separate from
  this compatibility-preserving cleanup.
- **Dependencies:** FZ-003.
- **Deletion precondition:** none; this changes documentation and report input,
  not runtime signaling.
- **API impact:** none; no package/API version change is authorized here.
- **Tests:** CI assertion that manifests, release checklist, release report,
  and attestation agree with Cargo metadata.
- **Complete when:** no active release artifact or comment identifies the
  cleanup baseline as **0.2.2**, and no implementation plan silently moves the
  work to a different release train.
- **Risk/rollback:** documentation-only risk; Cargo metadata remains the
  authority.

**Completion finding (2026-07-20):** active beta release-candidate metadata,
dependency examples, detached example manifests, and inherited-version
comments now agree with the workspace **0.2.5** package version. Wording says
“release candidate in this checkout” where publication is not established by
repository evidence. No crate version was changed. The attestation continues
to read and independently verify the runtime package version from captured
Cargo metadata, while a source test rejects a reintroduced stale beta version
in the active release surfaces.

## 7. Phase 1 — Close State-Mutation Bypasses

### IN-101 — Carry transition input through the existing lane

- **Current:** **state_machine/executor.rs: EventStateInput** carries only
  **remote_sdp**.
- **Gap:** response extras, local SDP, inbound transaction identity, auth
  correlation, and REFER metadata can be written before
  **acquire_state_machine_lane**.
- **Cleanup:** keep the private **EventStateInput** mechanism and extend it with
  typed optional fields for local SDP, reject/challenge response extras,
  inbound INVITE transaction key, auth transport/transaction/request URI, and
  REFER **Referred-By** and **Replaces** metadata. Apply all fields to the
  event-local state immediately after the exact snapshot is loaded and before
  table lookup. Existing **EventType** payloads remain unchanged.
- **Dependencies:** FZ-001 and FZ-002.
- **Deletion precondition:** delete each caller prewrite only after an
  equivalent lane-input test passes.
- **API impact:** none; **EventStateInput** remains private.
- **Tests:** fast 200/401/407/422 response before dispatch returns, accept with
  local SDP, reject/challenge extras, inbound INVITE transaction, REFER
  metadata, and old-generation input after ID reuse.
- **Complete when:** the architecture guard finds no transition-input prewrite
  outside the executor.
- **Risk/rollback:** input loss on no-transition/guard-failure rows; define and
  test whether each input is consumed, retained, or cleared on those outcomes
  before deleting the old write.

**Implemented finding (2026-07-21):** `EventStateInput` now carries the typed
transition inputs above, and `ResponseStateInput` freezes status override, SDP,
and application-authored headers as one response envelope. Exact helper entry
points apply the envelope after the lane snapshot is loaded and consume it in
the same canonical transition commit; response actions no longer assemble the
same response from independent prewrites.

### IN-102 — Add one private atomic stage-and-dispatch path

- **Current:** builders call staging and dispatch separately. Guarded variants
  reduce races for several methods, but ordinary public compatibility remains
  two-step.
- **Gap:** a builder can be cancelled between staging and dispatch, or a
  different caller can observe an occupied staging slot.
- **Cleanup:** add one private state-machine entry point that acquires the exact
  lane once, checks the outbound tracker, stages the immutable options, claims
  the exact pointer, and executes the matching event. Migrate all crate-owned
  builders to it. Retain the public two-step facade unchanged.
- **Dependencies:** IN-101.
- **Deletion precondition:** remove builder use of the two-step path only after
  method-specific conflict/cancellation tests pass.
- **API impact:** none.
- **Tests:** one concurrent same-method send returns the existing conflict;
  different methods remain independent; cancellation before, during, and
  after the wire action clears only the exact staged pointer.
- **Complete when:** only public compatibility tests and facade implementation
  call the unguarded two-step sequence.
- **Risk/rollback:** conflict timing could shift; preserve current public error
  type and method attribution and roll back builder migration per method.

**Implemented finding (2026-07-21):** crate-owned outbound builders use
`dispatch_outbound_with_options` (or the INVITE specialization) to stage and
claim one immutable exact snapshot under the lane. The public
`stage_outbound_options`/`dispatch_outbound` pair remains as the frozen
compatibility facade and materializes the same guarded dispatch behavior.

### IN-103 — Move response and API prewrites into typed events

- **Current:** **api/unified.rs**, **api/handle.rs**, and
  **adapters/session_event_handler.rs** directly set SDP, reject/challenge
  extras, pending REFER fields, and auth correlation.
- **Gap:** check-then-update and update-then-event sequences span revisions and
  can target a newer lifetime using the same raw ID.
- **Cleanup:** pass these values through IN-101, and perform consume/clear in
  the same transition commit. **accept_refer** and **reject_refer** must claim
  the exact transaction under the lane; the 500 ms default decision must use
  the same claim.
- **Dependencies:** IN-101 and IN-105.
- **Deletion precondition:** zero direct writes for each migrated field and
  parity tests for public return values and wire responses.
- **API impact:** none.
- **Tests:** simultaneous explicit/default REFER decision, duplicate decision,
  delayed reject after session reuse, challenge response header retention, and
  accept-call response racing remote BYE.
- **Complete when:** all listed fields have one executor writer and an explicit
  consume/clear rule.
- **Risk/rollback:** duplicate response risk; retain the exact transaction
  claim as the rollback boundary and never restore the old timer without its
  generation fence.

**Implemented finding (2026-07-21):** accept, provisional, reject, redirect,
challenge, and generic response APIs route through exact response-envelope
helpers. Incoming-call and response builders retain the captured lifecycle
handle, and REFER decision work carries one exact transaction claim. Stale or
duplicate decisions fail closed rather than prewriting a newly resolved raw
session ID.

### IN-104 — Resolve typed ingress to an exact handle before waiting

- **Current:** several handler branches check a raw **SessionId**, await work,
  and later look up or mutate by raw ID.
- **Gap:** an ID can be retired and reused between the check and mutation.
- **Cleanup:** resolve **SessionRegistryHandle** at ingress, carry it through
  queued work, and revalidate it after every await that precedes mutation.
  Extend **QueuedDialogToSessionEvent** with the optional exact handle resolved
  when the event is admitted; initial INVITE admission remains the exception
  because it creates the lifetime.
- **Dependencies:** FZ-002.
- **Deletion precondition:** every existing stale-ID test is converted to use
  two actual generations.
- **API impact:** none.
- **Tests:** enqueue on generation A, retire A, create B with the same ID, then
  release the queue; B must be unchanged and receive no event.
- **Complete when:** no delayed dialog/media callback authorizes mutation from
  a fresh raw-ID lookup.
- **Risk/rollback:** legitimate late terminal events may become no-ops; test
  exact release ownership separately and retain idempotent lower-layer cleanup.

**Implemented finding (2026-07-20):** typed dialog admission now captures an
optional `SessionRegistryHandle` in `QueuedDialogToSessionEvent`, and each
sharded worker revalidates that exact lifetime after queue wait and before
dispatch. Every session-addressed dialog lifecycle branch carries the same
handle into exact executor/store operations and terminal release. Initial
INVITE remains the documented creation exception; `DialogCreated`, inbound
REGISTER, standalone MESSAGE, out-of-dialog OPTIONS, and flow observations do
not manufacture state-machine lifetimes. Barrier-driven generation-A/reused-B
tests and static architecture fences prove that delayed A work cannot mutate B.
No public API surface changed.

### IN-105 — Generation-fence every delayed task

- **Current:** re-INVITE glare, REFER default acceptance, registration refresh,
  session timers, media watchdogs, and setup/teardown deadlines use mixed task
  patterns.
- **Gap:** a raw ID, state comparison, or map presence is weaker than an exact
  lifetime.
- **Cleanup:** use the existing retained-task/deadline infrastructure and carry
  **SessionRegistryHandle**, expected state, expected transition timestamp or
  request claim, and cancellation token as appropriate. Delayed work must
  revalidate all captured predicates immediately before dispatch.
- **Dependencies:** IN-104.
- **Deletion precondition:** no raw-ID **tokio::spawn** plus sleep remains for
  session lifecycle work.
- **API impact:** none.
- **Tests:** shutdown cancellation, ID reuse, state change before deadline,
  exact request replacement, and timer firing concurrently with terminal
  release.
- **Complete when:** the raw-ID delayed-task allowlist is empty.
- **Risk/rollback:** timer starvation or early cancellation; migrate one timer
  family per pull request and retain its current duration/jitter policy.

**Active-media watchdog stop-boundary finding (2026-07-20):** the current
watchdog is not an unretained raw-ID task. It is owned by the existing
`SetupTeardownDeadlineScheduler`, counted by `RetainedTasks`, interrupted by
scheduler shutdown, joined by `close_and_wait`, carries a
`SessionRegistryHandle`, revalidates before firing, and dispatches the timeout
through the exact handle. Its remaining limitation is that an individual
session teardown does not cancel a healthy watchdog while it sleeps. Moving
this indefinitely re-arming monitor into `SessionLeaseAuthority::spawn_owned_exact`
is unsafe with the retained primitives: exact operations require a finite hard
deadline, and a watchdog that performs exact release before returning would
make teardown wait for the operation that is itself performing teardown. A
new no-deadline monitor/post-completion primitive or a redesign of terminal
ownership is expressly outside this cleanup. Retain the shared scheduler and
its exact-dispatch fence unless such a foundation change is separately
approved.

## 8. Phase 2 — Consolidate Executor, Actions, and Store

### EX-201 — Establish one lane-owned working state

- **Status:** complete in the 2026-07-20 cleanup tree.
- **Current:** the executor applies the table transition and ordered action
  mutations to one lane-owned working state. Success and action-failure paths
  each publish that complete state exactly once; no pre-action full snapshot
  is visible through `SessionStore`.
- **Gap:** intermediate publication exposes partially executed transitions and
  forced the lane and merge compensation.
- **Cleanup:** actions mutate the executor's one event-local state and return
  follow-up events/effects. Commit application-visible state once after
  successful ordered actions. When a wire effect must expose identity before
  sending, commit only the exact identity through an explicit narrow
  store/registry operation, not a full session snapshot.
- **Dependencies:** all Phase 1 items.
- **Deletion precondition:** each action that currently rereads the store has a
  local-state or narrow-authority replacement.
- **API impact:** none.
- **Tests:** synchronous response during INVITE/BYE/CANCEL/REFER actions,
  action failure, guard failure, no-transition input, and history ordering.
- **Complete when:** ordinary transitions perform one full canonical commit and
  tests prove the peer can respond before the action future returns.
- **Risk/rollback:** follow-up events may require post-transition state; retain
  the existing internal event queue and enqueue only after local next state has
  been applied.
- **Implementation evidence:**
  **state_machine/executor.rs::StateMachine::process_one_event** contains no
  pre-action `commit_lane_state`; both terminal branches use the same canonical
  commit helper without a store reread or selective merge. The frozen public
  `internals::SessionStore` mutation methods acquire the exact cell's lane and
  revalidate its generation after the wait. The focused
  **async_action_exposes_no_intermediate_state_and_success_commits_once** and
  **action_failure_exposes_no_intermediate_state_and_commits_once** barrier
  tests hold an action before execution, assert that the immutable revision
  and lifecycle fields remain at the pre-event image, then assert the final
  revision advances by exactly one. Existing fast INVITE/BYE/CANCEL/REFER
  response tests cover enqueue-before-action-return behavior, and
  **public_store_writer_queues_behind_exact_state_machine_lane** proves that a
  public compatibility write cannot interleave with the executor's working
  state.

### EX-202 — Make ActionOutcome the action-to-executor boundary

- **Current:** actions both mutate their **&mut SessionState** and directly
  update the store for staging claims, SDP, retry counters, auth, and terminal
  coordination.
- **Gap:** direct updates compete with the executor's full snapshot.
- **Cleanup:** extend the existing private **ActionOutcome** with typed
  follow-up events, exact narrow-resource publications, scheduled-work
  requests, and post-commit observations. Keep ordinary state mutation on the
  passed local state. Executor applies the outcome in deterministic order:
  local mutation, required narrow publication, canonical commit, scheduled
  work, causal follow-up, then observation.
- **Dependencies:** EX-201.
- **Deletion precondition:** action-specific tests pass before deleting its
  direct store call.
- **API impact:** none.
- **Tests:** table-driven test for every **Action** variant proving its allowed
  effect class; failure injection between outcome phases.
- **Complete when:** action code cannot call full-state store replacement and
  the direct-write allowlist contains only narrow resource authorities.
- **Risk/rollback:** an oversized outcome can become a new abstraction rewrite;
  add only the four effect classes above and do not introduce a general actor
  protocol.

**Implemented finding (2026-07-21):** actions mutate the lane-owned working
state and return the existing bounded `ActionOutcome` effect classes. The
executor orders narrow protocol/resource publication, one final state commit,
retained scheduling/causal follow-up, and post-commit observation without a
replacement actor or request engine.

### EX-203 — Bring registration state into the transition commit

- **Current:** **state_machine/actions.rs: RegistrationStateProjection**
  copies fields that **DialogAdapter** writes while REGISTER awaits a response.
- **Gap:** two session writers exist for registration identity, digest count,
  transport context, retry count, and outcome.
- **Cleanup:** have the REGISTER dialog call return a typed private result with
  response metadata, transport context, auth challenge, Call-ID/CSeq values,
  and accepted expiry. The state-machine action applies that result to its
  local session. Dialog-core still creates and runs the transaction; it does
  not mutate **SessionState**.
- **Dependencies:** EX-202 and PR-402.
- **Deletion precondition:** all REGISTER, unregister, 401/407, 423, refresh,
  Service-Route, GRUU, NAT-contact, and nonce-count tests pass without
  projection reload.
- **API impact:** none.
- **Tests:** initial register, refresh, unregister, stale nonce, proxy auth,
  423 retry, shutdown during refresh, and same-ID generation replacement.
- **Complete when:** **RegistrationStateProjection** and
  **sync_registration_state** are deleted and adapter registration helpers no
  longer write session lifecycle fields.
- **Risk/rollback:** REGISTER is a broad slice; preserve dialog request/response
  construction and migrate only ownership of returned state.

**Implemented finding (2026-07-21):** `RegisterAttemptContext` and the typed
REGISTER result/post-commit effect boundary return dialog-owned transaction
facts to `execute_register_action`, which applies lifecycle fields to the
lane-owned state. `SendREGISTER`, `SendREGISTERWithAuth`, `SendUnREGISTER`, and
`SendREGISTERWithOptions` all delegate to that one implementation.
`RegistrationStateProjection` and `sync_registration_state` are absent.

### EX-204 — Delete snapshot merge and reconciliation compensation

- **Current:** **SessionStore::replace_session_exact_inner** accepts
  **merge_tracked_request_staging** and **preserve_auth_coordination**; executor
  rereads call state, SDP origin, and media security before final save.
- **Gap:** these mechanisms hide competing writers and can merge fields from
  logically different transitions.
- **Cleanup:** after IN-102, EX-202, and EX-203 eliminate those writers, remove
  both merge flags and the executor reconciliation reread. Keep exact-handle
  validation and immutable revision publication.
- **Dependencies:** IN-102, EX-201, EX-202, and EX-203.
- **Deletion precondition:** instrument the compensation branches first and run
  concurrency/perf suites until their hit counts are zero; then delete them.
- **API impact:** none.
- **Tests:** staging during unrelated event, auth challenge during response,
  simultaneous SDP/media-security completion, and final-state clearing.
- **Complete when:** store replacement has no field-specific merge policy and
  executor correctness does not depend on rereading its own state.
- **Risk/rollback:** a non-instrumented writer may surface; rollback only this
  deletion while keeping migrated writers, then add the missing owner task.

**Implemented finding (2026-07-21):** the tracked-staging and auth-preservation
parameters/branches and the SDP/media reconciliation rereads are deleted.
`replace_session_exact_inner` retains exact-handle/revision/index publication
without field-specific merge policy. The public signatures of
`SessionStore::update_session`, `update_session_with`, and
`update_session_snapshot_with` are unchanged, but each captures the exact
lifetime, queues behind its state-machine lane, and revalidates after the wait.
The executor's final call-state/`entered_state_at` reread is deleted; a public
write now executes before or after one complete YAML event, never inside it.

### EX-205 — Move long waits off the transition lane

- **Current:** **Action::ScheduleReinviteRetry** sleeps 0–4 seconds while the
  lane is held; REFER default handling sleeps 500 ms in an unretained task.
- **Gap:** unrelated exact-session events queue behind policy delays and
  shutdown cannot uniformly account for work.
- **Cleanup:** actions return a scheduled-work request containing the exact
  handle, request kind, retry attempt, randomized deadline, and expected state.
  The existing lifecycle scheduler wakes and dispatches a typed retry event
  only after revalidation. Convert REFER default handling to the same retained
  task family with an exact transaction claim.
- **Dependencies:** IN-105 and EX-202.
- **Deletion precondition:** RFC 3261 glare ranges and the public 500 ms REFER
  grace behavior are covered by paused-time tests.
- **API impact:** none.
- **Tests:** glare owner/non-owner ranges, retry cap, explicit REFER response
  cancelling default, shutdown, session reuse, and BYE during backoff.
- **Complete when:** no state-machine action sleeps and retained-task counts
  return to zero after drain.
- **Risk/rollback:** scheduling changes ordering; retain existing ranges,
  attempt caps, and grace duration exactly.

**Implemented finding (2026-07-21):** glare retry and REFER grace decisions
are retained exact scheduled work with generation and claim revalidation; no
state-machine action sleeps while holding the transition lane.

### EX-206 — Decouple terminal release from publication

- **Current:** several terminal paths compose application publication with
  exact release and track combined completion states.
- **Gap:** reporting pressure can delay or complicate authoritative cleanup.
- **Cleanup:** claim terminal ownership, commit terminal state, and release
  exact dialog/media/session resources independently of observation. Enqueue
  the compatibility event from the committed terminal record; publication
  failure is diagnostic and never changes release.
- **Dependencies:** EX-201, RT-303, and AP-601.
- **Deletion precondition:** bus closed/full/stalled tests demonstrate identical
  release metrics.
- **API impact:** event variants and successful delivery behavior are retained;
  only failure isolation changes.
- **Tests:** publication failure at each terminal stage, duplicate terminal
  sources, timeout versus BYE, and observer shutdown.
- **Complete when:** terminal release completion has no “publication and
  release” combined state and zero resources remain after drain.
- **Risk/rollback:** observers may miss an event during failure, which is
  already allowed for best-effort reporting; do not re-couple cleanup to avoid
  that loss.

**Implemented finding (2026-07-21):** exact terminal claim, committed terminal
state, and resource release are independent of nonblocking observational
publication. Source-level absent/full/closed observer fixtures fence that
separation; the final full observer/drain execution remains pending.

## 9. Phase 3 — Remove Duplicate Protocol Routing

### RT-301 — Use acknowledged typed dispatch for causal ingress

- **Current:** dialog-core converts typed events and usually calls general
  **publish**, which invokes a handler and also publishes a bus copy.
- **Gap:** causal handling and observation share one operation and ambiguous
  failure semantics.
- **Cleanup:** route every session-affecting **DialogToSessionEvent** through
  **dispatch_authoritative_handler** or the existing acknowledged direct
  equivalent. Keep **DialogToSessionDirectRouter** as the bounded, sharded
  executor. Do not add another queue or actor.
- **Dependencies:** IN-104.
- **Deletion precondition:** event-by-event routing table in section 15 is
  implemented and fault-injection tests pass.
- **API impact:** none.
- **Tests:** missing handler, duplicate handler, full shard, closed shard,
  shutdown drain, and handler failure for every response-bearing class.
- **Complete when:** no session-affecting dialog event relies on broadcast
  subscriber delivery.
- **Risk/rollback:** non-authoritative events can be overclassified; migrate by
  event class and preserve separate observational projection where required.

**Implemented finding (2026-07-21):** session-affecting
`DialogToSessionEvent` values enter the bounded sharded router through the
registered acknowledged causal ingress. Capability-bearing events never depend
on a broadcast subscriber, and any public copy is sanitized after authoritative
handling.

### RT-302 — Install causal sinks before opening transports

- **Current:** event subscriptions are started as part of coordinator
  initialization, but ordering is not expressed as an invariant at the
  transport receive boundary.
- **Gap:** a fast inbound packet can be processed before its sole session
  handler is ready.
- **Cleanup:** split initialization into create components, register direct
  handlers, verify exactly one handler per causal event type, then bind/start
  receive transports. Failure before the final step tears down components
  without opening the socket.
- **Dependencies:** RT-301.
- **Deletion precondition:** remove any startup retry/fallback only after an
  immediate-packet test passes.
- **API impact:** constructors and public results remain unchanged.
- **Tests:** inject a packet at transport-open, handler registration failure,
  partial startup shutdown, and repeated start rejection.
- **Complete when:** runtime assertion/metrics prove transport-open cannot
  precede causal-handler-ready.
- **Risk/rollback:** startup ordering across features; keep the public
  constructor orchestration and change only its private phases.

**Implemented finding (2026-07-21):** coordinator construction registers the
causal dialog ingress before it hands that ingress to the started dialog
adapter/transport path. Construction guards and immediate-event source tests
prevent a transport-ready path from preceding its causal sink.

### RT-303 — Make the event bus observational

- **Current:** **publish**, **publish_authoritative**, and
  **publish_observational** coexist, and some code publishes application events
  before canonical commit.
- **Gap:** callers cannot infer whether bus delivery is causal, authoritative,
  or best effort.
- **Cleanup:** reserve **dispatch_authoritative_handler** for private causal
  messages; use **publish_observational** only for sanitized committed
  outcomes. Prohibit the general **publish** method in signaling causal paths
  through FZ-002. Keep the coordinator API because other crates may use it.
- **Dependencies:** RT-301 and AP-601.
- **Deletion precondition:** each moved event has a commit-point test and
  sensitive capability fields are absent from its observational copy.
- **API impact:** none.
- **Tests:** healthy, absent, full, stalled, closed, and failing observer;
  compare SIP trace, transition history, public return value, and cleanup
  counters.
- **Complete when:** signaling-equivalence tests are byte-for-byte identical
  across observer states except timestamps and observation counters.
- **Risk/rollback:** diagnostics visibility can decrease; add post-commit
  metrics before removing a bus copy.

**Media-bus completion finding (2026-07-20):**
`SessionCrossCrateEventHandler::handle_media_to_session_event` now treats every
`MediaToSessionEvent` as reporting only. Quality updates/degradation and DTMF
retain their existing public observation shapes; stream start/stop, flow,
recording, playback, media error, RTP timeout, and packet-loss threshold
reports are diagnostic observations and cannot dispatch a state-machine
event. Synchronous lane-owned media actions establish resources and committed
media state, while the retained exact watchdog remains the lifecycle authority
for media failure policy. The `EventType` variants and runtime YAML grammar are
deliberately retained for authoritative typed callers and configuration
compatibility. Static and projection tests fence this separation. No public
API changed.

### RT-304 — Delete debug-string routing

- **Historical baseline:** **session_event_handler.rs** contained 28 unreferenced
  **handle_*(event_str: &str)** wrappers and
  **extract_session_id**, **extract_field**, **extract_debug_string_field**,
  and **extract_optional_field**.
- **Gap:** debug formatting is an apparent alternate protocol even though typed
  handlers are live.
- **Cleanup:** prove zero call sites, map each wrapper to its typed ***_parts**
  or typed match arm, delete the wrappers and extraction helpers, then add a
  guard that rejects **event_str** routing in production.
- **Dependencies:** RT-301.
- **Deletion precondition:** typed-path parity tests exist for every wrapper
  category before deletion.
- **API impact:** none; methods are private.
- **Tests:** incoming call, provisional/final response, auth, timer, transfer,
  ACK, BYE, dialog error, media lifecycle/quality, DTMF, NOTIFY, and RTP timeout.
- **Complete when:** repository search finds no production event debug parsing.
- **Risk/rollback:** a feature-only caller may be missed; run all features and
  platform-independent checks before deletion.

**Implemented finding (2026-07-21):** all 28 wrappers and all four extraction
helpers are deleted. Typed match arms/`*_parts` handlers are the only retained
routes, and the architecture fence rejects production `event_str` parsing.

### RT-305 — Replace session-to-dialog bus commands

- **Current:** REFER rejection and some REGISTER/response/mapping operations
  construct **SessionToDialogEvent** and use the global coordinator.
- **Gap:** local request/response capabilities are sent through a reporting
  abstraction and can lose exact transaction ownership.
- **Cleanup:** call existing typed dialog APIs with exact transaction/dialog
  identities. Return typed results directly to the state-machine action.
  Retain public application events as post-commit observations.
- **Dependencies:** RT-301, EX-202, and PR-401.
- **Deletion precondition:** wire parity and exact transaction tests pass for
  every converted command.
- **API impact:** none.
- **Tests:** REFER 202/603, REGISTER responses, duplicate/stale transaction,
  full/closed event bus, and immediate peer response.
- **Complete when:** no response-bearing session-to-dialog command requires the
  event bus.
- **Risk/rollback:** dialog APIs may lack one narrow operation; add that
  operation to dialog-core rather than creating a second rvoip-sip protocol
  implementation.

**Implemented finding (2026-07-21):** REGISTER and REFER responses, exact
response capabilities, and mapping operations use typed dialog/transaction or
registry APIs. No production `SessionToDialogEvent` response command remains;
event-coordinator traffic at this boundary is observational only.

### RT-306 — Preserve and fence the outbound in-dialog tracker

- **Current:** **OutboundInDialogRequestTracker** correlates INFO, REFER,
  NOTIFY, and UPDATE; deferred auth/completion events handle response-before-
  tracker-install races.
- **Gap:** cleanup could accidentally replace this proven authority or leave
  staging/auth fields as a second tracker.
- **Cleanup:** keep the tracker, attach exact session handles to entries and
  deferred deliveries, and make the state-machine staging slot ephemeral only
  until tracker installation. Completion clears the exact entry and associated
  auth data once.
- **Dependencies:** IN-102 and IN-104.
- **Deletion precondition:** no pending slot is removed until response-before-
  install and exact-pointer cancellation tests pass.
- **API impact:** none.
- **Tests:** response before install, auth before install, mismatched
  transaction, stale generation, cancellation, duplicate completion, and
  simultaneous different methods.
- **Complete when:** each in-dialog method has one in-flight authority and no
  full session snapshot retains request wire/options after installation.
- **Risk/rollback:** tracker retention; add entry/deferred counts to drain
  assertions before migrating fields.

**Implemented finding (2026-07-20):** `TrackedRequestKey` and every deferred
auth/completion delivery now carry the exact `SessionRegistryHandle`, so claim,
replay, retry, completion, abort, and cleanup fail closed across raw-ID reuse.
The retained tracker exposes live-entry and deferred-delivery counts through
internal performance diagnostics, and an actual generation-A/reused-B test
proves an old replay cannot complete B's request. The tracker remains the
single in-flight authority; this cleanup introduced no replacement engine and
no public API change.

## 10. Phase 4 — Consolidate Protocol Implementations

### PR-401 — Reduce DialogAdapter methods to one options path per method

- **Current:** **dialog_adapter.rs** contains multiple convenience, session,
  auth, and options variants for several SIP methods.
- **Gap:** request construction and header behavior can drift between entry
  points.
- **Cleanup:** choose the existing options-bearing dialog call as the one
  implementation for each method. Private convenience methods only construct
  options and delegate; remove methods that duplicate construction after all
  callers migrate. Retained compatibility methods must resolve canonical exact
  ownership and return an error when no wire operation can be performed.
  Remaining deletion candidates require the stated caller/API/wire proofs;
  their presence in this ledger is not deletion authorization.
- **Dependencies:** FZ-001.
- **Deletion precondition:** repository/all-feature zero-caller proof and wire
  fixture parity per removed method.
- **API impact:** no public **src/api** method changes; internal removals must
  not alter frozen signatures.
- **Tests:** canonical request snapshot for each method, including custom
  headers, route, body, content type, auth, and transport.
- **Complete when:** each intentionally distinct context—initial, in-dialog,
  standalone, or subscription refresh—has one implementation per method that
  reaches dialog-core's send API, and all same-context helpers delegate to it.
- **Risk/rollback:** method-specific semantics differ; migrate one SIP method
  at a time and retain its public builder tests.

**False-success cleanup finding (2026-07-20):** without changing a public
`src/api` signature, **send_response_by_dialog** now resolves the canonical
generation-qualified session and delegates to the session response path;
legacy **send_bye**, **send_reinvite**, and **send_refer** fail closed when the
dialog has no exact owner. **send_ack** now errors when its exact INVITE
transaction is absent, and **get_remote_uri** returns dialog-core's actual
remote URI instead of a placeholder. Static fences preserve exact resolver
ordering, delegation, and these fail-closed boundaries.

**Canonical-method finding (2026-07-21):** initial INVITE entry points now
delegate to `send_initial_invite_staged`; the four REGISTER action variants
delegate to `execute_register_action`; and standalone MESSAGE enters the shared
typed standalone request driver. Intentionally different initial, in-dialog,
standalone, and subscription contexts remain separate, as required by this
plan, but same-context helpers no longer construct competing wire requests.

Inbound REGISTER now has one response materializer. Its causal request is
delivered only to the acknowledged registrar handler; observer absence can
never authorize a binding. An absent handler receives 503, a handler error is
propagated without a competing fallback response, and the retained legacy
auto-REGISTER flag returns 501 because dialog-core owns no location-service
store. Successful registrar responses echo or override Contact correctly, and
401/407 select WWW-Authenticate/Proxy-Authenticate respectively.

### PR-402 — Reuse one immutable request snapshot for authentication

- **Current:** auth paths mix pending session fields, adapter reconstruction,
  and tracker-retained options.
- **Gap:** retry can lose custom headers/body/routing or race nonce-count state.
- **Cleanup:** retain the method's immutable options in its existing request
  owner, derive Authorization or Proxy-Authorization from the typed challenge
  and exact transport context, and produce one retry options value differing
  only in stack-owned auth/CSeq/branch fields required by SIP. Keep digest
  nonce counts exact-lifetime-owned and serialized.
- **Dependencies:** PR-401 and RT-306.
- **Deletion precondition:** authenticated wire snapshots prove all
  application-authored fields survive 401 and 407.
- **API impact:** none; supported Digest/Basic/Bearer/AKA configuration and
  public errors remain.
- **Tests:** 401, 407, stale nonce, qop auth/auth-int, MD5 and stronger
  supported algorithms, body integrity, proxy routing, repeated refresh, and
  missing credentials.
- **Complete when:** no auth retry reconstructs a request from scattered
  **SessionState** fields.
- **Risk/rollback:** standards regression; keep legacy standards-conforming
  algorithms and peer interoperability even if their implementation is shared.

**Implemented finding (2026-07-21):** initial INVITE keeps one staged options
snapshot through challenge retry, the in-dialog tracker owns its exact retained
options/auth correlation, and the standalone driver adds credentials to its
typed retained request. The deleted session merge/auth-preservation path is no
longer used as an alternate request reconstruction authority.

### PR-403 — Consolidate standalone MESSAGE, OPTIONS, and SUBSCRIBE

- **Current:** **UnifiedCoordinator::send_message_oob_with_optional_auth**,
  **send_options_oob_with_optional_auth**, and
  **send_subscribe_oob_with_optional_auth** are separate direct flows.
- **Gap:** shared authentication, retry, response, and redaction behavior can
  diverge.
- **Cleanup:** keep these operations dialog/transaction-owned and create one
  private generic standalone request/auth driver parameterized by typed method
  options and response parser. Public builders continue to call their same
  methods. Do not create artificial **SessionState** entries or YAML rows.
- **Dependencies:** PR-401 and PR-402.
- **Deletion precondition:** per-method wire and error parity, including
  subscription refresh/termination.
- **API impact:** none.
- **Tests:** unauthenticated/authenticated MESSAGE and OPTIONS, SUBSCRIBE
  200/202/401/407/423/481, NOTIFY correlation, timeout, cancellation, and
  sensitive-log redaction.
- **Complete when:** one private driver owns standalone auth retry and each
  method supplies only typed construction/interpretation.
- **Risk/rollback:** over-generalization can erase RFC differences; the driver
  owns only shared transaction/auth mechanics, not method-specific policy.

**Implemented finding (2026-07-21):** MESSAGE, OPTIONS, and SUBSCRIBE are typed
variants of `StandaloneRequestOptions` and all use the same private standalone
send/auth/retry driver. Method-specific option construction and SUBSCRIBE
interpretation remain explicit. No temporary session or YAML transition was
introduced.

### PR-404 — Select canonical mapping owners

- **Current:** **SessionRegistry** owns generation-qualified cross-layer dialog
  and media associations. Dialog-core retains only its protocol-internal
  routing maps, while **DialogAdapter** and **MediaAdapter** retain exact
  managed-resource and operation tables rather than compatibility routing-map
  mirrors.
- **Gap:** guard the completed migration against raw-identifier projections,
  partial publication, and stale-generation cleanup regressions.
- **Cleanup:** make **SessionRegistry** the canonical exact cross-layer mapping.
  Dialog-core retains its internal maps for dialog routing. MediaAdapter retains
  actual resource tables but resolves lifetime/dialog association through the
  registry. Migrate reads, then writes, then remove adapter compatibility maps
  one map at a time. Observational Call-ID correlation must resolve through
  registry-owned exact slot metadata, fail closed on ambiguity, and must not
  introduce another reverse index.
- **Dependencies:** IN-104 and FZ-002.
- **Deletion precondition:** every removed map reports zero reads/writes under
  all-feature static scan and zero entries in shadow-comparison soak.
- **API impact:** none.
- **Tests:** inbound/outbound setup, fork/redirect, early response, exact
  cleanup, failed setup compensation, ID reuse, media-only/signaling-only, and
  concurrent lookup/removal.
- **Complete when:** diagnostics distinguish canonical authority counts from
  resource counts and no compatibility routing map is needed for correctness.
- **Risk/rollback:** hot-path lookup regression; benchmark each map removal and
  allow a read-through cache only if it is generation-keyed, derived, and
  never independently written.

**Cleanup status (2026-07-20):** the adapter-owned forward/reverse dialog and
media compatibility maps are deleted. Media lookup now requires agreement
between the current exact **SessionRegistry** association and the exact
**MediaSessionResource** binding; retained cleanup cross-checks registry,
session-state, and resource identity and fails closed if owners disagree.
Compatibility performance keys remain present with value zero, while canonical
registry bindings and managed media resources are reported separately. Static
fences require zero adapter routing-map writes and prevent either adapter map
from being restored.

No adapter routing read-through cache remains. This deletion does not include
dialog/transaction protocol caches such as INVITE 2xx retransmission storage;
those are standards-owned wire mechanics, not duplicate cross-layer mappings.

### PR-405 — Route lifecycle timers through retained exact tasks

- **Current:** registration refresh is adapter-owned, dialog-core owns session
  refresh mechanics, and some session retry policy lives in actions/tasks.
- **Gap:** cancellation and final release accounting are inconsistent.
- **Cleanup:** keep protocol timers in dialog/transaction code and lifecycle
  policy timers in the state-machine scheduler. Every bridge between them
  carries exact identity and emits a typed result. Registration refresh,
  re-INVITE glare, session refresh failure, and transfer grace are registered
  with the existing retained-task accounting.
- **Dependencies:** IN-105, EX-205, and EX-203.
- **Deletion precondition:** timer-family retained count reaches zero after
  normal completion, failure, and shutdown.
- **API impact:** timer configuration fields and behavior remain unchanged.
- **Tests:** paused-time boundary values, transport recovery, shutdown, final
  release, and generation reuse.
- **Complete when:** each timer has one policy owner, one wire owner, and one
  cancellation owner.
- **Risk/rollback:** timer behavior is interoperability-sensitive; never move
  transaction retransmission timers into the state machine.

PR-405's “each timer” completion criterion is subject to the IN-105
active-media watchdog stop boundary above. That watchdog already has one
retained scheduler owner and exact firing identity; this cleanup must not
replace it with an owned operation that imposes a maximum call duration or
deadlocks exact release.

**Implemented finding (2026-07-21):** registration refresh, glare retry,
session refresh/failure, transfer grace, setup, and teardown work use the
existing exact retained task/deadline owners and revalidate their captured
lifetime. The active-media shared-scheduler boundary above remains intentional;
transaction retransmission timers remain dialog/transaction-owned.

## 11. Phase 5 — Clean YAML and Dead Machinery

### YA-501 — Add bidirectional state-table reachability

- **Current:** loader validation checks syntax and several structural rules,
  while Rust enums, wiring manifest, and YAML can still drift.
- **Gap:** an action/event may compile but have no reachable row, or YAML may
  name machinery with no supported ingress.
- **Cleanup:** for the embedded **default.yaml**, add tests that compute:
  YAML-to-Rust resolution for every state, event, guard, action, condition, and
  event template; Rust-to-YAML reachability for lifecycle variants; and
  manifest-to-YAML/direct-route agreement for every SIP method. Configured
  custom YAML keeps its existing parser and structural-validation contract; it
  is not required to reproduce the built-in wiring manifest.
- **Dependencies:** FZ-002.
- **Deletion precondition:** none.
- **API impact:** none; runtime YAML grammar is unchanged.
- **Tests:** fixtures for unknown, duplicate, unreachable, direct-owned, and
  intentionally deferred entries.
- **Complete when:** each embedded-default exception is explicitly classified
  as direct, dialog-managed, transport-only, internal, removed, or deferred,
  and valid configured-table fixtures retain their current behavior.
- **Risk/rollback:** configurable external YAML can use valid variants absent
  from the default table; keep a declared extension allowlist and do not narrow
  parsing.

**Completion finding (2026-07-20):** test-only exact inventories now compare
the embedded table with all 106 `EventType`, 14 `Guard`, 98 `Action`, and 13
`EventTemplate` variants. Default-unused variants require an exact owner and
reason; stale, duplicate, newly unclassified, or now-reachable allowances
fail. State declarations/references and condition declarations/writers are
also bidirectional. Unknown lifecycle events/actions remain hard failures,
while configured custom guard and publish-template grammar is unchanged.

### YA-502 — Move only duplicated lifecycle decisions into existing YAML

- **Current:** glare checks, retry caps, automatic transitions, and terminal
  behavior are partly hard-coded around table execution.
- **Gap:** some application-visible decisions bypass the intended single
  transition source.
- **Cleanup:** inventory every branch that selects next state, suppresses a
  legal transition, chooses lifecycle retry/failure, or orders lifecycle
  cleanup. Express those decisions using existing **EventType**, **Guard**,
  **Action**, and transition fields where representable. Leave wire legality
  checks in dialog/transaction code.
- **Dependencies:** YA-501 and EX-202.
- **Deletion precondition:** behavior parity test names the old branch and new
  YAML row.
- **API impact:** none.
- **Tests:** golden compiled-table test and behavior tests for every moved
  branch.
- **Complete when:** the branch inventory is empty or each retained branch is
  documented as protocol mechanics, input normalization, resource ownership,
  or invariant enforcement.
- **Risk/rollback:** do not turn YAML into a programming language; if existing
  constructs cannot express a decision without grammar expansion, retain the
  narrow Rust action and document its ownership.

**Completion finding (2026-07-20):** the production branch audit found zero
remaining duplicated lifecycle decisions that can be moved into the embedded
table using the current one-transition-per-`{role, state, event}` model and
the existing accepted YAML names. One redundant branch was deleted:
`state_machine::actions::execute_action` no longer rechecks
`Config::auto_180_ringing` after
`state_machine::executor::StateMachine::should_skip_action` has already
suppressed the automatic 180 action. A static architecture fence requires that
configuration policy to remain single-owned. No default-table row, public API,
or configured-table grammar changed.

The retained branch inventory is exact at the symbol/family level below.
“Protocol mechanics” includes status/challenge normalization and RFC-bounded
wire retries selected by an action that YAML already ordered; it does not own
the application call state.

| Current symbol/location | Retained branch | Classification and stop boundary |
|---|---|---|
| `state_machine/executor.rs::StateMachine::process_one_event` (`Active + ReinviteReceived + pending_reinvite`) | Send 491 and suppress the ordinary active-call re-INVITE transition. | **Protocol mechanics / invariant enforcement.** The table stores one transition per normalized `{role, state, event}` key. A guarded 491 row and the ordinary unguarded 200 row collide; representing both requires a multi-candidate table model or a new event/action name, both outside this cleanup. |
| `state_machine/executor.rs::StateMachine::process_one_event` (final-state pending-request clear) | Clear all staged request/auth coordination on every final-state entry even if a configured row omits a per-method clear action. | **Invariant enforcement.** Runtime YAML remains configurable and its accepted grammar cannot prove an exhaustive per-method clear list for every final row. |
| `state_machine/executor.rs::{is_exact_retirement_safe_dispatch_only_transition, process_one_event}` | Treat an exact lifetime retired by synchronous BYE/CANCEL/REFER completion as a completed dispatch and never resurrect its local snapshot. | **Resource ownership / invariant enforcement.** YAML ordered the wire action; exact registry retirement determines whether the resource still exists. |
| `state_machine/executor.rs::{run_deferred_reinvite_retry, schedule_deferred_action_effects}` | Cancel or suppress a delayed retry when its exact generation, pending kind, attempt, or lifecycle owner is stale. | **Resource ownership / invariant enforcement.** YAML selected `ScheduleReinviteRetry`; Rust only fences delayed work to that committed exact lifetime. |
| `state_machine/actions.rs::execute_register_action` and the `SendREGISTERWithOptions` arm | Normalize REGISTER outcomes to typed follow-up events; bound auth/stale and 423 retries. | **Protocol mechanics / input normalization.** Existing guards cannot inspect response metadata, nonce replacement, `Min-Expires`, or numeric attempt counters. The resulting `Registration*` event re-enters YAML for lifecycle state selection. Consolidating the two wire facades belongs to PR-401/PR-402, not a second lifecycle authority. |
| `state_machine/actions.rs::{ScheduleReinviteRetry, RetryWithContact}` | Apply RFC backoff/owner ranges and bounded redirect/glare loop breakers. | **Protocol mechanics.** YAML chooses and orders the action; existing transition fields cannot calculate randomized role-dependent delay, pop a Contact, or compare a retry counter. |
| `state_machine/actions.rs::{SendINVITEWithAuth, SendRequestWithAuth, SendINVITEWithBumpedSessionExpires}` | Validate challenge protection space and cap auth/422 wire replay. | **Protocol mechanics / input normalization.** Success or failure remains a typed result/follow-up consumed by the executor; current guards cannot express challenge or transaction identity. |
| `state_machine/actions.rs::SendReINVITE` | Select hold versus resume SDP for the existing custom-table action from the committed `HoldPending`/`Resuming` state. | **Protocol mechanics.** Replacing this with separate YAML action names would expand the accepted runtime grammar; narrowing the action to only default-table event names would break supported configured tables. |
| `api/unified.rs::{SetupTeardownWatchdogKind, fire_setup_teardown_deadline, schedule_active_call_media_timeout_if_current}` | Revalidate exact timer identity and dispatch `DialogTimeout` or `HangupCall`; publish/release only after the YAML transition commits. | **Resource ownership / invariant enforcement.** The scheduler owns time and exact cancellation, while YAML still selects the application next state and cleanup actions. The active-media policy retains its separately documented stop boundary. |
| `api/unified.rs::{PendingExactResponseRegistry, author_pending_exact_response, send_standalone_oob_with_optional_auth}` | Retry exact final-response writes and standalone MESSAGE/OPTIONS/SUBSCRIBE authentication. | **Protocol mechanics / resource ownership.** These are transaction obligations or deliberately sessionless standalone methods, not application session lifecycle transitions. |
| `api/unified.rs::hangup_serialized` and local-BYE finalizers | Join the exact BYE response for an established call or the INVITE/CANCEL outcome for setup teardown, then reclaim the exact lifetime. | **Protocol mechanics / resource ownership.** `HangupCall` first enters YAML; the retained branches only join the wire owner selected by the committed pre-dispatch state. |
| `adapter.rs::{failed_inbound_termination, terminate_failed_inbound, run_inbound_drain}` | Choose reject, hangup, or cleanup-only compensation when an orchestrator route cannot be published or is draining. | **Input normalization / resource ownership.** This is failure compensation across the public adapter boundary; each signaling choice dispatches an existing YAML event. A table move would require a new adapter-failure event and public-policy inputs. |
| `api/incoming.rs::IncomingCallGuard` timeout/drop paths | Resolve an application-owned response deadline once; the retained timeout dispatches exact `RejectCall`, while immediate drop uses the existing public rejection facade. | **Input normalization / invariant enforcement.** YAML owns the resulting transition; the guard owns only its public API deadline and single-resolution token. |
| `adapters/session_event_handler.rs::{handle_call_established_parts, handle_call_failed_parts, handle_call_terminated_parts, handle_bye_received_parts, handle_call_redirected_typed}` | Normalize typed dialog outcomes, suppress false terminal observations for mid-call re-INVITE failure, and guarantee exact release after a terminal wire event. | **Input normalization / resource ownership / invariant enforcement.** These branches do not author a next state; they dispatch a typed YAML event and classify the committed result. Mapping and terminal-publication cleanup remains under PR-404/EX-206. |
| `adapters/dialog_adapter.rs::{mutate_retained_dialog_auth, cleanup_session_exact_lane_owned}` and retained request/BYE/MESSAGE loops | Reject stale/non-active exact dialog work, serialize wire retry with cleanup, and remove dialog/transaction resources in ownership order. | **Protocol mechanics / resource ownership.** Moving transaction legality or exact lock ordering into YAML would violate the retained ownership model. |
| `adapters/registration_adapter.rs::handle_incoming_register` | Authenticate a standalone inbound REGISTER and author its exact transaction response. | **Protocol mechanics / input normalization.** Registrar binding storage is not a call/session state-machine lifecycle. |
| `media_stream.rs` terminal-state bind check and `session_lifecycle.rs` teardown/retry state | Stop late media binding and drain/quarantine exact owned resources. | **Resource ownership / invariant enforcement.** These states describe resource supervisors, not SIP application `CallState`, and therefore must not be represented as YAML call transitions. |

The only table-model stop condition exposed by this audit is the active
re-INVITE glare collision above. It must be escalated before changing the
table from one transition per normalized key, adding a new YAML event/action,
or narrowing the externally configurable grammar. It is not authorization for
any of those changes.

### YA-503 — Delete unreachable states, events, actions, and effects

- **Current:** subscription, publishing, messaging, compatibility, and removed
  feature variants coexist with live call/registration machinery.
- **Gap:** names can imply a second lifecycle implementation even when no row
  or caller exists.
- **Cleanup:** use YA-501 reachability, public API snapshot, all-feature caller
  scan, and runtime coverage to create a deletion ledger. Delete only private
  zero-reachability machinery that is not expressible through the accepted
  configurable YAML grammar. Names currently accepted by **YamlTableLoader**
  are retained, and may be documented as deprecated, unless a separately
  approved and versioned grammar change removes them. Public compatibility
  variants remain as output projections or documented supported input even if
  the default YAML does not use them.
- **Dependencies:** FZ-001 and YA-501.
- **Deletion precondition:** four proofs: no public contract, not expressible
  through the accepted configurable YAML grammar, no caller, and no compliance
  claim.
- **API impact:** none.
- **Tests:** embedded-default reachability rejects stale references; configured
  YAML fixtures for every retained accepted name and extension continue to
  compile with unchanged semantics.
- **Complete when:** every enum variant and YAML name has an owner and a
  reachable purpose.
- **Risk/rollback:** external YAML compatibility; prefer deprecation metadata
  over parser removal when usage cannot be disproven.

**Completion finding (reconciled 2026-07-21):** the exact allowance audit is
closed. All 24 formerly provisional `EventType`, `Action`, and `EventTemplate` allowances
have durable owners. None satisfies all four deletion proofs: every variant is
part of a public `Serialize`/`Deserialize` enum reachable through the public
`state_table` module, twelve actions retain executor implementations, one event
name and the three legacy REGISTER action names are accepted YAML compatibility
shapes, and programmatic transitions retain the remaining shapes. The three
REGISTER actions now delegate to the same canonical typed REGISTER
implementation; their names do not represent duplicate writers. No public
variant or configured-YAML grammar was removed or expanded. A focused test
round-trips every audited serde shape, freezes the
`Registration401 -> AuthRequired` alias, verifies the accepted REGISTER names,
proves the other audited names remain programmatic-only, proves the three
template names retain their existing `Custom` publication semantics, and
rejects any future `inventory-boundary:` placeholder.

`Pass` below means that individual deletion proof is present; deletion remains
forbidden unless all four columns pass.

| Allowance | No public contract | Not expressible in configured YAML | No caller | No compliance claim | Durable owner / disposition |
|---|---:|---:|---:|---:|---|
| `EventType::CallEstablished` | **Fail:** public serde enum | Pass: no lifecycle parser name | **Fail:** history redaction accepts it | Pass | Retain for programmatic-table/history compatibility; typed dialog establishment routes as `Dialog200OK`. |
| `EventType::DialogInvite` | **Fail:** public serde enum | Pass: no lifecycle parser name | Pass: zero internal constructors | Pass: `IncomingCall` owns live ingress | Retain as a programmatic-table compatibility event; inbound INVITE routes canonically as `IncomingCall`. |
| `EventType::DialogREFER` | **Fail:** public serde enum | Pass: no lifecycle parser name | Pass: zero internal constructors | Pass: `TransferRequested` owns the live claim | Retain as a programmatic-table compatibility event; typed REFER ingress routes as `TransferRequested`. |
| `EventType::DialogReINVITE` | **Fail:** public serde enum | Pass: no lifecycle parser name | Pass: zero internal constructors | Pass: `ReinviteReceived` owns the live claim | Retain as a programmatic-table compatibility event; typed re-INVITE ingress routes as `ReinviteReceived`. |
| `EventType::InternalACKSent` | **Fail:** public serde enum | Pass: no lifecycle parser name | Pass: zero internal constructors | Pass: `DialogACK` owns live ACK compliance | Retain as a programmatic-table compatibility event; live ACK ingress routes as `DialogACK`. |
| `EventType::InternalUASMedia` | **Fail:** public serde enum | Pass: no lifecycle parser name | Pass: zero internal constructors | Pass | Retain as a programmatic-table compatibility event; live UAS media readiness uses normalized media ingress. |
| `EventType::InternalCleanupComplete` | **Fail:** public serde enum | Pass: no lifecycle parser name | Pass: zero internal constructors | Pass | Retain as a programmatic-table compatibility event; exact lifecycle release owns live completion. |
| `EventType::Registration401` | **Fail:** public serde enum | **Fail:** accepted legacy alias | **Fail:** normalization/alias behavior | **Fail:** REGISTER challenge compatibility | Retain; configured YAML maps the legacy name to `AuthRequired { status_code: 401, method: "REGISTER" }`. |
| `EventType::CleanupComplete` | **Fail:** public serde enum | Pass: no lifecycle parser name | Pass: zero internal constructors | Pass | Retain as a programmatic-table compatibility event; exact lifecycle release owns live completion. |
| `Action::SendREGISTER` | **Fail:** public serde enum | **Fail:** accepted configured-YAML name | **Fail:** executor arm | **Fail:** REGISTER compatibility | Retain as the legacy initial/refresh facade; delegates to `execute_register_action`. |
| `Action::SendREGISTERWithAuth` | **Fail:** public serde enum | **Fail:** accepted configured-YAML name | **Fail:** executor arm | **Fail:** challenged REGISTER compatibility | Retain as the legacy challenged facade; delegates to `execute_register_action`. |
| `Action::SendUnREGISTER` | **Fail:** public serde enum | **Fail:** accepted configured-YAML name (including `SendREGISTERWithExpires0`) | **Fail:** executor arm | **Fail:** unregister compatibility | Retain as the legacy Expires-zero facade; delegates to `execute_register_action`. |
| `Action::HoldCall` | **Fail:** public serde enum | Pass: programmatic-only action name | **Fail:** executor arm | **Fail:** hold re-INVITE behavior | Retain for `StateTableBuilder`; the arm sends one lane-owned hold re-INVITE. |
| `Action::ResumeCall` | **Fail:** public serde enum | Pass: programmatic-only action name | **Fail:** executor arm | **Fail:** resume re-INVITE behavior | Retain for `StateTableBuilder`; the arm sends one lane-owned resume re-INVITE. |
| `Action::TransferCall` | **Fail:** public serde enum | Pass: programmatic-only action name | **Fail:** executor and history arms | **Fail:** REFER behavior | Retain for `StateTableBuilder`; the arm sends the options-based REFER. |
| `Action::StartRecording` | **Fail:** public serde enum | Pass: programmatic-only action name | **Fail:** executor arm | Pass | Retain the public shape, but fail closed until media-core exposes a real recording owner; no path or recording ID is fabricated. |
| `Action::StopRecording` | **Fail:** public serde enum | Pass: programmatic-only action name | **Fail:** executor arm | Pass | Retain the public shape, but fail closed until media-core exposes a real recording owner. |
| `Action::ReleaseAllResources` | **Fail:** public serde enum | Pass: programmatic-only action name | **Fail:** executor arm | Pass | Retain as a programmatic exact dialog-and-media cleanup action. |
| `Action::StartEmergencyCleanup` | **Fail:** public serde enum | Pass: programmatic-only action name | **Fail:** executor arm | Pass | Retain as a programmatic best-effort exact cleanup action. |
| `Action::AttemptMediaRecovery` | **Fail:** public serde enum | Pass: programmatic-only action name | **Fail:** executor compatibility arm | Pass | Retain the public shape and return `InvalidTransition`; a missing recovery implementation can never report success. |
| `Action::CleanupResources` | **Fail:** public serde enum | Pass: programmatic-only action name | **Fail:** executor compatibility arm | Pass | Retain as an alias to the same exact dialog-and-media release implementation used by `ReleaseAllResources`. |
| `EventTemplate::IncomingCall` | **Fail:** public serde enum | Pass: typed variant is programmatic-only | **Fail:** executor fallback handles it | Pass | Retain for programmatic transitions; it preserves the legacy named `Custom` observation. |
| `EventTemplate::MediaNegotiated` | **Fail:** public serde enum | Pass: typed variant is programmatic-only | **Fail:** executor fallback handles it | Pass | Retain for programmatic transitions; it preserves the legacy named `Custom` observation. |
| `EventTemplate::MediaSessionReady` | **Fail:** public serde enum | Pass: typed variant is programmatic-only | **Fail:** executor fallback handles it | Pass | Retain for programmatic transitions; it preserves the legacy named `Custom` observation. |

The source-file audit separately found four items that did satisfy every
proof:
`api/callbacks.rs`, `api/terminal.rs`, `session_store/inspection.rs`, and
`session_store/cleanup.rs` were not declared or included by any module and all
of their unique symbols were zero-caller. Those four orphan files (1,514
logical lines) and their stale commented module references have been deleted;
an architecture test prevents either a file or module declaration from being
silently restored. `state_machine/effects.rs`, the public enum variants, and
the subscriber registry remain explicit compatibility boundaries.

### YA-504 — Make configured YAML selection explicit without changing Config behavior

- **Current:** **Config::state_table_path** uses a two-tier selection: try the
  configured runtime YAML and fall back to the embedded default if loading or
  validation fails.
- **Gap:** reports can omit which table actually became authoritative, and an
  operator can mistake a fallback run for a custom-table run.
- **Cleanup:** retain the field, its type, the external grammar, and the
  existing two-tier behavior. Resolve and validate the selected table before
  transports open, emit a structured redacted diagnostic when fallback is
  used, and record selected source plus hash in reports and attestation. A
  future fail-closed change requires separate compatibility approval.
- **Dependencies:** YA-501 and RT-302.
- **Deletion precondition:** no fallback branch is deleted in this cleanup.
- **API impact:** none; type, signature, valid-table behavior, and current
  fallback semantics remain unchanged.
- **Tests:** missing file, malformed YAML, unknown variant, incomplete
  lifecycle, valid extension, and **None**; assert selected source/hash and
  secret-safe diagnostics for each case.
- **Complete when:** evidence records the exact loaded YAML hash.
- **Risk/rollback:** fallback can hide configuration errors; this slice makes
  it observable and attested without changing the compatibility contract.

**Completion finding (2026-07-20):** the only runtime call graph remains
`UnifiedCoordinator::new` -> `load_state_table_with_config`; the public
function and `Config::state_table_path: Option<String>` are unchanged. The
loader now produces a crate-private selection result containing the exact
selected YAML bytes, SHA-256, and one of embedded default, configured path, or
configured-path fallback with a bounded reason. Runtime diagnostics project
only the source class, bounded reason, and hash: configured paths and detailed
parser/validation errors are not logged. Read, UTF-8 decode, compile, and table
validation failures still select the embedded default exactly as before.

The beta gate now copies and hashes `BETA_STATE_TABLE_SELECTED_YAML` instead of
unconditionally attesting `default.yaml`, records `BETA_STATE_TABLE_SOURCE`,
and cross-checks that selection against the copied input in the standalone
attestation verifier. `BETA_REQUIRE_CONFIGURED_STATE_TABLE_EVIDENCE=1` fails
closed unless the source is `configured-path` and the selected YAML path was
supplied explicitly; embedded/fallback claims must byte-match the embedded
default. Focused fixtures cover configured success, missing/malformed/unknown
and lifecycle-invalid fallback, exact bytes/hash/source, redacted diagnostics,
configured evidence validation, and post-attestation YAML/hash tampering.

## 12. Phase 6 — Compatibility Projections and Downstream Validation

### AP-601 — Emit compatibility events from one committed outcome

- **Current:** detailed and legacy event variants can be published from
  different paths or before terminal cleanup.
- **Gap:** observers can see contradictory order or duplicate terminal facts.
- **Cleanup:** create one private post-commit projection function from
  transition result plus immutable committed snapshot. It emits existing event
  variants in their documented order and applies exact-terminal deduplication.
  The public **get_incoming_call** queue and **IncomingCallInfo** remain a
  compatibility projection from that same committed inbound-call snapshot;
  they never authorize acceptance, response, transition, or cleanup.
- **Dependencies:** EX-201, EX-206, and RT-303.
- **Deletion precondition:** event-order snapshots exist for all public peer
  surfaces.
- **API impact:** no variants, fields, or documented successful events removed.
- **Tests:** Endpoint, StreamPeer, CallbackPeer, SessionHandle, raw coordinator,
  and **get_incoming_call** event sequences for
  success/failure/cancel/transfer/registration; a full/closed incoming-call
  compatibility queue must not alter SIP processing.
- **Complete when:** no application event publisher can mutate signaling state
  or originate a lifecycle transition.
- **Risk/rollback:** consumer timing assumptions; preserve per-session order
  and add explicit tests before rerouting.

**Bounded implementation finding (2026-07-20):** provisional, established,
and failed response compatibility pairs now come from one private projector
that accepts an immutable exact committed snapshot and preserves their existing
legacy/detailed order. A failed or no-transition state-machine attempt cannot
originate those application events. The legacy incoming-call queue receives a
clone of the same exact admission bundle committed to the registry; full or
closed queue admission remains a nonblocking observation failure.

The explicit stop boundaries are unchanged: CallbackPeer may translate an
application decision into a normal coordinator command, and typed inbound
INFO/REFER request capabilities may send an application-selected response.
Those are public control APIs, not publisher-owned state mutation; they still
re-enter exact coordinator/dialog paths. Endpoint mapping, StreamPeer/raw event
delivery, CallbackPeer projection, and SessionEventPublisher contain no direct
state-machine/store mutation. Lower dialog termination and inbound UAS CANCEL
remain causal protocol outcomes eligible for terminal projection even when no
new YAML row is required. No public event variant, field, successful order, or
event API was redesigned in this slice.

### AP-602 — Compile every downstream consumer

- **Current:** the workspace contains direct consumers in **rvoip**,
  **rvoip-client**, **rvoip-core**, **rvoip-amazon-connect**, **rvoip-uctp**,
  **rvoip-webrtc**, **rvoip-audio-device**, extensions, and standalone SIP
  examples.
- **Gap:** crate-local tests do not prove public compatibility.
- **Cleanup:** add a downstream matrix covering default and relevant feature
  sets, plus the independent examples under the workspace **examples/**
  directories. It must explicitly include
  **examples/12-customer-escalation-sip-webrtc** and
  **examples/13-sip-to-amazon-connect**, which are not covered by a matrix that
  stops at examples 01–10.
- **Dependencies:** FZ-001.
- **Deletion precondition:** every internal deletion waits for this matrix.
- **API impact:** none.
- **Tests:** cargo check/test for each consumer and doctests/examples for
  rvoip-sip.
- **Complete when:** matrix is required and produces no feature-unification-only
  false pass.
- **Risk/rollback:** build cost; split CI jobs but require all before merge.

**Completion finding (2026-07-20):** **scripts/beta_gate.sh** now runs every
downstream package/profile in its own Cargo invocation, including the facade,
client, Core examples, Amazon Connect server surface, UCTP plus QUIC,
WebTransport, and WebSocket substrates, headless WebRTC interop features, and
the audio-device bridge. It separately checks all 13 detached example
manifests; examples 12 (SIP/WebRTC escalation) and 13 (SIP/Amazon Connect) are
explicit inventory entries in both the beta gate and the GitHub example build
matrix. **scripts/test_beta_gate_source.py** fences the package isolation,
exact example inventory, mandatory API gate, and release metadata alignment so
a feature-unified or truncated matrix cannot silently replace the required
evidence.

### AP-603 — Preserve public behavior, errors, and cancellation

- **Current:** cleanup changes internal ordering around staging, tasks, and
  observation.
- **Gap:** source compatibility alone does not protect conflict timing,
  cancellation safety, idempotency, or event order.
- **Cleanup:** record black-box contract tests for builders and coordinator
  methods before migration. Compare return values, error variants, wire
  messages, event sequences, and resource drain after each slice.
- **Dependencies:** FZ-001.
- **Deletion precondition:** behavior fixture exists for the path being
  removed.
- **API impact:** none.
- **Tests:** all public call/registration/request/response builders, waiters,
  shutdown, and double-invocation behavior.
- **Complete when:** internal implementation can be swapped in tests without a
  public fixture difference.
- **Risk/rollback:** a current behavior may itself be a bug; any intentional
  public behavior change is removed from this cleanup and reviewed separately.

## 13. Phase 7 — Verification, Reporting, and Release Evidence

The test, diagnostics, reporting, and attestation source needed by this phase
is present. This phase is not complete until those artifacts are produced by
the final clean-tree run. Every runtime result in TE-701 through TE-706 is
therefore **PENDING**; source fixtures are not a substitute for a PASS.

### TE-701 — Deterministic concurrency matrix

- **Current:** race fixes are distributed across unit, integration, and perf
  tests.
- **Gap:** scheduling luck can hide a competing writer.
- **Cleanup:** add barrier-driven tests that pause at lane acquisition, narrow
  identity publication, request tracker installation, wire send, synchronous
  response, canonical commit, scheduled wake, observation, and release.
- **Dependencies:** Phases 1–4.
- **Deletion precondition:** no race compensation is deleted until its
  corresponding barrier-driven interleaving passes.
- **API impact:** none.
- **Tests:** fast response, cancel versus answer, BYE versus timeout, auth retry,
  glare, stale generation, duplicate final response, teardown, and ID reuse.
- **Complete when:** every interleaving has explicit invariants and passes under
  repeated and model-scheduler runs where available.
- **Risk/rollback:** test-only.

### TE-702 — Observer isolation matrix

- **Current:** event coordinator supports authoritative and observational
  modes, but signaling equivalence is not a release gate.
- **Gap:** queue pressure can still become a hidden correctness dependency.
- **Cleanup:** run the same canonical signaling scenarios with healthy, absent,
  saturated, stalled, failing, closed, and shutdown-racing observers.
- **Dependencies:** RT-303 and EX-206.
- **Deletion precondition:** no causal bus route or combined
  publication/release state is deleted until this matrix passes.
- **API impact:** none.
- **Tests:** compare SIP request/response sequence, final state, transition
  history, tracker counts, registry counts, and retained tasks.
- **Complete when:** all authoritative outputs match and only observational
  delivery counters differ.
- **Risk/rollback:** none; a difference blocks release.

### TE-703 — Tie standards claims to executable evidence

- **Current:** **RFC_COMPLIANCE_MATRIX.md** and compatibility documents contain
  claims of varying evidence strength.
- **Gap:** a claim can outlive its ignored test or cover only request
  construction rather than wire behavior.
- **Cleanup:** give every verified claim an evidence ID pointing to a
  non-ignored unit/integration/wire/interop test and latest attested result.
  Downgrade unsupported claims rather than broadening implementation during
  cleanup.
- **Dependencies:** PR-401 through PR-405 and YA-501.
- **Deletion precondition:** no compatibility implementation is deleted while
  a retained verified claim still identifies it as required evidence.
- **API impact:** none.
- **Tests:** RFC 3261 core transactions/dialogs, RFC 3262 reliable provisional,
  RFC 3311 UPDATE, RFC 3515 REFER, RFC 3581 rport, RFC 3891 Replaces, RFC 4028
  session timers, RFC 5626 outbound, RFC 6665 subscriptions, and supported auth
  standards.
- **Complete when:** no “verified” row references ignored, manual-only, missing,
  or unattested evidence.
- **Risk/rollback:** documentation claim may be downgraded; do not add a new
  feature merely to preserve wording.

### TE-704 — Enforce zero-resource drain

- **Current:** the archived soak retained 27 objects and one receiver.
- **Gap:** aggregate totals do not directly identify owner/lifetime.
- **Cleanup:** assert zero exact sessions, registry entries, adapter
  compatibility maps, dialog maps/resources, media sessions/resources,
  receivers, mixers, SRTP state, retained tasks/timers, request trackers,
  deferred deliveries, transaction managers, and transaction runners after the
  documented drain interval. On failure, report owner class and exact
  generation without sensitive SIP data.
- **Dependencies:** PR-404, PR-405, EX-206, and RT-306.
- **Deletion precondition:** no resource fallback or compatibility map is
  removed before owner-specific drain assertions cover its replacement.
- **API impact:** none.
- **Tests:** success, setup failure, media failure, auth failure, cancel, BYE,
  timeout, transfer, shutdown, and high-churn soak.
- **Complete when:** no whitelist permits nonzero final retained resources.
- **Risk/rollback:** a bounded standards timer may legitimately remain before
  the drain deadline; set the deadline from protocol maxima and assert zero
  after it rather than waiving the object.
- **Implemented diagnostic ledger (2026-07-20):** the private perf snapshot now
  reports SessionStore secondary indexes; exact registry entries plus dialog
  and media associations; outbound initial-INVITE, tracked-request, deferred
  delivery, and registration-refresh task owners; pending exact-response
  obligations, retry records, deadlines, and fire-in-flight work; setup and
  teardown deadlines by owner class, fire-in-flight work, and retained runtime
  tasks; media reservations/resources/receivers/SRTP/mixers; transaction
  subscribers and process-global runners; and app-publisher/global-bus queue
  and observer state. Short resilience and long soak retention totals include
  every per-session owner above. Coordinator runtime tasks are reported but
  excluded from live-call totals because fixed scheduler/router workers remain
  while an endpoint is open; the shutdown test separately requires that count,
  both deadline queues, and both fire-in-flight counts to reach zero.

### TE-705 — Performance and soak qualification

- **Current:** interop/security passed in archived dirty-tree runs, while the
  monolithic soak failed despite acceptable post-drain RSS growth.
- **Gap:** functional cleanup can regress throughput, latency, allocation, or
  cleanup at scale.
- **Cleanup:** run targeted microbenchmarks after each map/commit hot-path
  change; then the complete endpoint, PBX media server, and signaling-only
  matrices, burst matrix, media churn, mid-call signaling, monolithic soak,
  split soak, and three canonical 2K runs.
- **Dependencies:** all implementation phases.
- **Deletion precondition:** final compatibility-path deletion waits for the
  full performance and soak qualification.
- **API impact:** none.
- **Tests:** existing beta performance targets with current checked thresholds;
  add per-session lane queue delay, commit count, tracker count, and retained
  task metrics.
- **Complete when:** no existing beta threshold regresses, all error counters
  are zero, all resources drain to zero, and all three 2K runs pass from the
  unchanged source fingerprint.
- **Risk/rollback:** identify the first regressing slice using per-PR benchmark
  artifacts; roll back that slice, not the authority invariants already proven.

### TE-706 — Produce mode-specific reports and machine-readable attestation

- **Current:** **beta_gate.sh** records environment/source data and hashes some
  canonical evidence, but summaries from separate modes can be mixed and the
  mutable target JSON has no intrinsic run identity.
- **Gap:** a release candidate needs one verifiable chain from source and
  inputs to every claimed artifact.
- **Cleanup:** write **attestation.json** at each report root with schema
  **rvoip-sip-beta-attestation-v1** and these required sections:
  source commit/tree/clean flag/fingerprint; mode and timestamps; compiler,
  target, features, and executable hashes; loaded YAML path/hash; effective
  redacted configuration hash; performance recipe/burst scenario hashes; peer
  product/version or container image digest and config hash; every gate status,
  duration, relative log path and SHA-256; result JSON hashes; canonical 2K
  index/run hashes; final failure/skip counts; and overall PASS/FAIL. Also emit
  **attestation.json.sha256**. Maintain separate latest pointers for local,
  interop, security, performance, and clean full modes; a generic
  **latest.txt** is informational and cannot select release evidence.
- **Dependencies:** FZ-003 and TE-703 through TE-705.
- **Deletion precondition:** no prior evidence pointer is retired until the new
  attestation verifies a copied report independently.
- **API impact:** none.
- **Tests:** schema validation, missing artifact, changed source, changed YAML,
  replaced result JSON, peer version absence, dirty tree, skip, and failed gate.
- **Complete when:** a standalone verifier can validate a copied report without
  reading mutable workspace paths.
- **Risk/rollback:** no signing key is introduced by this cleanup. SHA-256 and
  clean-source fencing provide internal consistency and reproducibility, not
  independent authenticity. Cryptographic signing requires a separate
  release-infrastructure decision.

## 14. State-Write Ledger

This is the reconciled source ledger. “Retained” means the write is owned and
narrowly scoped; “deleted/migrated” means the historical writer is absent from
the named site. Runtime qualification of these dispositions remains pending.

| ID | Current writer/symbol | Fields or authority | Disposition | Work item |
|---|---|---|---|---|
| SW-01 | **StateMachine::process_one_event** | event data, state, conditions, history | **Retained:** canonical lane-owned lifecycle writer with one final `commit_lane_state` per success/action-failure branch. | IN-101, EX-201 |
| SW-02 | **StateMachine::stage_outbound_options*** | pending method options | **Retained facade:** crate builders use the private atomic dispatch; the public two-step API remains compatible and guarded. | IN-102 |
| SW-03 | historical executor final compatibility reread | call state and `entered_state_at` | **Deleted:** frozen public SessionStore mutators now acquire the exact state-machine lane, so no competing write exists to reconcile. | EX-201, EX-204 |
| SW-04 | historical **SessionStore::replace_session_exact_inner** merge flags | tracked staging and auth coordination | **Deleted:** field-specific tracked-staging/auth merge policy is absent. | EX-204 |
| SW-05 | historical **RegistrationStateProjection** | registration/auth/digest fields | **Deleted:** typed REGISTER result is applied to the local lane state. | EX-203 |
| SW-06 | **SessionCrossCrateEventHandler::handle_auth_required_parts** | auth transport/transaction/URI | **Migrated:** typed exact lane input; no competing prewrite. | IN-101, IN-103 |
| SW-07 | **handle_transfer_requested_parts** and REFER default task | transfer target, transaction, metadata | **Migrated:** one exact transaction claim and retained generation-fenced grace task. | IN-103, EX-205 |
| SW-08 | inbound call setup handler | URIs, inbound INVITE transaction, dialog identity | **Implemented:** inputs enter under the exact lane; only narrow exact registry/dialog identity publication precedes required wire work. | IN-101, IN-104 |
| SW-09 | accept/provisional/reject/redirect/challenge/generic response paths | status, local SDP, response headers | **Migrated:** one `ResponseStateInput` envelope under the exact lane/action outcome. | IN-103 |
| SW-10 | REGISTER dialog/action boundary | CSeq, Call-ID, auth, result, refresh metadata | **Migrated:** typed attempt/result/post-commit effects; dialog code no longer owns session lifecycle state. | EX-203 |
| SW-11 | DialogAdapter initial INVITE publication | exact dialog identity before wire | **Retained narrow authority:** one generation-fenced identity publication required for synchronous peer response. | EX-201, PR-404 |
| SW-12 | DialogAdapter request auth bookkeeping | nonce count and transport context | **Consolidated:** lifecycle state is executor-owned; immutable request/tracker owners retain exact correlation. | PR-402, RT-306 |
| SW-13 | **MediaAdapter::record_media_security_negotiated** | media security state | **Migrated with public facade retained:** lane-owned result plus narrow exact compatibility projection; resource installation remains media-owned. | EX-202 |
| SW-14 | MediaAdapter bridge/mute/resource methods | media resource state and bridge lifetime | **Consolidated:** `bridge_rtp_sessions` plus its returned `BridgeHandle` is the real owner. Non-owning bridge, mixer, playback, recording, and raw media-allocation signatures remain source-compatible but fail closed instead of fabricating state, identifiers, or success. Mute alone delegates to media-core and revalidates the exact allocation after the operation. | EX-202, PR-404 |
| SW-15 | **state_machine/helpers.rs** historical direct session updates | leg links, subscriber/bridge lifecycle | **Migrated:** SIP lifecycle helpers enter the exact executor; media resources remain with their exact lower-layer owner. | EX-202, YA-502 |
| SW-16 | public API compatibility tests | synthetic direct writes | Keep under test-only allowlist; production scan excludes test modules. | FZ-002 |
| SW-17 | **SessionCrossCrateEventHandler::handle_media_to_session_event** | formerly dispatched lifecycle events from observational media-bus reports | Reporting only: retain quality/DTMF application projections and diagnostics; synchronous media actions/watchdogs own causal state. | RT-303 |

**SW-03 completion finding (2026-07-21):** a repository-wide production caller
scan found no non-executor signaling writer. Public internals writes retain
their source contract but queue behind the exact lane. The executor contains
no final store reread or field-selective reconciliation.

**SW-13 completion finding (2026-07-20):** production state-machine actions
and retained glare retries now advance SDP origin and apply negotiated media
security only to their lane-owned working `SessionState`; the executor's
SDP-origin and media-security reconciliation rereads have been deleted.
Public `MediaAdapter` methods keep their signatures and act as exact-lane
compatibility facades. Their one narrow projection commit revalidates the
captured `SessionStateSnapshot` and copies only SDP-origin/media-security
fields through `update_session_snapshot_with`; security observations are
queued only after that commit. The production
`MediaAdapter::update_session_with` inventory is now zero.

**SW-14 completion finding (2026-07-21):** `UnifiedCoordinator::bridge` calls
`bridge_rtp_sessions`, whose returned media-core `BridgeHandle` owns the real
RTP bridge and synchronously clears the controller partner map on drop. The
legacy `create_bridge`/`destroy_bridge` signatures cannot represent that
lifetime, so they now return an explicit `InvalidTransition` and never mutate
two session snapshots or claim success. No replacement bridge registry or
second bridge authority was introduced.

The same caller/public-contract audit found metadata-only mixer state, generated
recording identifiers, log-only playback/recording methods, and raw
create/stop-media facades. Their signatures remain stable, but the fake state
map was deleted and every unsupported operation now fails explicitly. The one
operation with a real lower-layer implementation, mute, resolves a live exact
media allocation, delegates to media-core, and rejects lifetime replacement.
`audio_mixers` remains a zero-valued compatibility diagnostics key rather than
a second ownership map.

The ledger is complete only when a repository-wide scan covers production uses
of **update_session_with**, **update_session_exact_with**, full snapshot
replacement, and mutable **SessionState** access. Read-only **with_session**
calls are not writers but delayed uses still require IN-104 review.

## 15. Causal Event Ingress/Egress Ledger

| Event family | Causal ingress and owner | State-machine effect | Observation point |
|---|---|---|---|
| Initial INVITE | dialog-core typed incoming call; creates exact lifetime | YAML **IncomingCall** row | after admission and committed session state |
| 1xx/2xx/3xx/4xx–6xx INVITE response | dialog transaction correlation, then exact routed event | YAML response row and media/session action | after commit |
| ACK | dialog-core generates/validates wire ACK | YAML **DialogACK** only when application lifecycle changes | after commit |
| BYE/CANCEL inbound | dialog-core sends required response and acknowledges exact delivery | YAML terminal/cancel row and exact release | after terminal commit; never blocks response/release |
| BYE/CANCEL outbound response | dialog transaction correlation | terminal confirmation and release | after commit |
| Auth required | exact challenged transaction/request owner | YAML **AuthRequired** retry policy | challenge/result after commit, with secrets removed |
| re-INVITE/UPDATE | dialog transaction and SDP input | YAML lifecycle/renegotiation rows | after commit |
| REGISTER result | dialog standalone transaction returns typed result | YAML registration/auth/refresh row | after commit |
| REFER/NOTIFY | dialog transaction/subscription correlation plus exact REFER claim | YAML transfer lifecycle where application-visible | after commit |
| INFO | dialog transaction and application response capability | YAML only for session-affecting send/result | sanitized application request/result after causal response |
| MESSAGE/OPTIONS | dialog-owned standalone/in-dialog mechanics | no artificial session transition unless a real call lifecycle field changes | application request/result event |
| SUBSCRIBE | dialog subscription manager and transaction | no artificial session state | typed subscription/NOTIFY event |
| PUBLISH | deferred; no rvoip-sip implementation added | none | none until separately implemented |
| Media lifecycle/quality | media-core typed result with exact session handle | YAML condition/lifecycle row where applicable | after commit |
| Transport failure/recovery | transport/dialog flow owner | typed registration/call failure or recovery event | after commit |

No row uses broadcast subscription as its causal ingress. Capability-bearing
events such as response transaction handles are never copied to observational
subscribers.

**Source disposition (2026-07-21):** this ledger is implemented. Typed causal
ingress is acknowledged by the sharded direct router, and sanitized reporting
is post-commit/nonblocking. Full healthy/absent/full/stalled/closed/shutdown
execution remains **PENDING** under TE-702.

## 16. SIP Method Cleanup Ledger

| Method | Retained lifecycle owner | Retained wire owner | Implemented disposition |
|---|---|---|---|
| INVITE | YAML/state machine | dialog/transaction | **Consolidated:** builders atomically dispatch one staged snapshot to `send_initial_invite_staged`; exact dialog identity is narrowly published before wire; auth reuses that snapshot. |
| re-INVITE | YAML/state machine | dialog/transaction | **Consolidated per in-dialog context:** exact tracker owns correlation, SDP enters under the lane, and glare retry is retained off-lane work. |
| ACK | YAML observes lifecycle consequence | dialog/transaction | **Direct/exact:** dialog-core owns generation/validation; no causal bus or debug-string wrapper. |
| BYE | YAML terminal intent/release | dialog/transaction | **Direct/exact:** one terminal claim/send and release independent of observation. |
| CANCEL | YAML cancel intent/release | dialog/transaction | **Direct/exact:** RFC legality and transaction correlation remain dialog-owned; no duplicate send path. |
| UPDATE | YAML for session modification/refresh result | dialog/transaction | **Consolidated per in-dialog context:** one options dispatch and exact tracker. |
| REGISTER | YAML registration lifecycle | dialog standalone transaction | **Consolidated:** all four action shapes use one typed attempt/result implementation; projection/direct lifecycle writes are deleted. |
| REFER | YAML transfer lifecycle/decision | dialog transaction plus implicit subscription mechanics | **Direct/exact:** one REFER claim, typed response capability, and retained generation-fenced grace timer. |
| INFO | YAML for outbound session action | dialog transaction | **Direct/exact:** one options dispatch, exact tracker, and acknowledged response-bearing ingress. |
| NOTIFY | YAML only for transfer/application lifecycle effect | dialog subscription/transaction | **Consolidated per subscription/in-dialog context:** options path plus exact REFER/subscription correlation. |
| MESSAGE | none for standalone messaging; YAML only for real session-affecting in-dialog use | dialog transaction | **Consolidated:** standalone MESSAGE uses the shared typed auth driver without a fake session; in-dialog correlation remains tracker-owned. |
| OPTIONS | none for standalone capability query | dialog standalone transaction | **Consolidated:** shared typed standalone auth driver; no fake session. |
| SUBSCRIBE | none unless a real exposed lifecycle is later specified | dialog subscription manager | **Consolidated mechanics:** shared standalone auth driver; RFC 6665 subscription policy/wire ownership remains dialog-owned. |
| PUBLISH | none; explicitly deferred | public dialog presence facade fails closed | **Unchanged/deferred:** no speculative implementation or compliance claim. |
| PRACK | YAML observes reliable provisional consequence where needed | dialog/transaction | **Retained direct owner:** no duplicate session wire implementation. |

These dispositions consolidate same-context implementations. They deliberately
do not collapse standards-distinct initial, in-dialog, standalone, or
subscription-refresh contexts. Wire/RFC/PBX proof for the table is **PENDING**.

## 17. Mapping and Resource Ledger

| Data | Canonical owner after cleanup | Allowed derived/resource view |
|---|---|---|
| Exact session lifetime and generation | **SessionRegistry** | immutable **SessionRegistryHandle** |
| Session to dialog association | **SessionRegistry** cross-layer; dialog-core internally | no DialogAdapter routing map or read-through cache |
| Dialog to session association | **SessionRegistry** cross-layer; dialog-core internally | no DialogAdapter reverse map or read-through cache |
| Call-ID to initial outbound owner | exact initial INVITE binding/registry slot | transaction-layer correlation and observational exact lookup only; no adapter cache |
| Dialog route set/tags/CSeq | dialog-core | immutable typed request/response options |
| In-dialog request transaction | **OutboundInDialogRequestTracker** plus dialog transaction manager | ephemeral pending slot before install |
| Media resource binding | MediaAdapter resource table registered to exact lifetime | SessionRegistry media association |
| Application lifecycle state | lane-owned **SessionState** | immutable snapshot for readers |
| Timer/task ownership | existing retained scheduler keyed by exact handle | diagnostic counters |

**Source disposition (2026-07-21):** adapter forward/reverse mapping maps and
their routing caches are removed; compatibility diagnostic keys remain fixed at
zero. Protocol retransmission caches and exact media resource tables are
retained because they own wire/resource mechanics rather than duplicate
cross-layer identity. Runtime drain/performance proof remains **PENDING**.

## 18. Deletion Ledger

The ordered source deletions are reconciled below. “Closed” records source
removal or an explicit retained compatibility boundary; final all-feature,
runtime, performance, and release evidence is still pending.

| Delete ID | Historical candidate | Reconciled disposition | Owning slice |
|---|---|---|---|
| DL-01 | 28 debug-string handlers and four extractors | **Deleted;** typed routing and a production source fence remain. | RT-304 |
| DL-02 | **RegistrationStateProjection** and sync helper | **Deleted;** typed REGISTER attempt/result owns the fields. | EX-203 |
| DL-03 | tracked-staging merge flag/branch | **Deleted;** crate builders use exact atomic dispatch. | EX-204 |
| DL-04 | auth-preservation flag/branch | **Deleted;** immutable exact request owners carry auth correlation. | EX-204 |
| DL-05 | executor SDP/media reconciliation rereads | **Deleted;** exact-lane public writers removed the last call-state/`entered_state_at` stop-boundary reconciliation. | EX-204 |
| DL-06 | state-machine action sleeps | **Deleted;** glare/REFER delays are retained exact scheduled work. | EX-205 |
| DL-07 | raw-ID REFER default task | **Deleted;** one generation-qualified transaction claim owns explicit/default response. | EX-205 |
| DL-08 | session-to-dialog response bus commands | **Deleted;** exact dialog/transaction APIs own responses. | RT-305 |
| DL-09 | duplicate same-context protocol implementations | **Closed for approved families:** initial INVITE, all REGISTER action shapes, and standalone MESSAGE delegate to one canonical implementation; retained helpers represent standards-distinct contexts or delegate. | PR-401 |
| DL-10 | scattered auth request reconstruction | **Closed:** INVITE, exact in-dialog tracker, REGISTER, and standalone driver retain their immutable request context. | PR-402 |
| DL-11 | DialogAdapter compatibility maps/caches | **Deleted;** zero-valued diagnostic compatibility keys only. | PR-404 |
| DL-12 | MediaAdapter duplicate routing maps/caches | **Deleted;** exact managed-resource tables remain. | PR-404 |
| DL-13 | private unreachable YAML/Rust variants | **Closed:** all 24 audited shapes have public serde, runtime-YAML, programmatic, or compliance owners; none meets all four deletion proofs. | YA-503 |
| DL-14 | combined publication/release completion states | **Deleted:** terminal release does not wait on observational publication. | EX-206 |
| DL-15 | four uncompiled orphan modules (`api/callbacks.rs`, `api/terminal.rs`, `session_store/inspection.rs`, `session_store/cleanup.rs`) | **Deleted;** absence fence retained. | YA-503 |

## 19. Test Migration Ledger

The source tree contains focused fixtures/fences for these categories, but this
documentation-only reconciliation did not execute them. Every row requires a
recorded run before it can contribute release evidence and is **PENDING**.

| Test ID | Scenario | Required assertions |
|---|---|---|
| TM-01 | response arrives before outbound action returns | exact dialog/request identity exists; one transition; no stale overwrite |
| TM-02 | two same-method sends | one exact request; loser gets existing conflict; no occupied slot after completion |
| TM-03 | builder future cancellation | exact staged pointer cleared; unrelated/newer request untouched |
| TM-04 | auth challenge race | same body/headers/route; correct auth header; one nonce-count advance |
| TM-05 | cancel versus 200 | legal ACK/CANCEL/BYE sequence; one terminal event/release |
| TM-06 | duplicate final response | one final transition and release; duplicates idempotent |
| TM-07 | glare | correct owner/non-owner delay ranges; lane remains available; retry cap |
| TM-08 | REFER explicit versus default | exact transaction claimed once; one final response |
| TM-09 | generation reuse | no callback/timer/cleanup from generation A mutates B |
| TM-10 | observer fault matrix | identical wire/state/release across observer modes |
| TM-11 | registration lifecycle | Call-ID/CSeq/auth/423/refresh/GRUU/Service-Route preserved |
| TM-12 | shutdown | all queues/tasks/timers drain or cancel; no accepted causal event dropped |
| TM-13 | resource drain | every owner class zero after protocol drain deadline |
| TM-14 | PBX/strict-UA | Asterisk, FreeSWITCH, SIPp, and baresip retained profile passes |
| TM-15 | API compatibility | public snapshot, semver, downstream matrix, examples and docs pass |

Tests that assert compensation behavior are replaced only after the
corresponding competing writer is gone. Tests for exact identity, transaction
correlation, wire compliance, and public behavior are retained permanently.

## 20. Implementation Slice Sequence and Checkpoints

The labels below preserve the intended review/rollback sequence; they do not
assert that the shared working-tree implementation landed as numbered Git pull
requests or commits. Source slices 0–12 are represented in the reconciled tree.
Slice 13, including the required full qualification evidence, is **PENDING**.

1. **PR 0 — fences only:** FZ-001 through FZ-004. No runtime behavior change.
2. **PR 1 — exact transition input:** IN-101, then migrate SDP and response
   extras.
3. **PR 2 — atomic builder dispatch:** IN-102 method by method; retain public
   facade.
4. **PR 3 — exact ingress/tasks:** IN-103 through IN-105, including REFER
   claim and delayed-work fencing.
5. **PR 4 — executor outcomes:** EX-201 and EX-202 in vertical method slices.
6. **PR 5 — registration ownership:** EX-203 with full PBX registration suite.
7. **PR 6 — compensation deletion:** EX-204 only after zero-hit evidence.
8. **PR 7 — scheduler/terminal isolation:** EX-205 and EX-206.
9. **PR 8 — causal routing:** RT-301 through RT-306; delete string routing and
   response bus commands.
10. **PR 9 — protocol consolidation:** PR-401 through PR-403 one method family
    at a time.
11. **PR 10 — mapping consolidation:** PR-404 and PR-405 one map/timer family
    at a time, with perf evidence.
12. **PR 11 — YAML cleanup:** YA-501 through YA-504; private deletions only.
13. **PR 12 — compatibility projection:** AP-601 through AP-603.
14. **PR 13 — release evidence:** TE-701 through TE-706 and full clean-tree
    qualification.

At each checkpoint:

- the supported API snapshot is identical;
- the static exception allowlists do not grow;
- all migrated method tests pass;
- retained-object counts do not increase;
- no fallback/dual path is added; and
- a hot-path performance comparison is attached when store, mapping, routing,
  or request construction changed.

If a checkpoint cannot pass without a public API change or replacement of a
retained architecture component, stop. Do not hide the mismatch behind a
compatibility branch.

## 21. Final Release Procedure and Acceptance

> **CURRENT STATUS: PENDING.** The commands in this section have not been
> recorded as a completed final qualification for the reconciled source tree.
> There is no final PASS claim, no final release-candidate artifact directory,
> and no final attestation path in these documents. Fill those facts only from
> the actual clean-tree run; do not reuse the historical July 20 artifact paths.

The final candidate must be built from a clean, unchanged source tree. First
run the existing canonical 2K clean profile three times and record the three
printed artifact directories:

```sh
crates/sip/rvoip-sip/scripts/perf_call_setup_2k_profile.sh clean
crates/sip/rvoip-sip/scripts/perf_call_setup_2k_profile.sh clean
crates/sip/rvoip-sip/scripts/perf_call_setup_2k_profile.sh clean
```

Export `RVOIP_STRICT_UA_HOST_IP` with the reachable local host address and
`BETA_CANONICAL_2K_RUN_DIRS` with the three absolute run directories in
oldest-to-newest, colon-separated order. The checks below make an unset value
fail safely instead of interpreting a placeholder as shell syntax:

```sh
: "${RVOIP_STRICT_UA_HOST_IP:?export the reachable strict-UA host IP}"
: "${BETA_CANONICAL_2K_RUN_DIRS:?export three canonical run directories}"

RVOIP_STRICT_UA_HOST_IP="$RVOIP_STRICT_UA_HOST_IP" \
RVOIP_REQUIRE_API_TOOLS=1 \
BETA_REPORT_PACKAGE=1 \
BETA_REQUIRE_CANONICAL_2K_EVIDENCE=1 \
BETA_CANONICAL_2K_RUN_DIRS="$BETA_CANONICAL_2K_RUN_DIRS" \
BETA_RUN_LOCAL_PBX=1 \
BETA_RESTORE_LOCAL_PBX=1 \
BETA_PBX_PROVIDER=both \
BETA_PBX_API=all \
BETA_PBX_SCENARIO=all \
BETA_PBX_G729_PROFILES="g729a g729ab" \
BETA_RUN_SIPP=1 \
BETA_SIPP_CPS="30 100 300 1000 2000" \
BETA_SIPP_DIAGNOSTICS=0 \
BETA_RUN_STRICT_UA=1 \
BETA_RUN_FUZZ_SMOKE=1 \
BETA_RUN_PERF_ALL=1 \
BETA_PERF_REGRESSION_FAIL=1 \
BETA_PERF_REGRESSION_BASELINE_ROOT=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z \
BETA_PERF_REGRESSION_BASELINE_MANIFEST=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json \
BETA_RUN_BURST_SMOKE=1 \
BETA_RUN_BURST_MATRIX=1 \
BETA_BURST_MATRIX=all \
BETA_RUN_LONG_SOAK=1 \
BETA_PERF_MEDIA_CHURN_DURATION_SECS=120 \
BETA_PERF_MONOLITHIC_SOAK_DURATION_SECS=3600 \
RVOIP_PERF_SOAK_DURATION_SECS=3600 \
RVOIP_PERF_SOAK_ACTIVE_CALLS=500 \
RVOIP_PERF_SOAK_MIN_HOLD_SECS=10 \
RVOIP_PERF_SOAK_MAX_HOLD_SECS=360 \
RVOIP_PERF_SOAK_CPS=0 \
RVOIP_PERF_SOAK_DRAIN_CPS=10 \
RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS=120 \
RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR=10 \
crates/sip/rvoip-sip/scripts/beta_gate.sh --full --require-external
```

Release acceptance requires all of the following:

- zero failures and zero skips in required gates;
- identical public API snapshot and passing semver/downstream matrices;
- no duplicate BYE, CANCEL, ACK, REFER response, or final response;
- zero call, media-setup, and teardown errors in both soaks;
- zero retained sessions, registry entries, dialogs, mappings, media
  resources, receivers, timers, retained tasks, trackers, deferred events,
  transaction managers, and transaction runners after drain;
- observer isolation tests pass in every fault mode;
- RFC verified claims point to non-ignored attested evidence;
- Asterisk, FreeSWITCH, SIPp, and strict-UA matrices pass for the retained
  profile;
- all three canonical 2K runs pass and the source fingerprint is unchanged;
- the full performance matrix stays within existing beta thresholds;
- the hard performance-regression audit passes against its attested baseline;
- mode-specific latest pointers resolve to the correct attested artifacts and
  a generic latest pointer is not used for release selection; and
- **attestation.json** and its SHA-256 verify every report input and artifact.

The cleanup is complete only after the deletion ledger is closed, the static
allowlists have reached their intended minimum, and no correctness path
depends on the observational event bus.

## 22. Explicitly Out of Scope

- replacing YAML/runtime loading with generated Rust;
- splitting the existing state machine into new actors or domain runtimes;
- introducing a new event bus, mailbox framework, request engine, registry, or
  store;
- forcing standalone MESSAGE, OPTIONS, or SUBSCRIBE through artificial
  sessions;
- implementing speculative PUBLISH support;
- changing public **src/api** interfaces, events, configuration, builders, or
  errors;
- removing standards-conforming compatibility needed for interoperable peers;
- changing performance thresholds to make a regression pass; or
- retaining old and new protocol implementations behind a runtime switch.
