# SIP Signaling Single-Authority Cleanup Plan

- **Status:** Source cleanup implemented; final qualification pending
  (authorized 2026-07-20, reconciled 2026-07-21)
- **Audience:** `rvoip-sip`, `sip-dialog`, transport, media, test, and release maintainers
- **Companion document:** [SIP Signaling Single-Authority Implementation Plan](SIP_SIGNALING_SINGLE_AUTHORITY_IMPLEMENTATION_PLAN.md)

> **Release evidence status:** **PENDING.** This document records the source
> cleanup now present in the working tree. It does not claim that the final
> Cargo matrix, beta gate, performance/soak runs, PBX/strict-UA matrices, or
> attestation have passed. Those claims may be added only from the final
> clean-tree run described in the companion plan.

## Executive decision

The current SIP architecture is suitable for cleanup. It does not require a
rewrite.

The repository already contains the architectural components needed for a
single-authority, standards-conforming, high-performance implementation:

- a runtime-loaded YAML state table in `state_tables/default.yaml` through
  `YamlTableLoader`;
- the existing `StateMachine` executor and action/guard model;
- exact-lifetime session identity through `SessionRegistry`;
- session state storage through `SessionStore`;
- an exact-session execution lane attached to each session state cell;
- typed dialog ingress through `DialogToSessionEvent` and the existing sharded
  `DialogToSessionDirectRouter`;
- mature SIP dialog and transaction machinery;
- explicit authoritative and observational delivery modes in
  `GlobalEventCoordinator`.

The cleanup retains and tightens those components. The implemented source
changes remove duplicate writers, mirrored indexes, debug-string handlers,
causal event-bus hops,
compensating snapshot merges, and overlapping request implementations around
them. It will not replace them with a new architecture.

The desired result is one authoritative path for every state-changing SIP
operation while preserving the supported public API, IETF-conforming wire
behavior, interoperability, and the existing performance model.

## Why cleanup is the correct response

### Evidence baseline

The July 20, 2026 beta evidence establishes two different facts that must not
be conflated:

1. The protocol architecture works across broad functional, security, and
   performance coverage.
2. Long-running concurrency can still expose inconsistent lifecycle ownership
   and incomplete teardown.

The interop run at source revision `85b932e4` passed the local Asterisk and
FreeSWITCH matrices, the SIPp standalone matrix, and the baresip strict-UA
matrix with zero recorded failures or skips. The security run at the same
revision passed its dependency-audit and parser-fuzz gates. In the performance
run, every listed performance and resiliency gate before the monolithic soak
passed, including call setup, registration, active calls, RTP, backpressure,
transport recovery, mid-call signaling, TLS, SRTP, session churn, transfers,
and media churn.

The monolithic soak then completed 5,012 of 5,016 offered calls (`0.9992` ASR)
but failed its release gate with:

- 4 call failures;
- 3 media setup failures;
- 1 teardown failure;
- 27 retained objects after drain;
- 1 active Bob audio receiver after drain.

That failure shape is narrow and lifecycle-oriented. It does not show that the
YAML state-machine architecture, dialog/transaction core, transport stack, or
media stack must be replaced. It does show that the boundaries between them
must become exclusive and exact-lifetime-safe.

The cited July runs were made from a dirty tree, and the current checkout is a
later, still-unqualified working tree. They are historical diagnostic evidence,
not current release evidence or publish attestation. Logs,
JSON, summaries, and binaries from different revisions or timestamps must not
be combined into one release claim. The final cleanup gate must run from a
clean tree and bind every artifact to its exact source and configuration.

### Current implementation evidence

The source reconciliation on 2026-07-21 found the planned cleanup architecture
in place:

- `EventStateInput` and `ResponseStateInput` carry transition and complete
  response-envelope data under the exact-session lane. Accept, provisional,
  reject, redirect, challenge, and generic response paths consume the same
  typed envelope rather than prewriting response fields.
- `StateMachine::process_one_event` retains one lane-owned working state and
  uses the canonical `commit_lane_state` publication at the end of the
  successful or action-failure branch. Narrow identity/resource publication
  remains only where the wire operation requires it. The frozen public
  `internals::SessionStore` mutators now queue on that same exact-session lane,
  so the executor no longer rereads or reconciles a competing public write.
- Generation-qualified `SessionRegistryHandle` values are captured by typed
  ingress, incoming-call/response builders, trackers, timers, and retained
  tasks. Work admitted for an older lifetime fails closed after raw-ID reuse.
- The tracked-staging/auth-preservation merge flags,
  `RegistrationStateProjection`, and its synchronization helper are deleted.
  REGISTER variants share one typed attempt/result implementation and apply
  post-commit timer/observer effects after the canonical state commit.
- Initial INVITE variants delegate to `send_initial_invite_staged`, preserving
  one immutable staged request snapshot across authentication. Standalone
  MESSAGE, OPTIONS, and SUBSCRIBE delegate to one typed transaction/auth
  driver without synthetic state-machine sessions; in-dialog methods retain
  the exact outbound request tracker.
- Session-affecting dialog ingress is typed and acknowledged through the
  sharded direct router. The 28 debug-string wrappers and four extraction
  helpers are deleted, response-bearing session-to-dialog bus commands are
  replaced by exact dialog operations, and observational publication is
  non-causal and post-commit.
- Adapter-owned forward/reverse dialog and media mapping mirrors are deleted.
  `SessionRegistry` owns exact cross-layer associations; dialog-core owns its
  protocol maps; media retains only exact managed-resource tables.
- Bidirectional YAML/wiring checks are present. The deletion audit now records
  24 retained public serde/runtime-YAML/programmatic compatibility shapes,
  including the three legacy REGISTER action names that delegate to the one
  canonical REGISTER implementation. No externally accepted YAML grammar was
  narrowed.

These are source-level findings, not final runtime qualification. The full
Cargo, beta, performance, soak, PBX, strict-UA, drain, and attestation evidence
remains pending.

## Required outcome

At completion, each state-changing fact has exactly one authoritative owner
and one serialized mutation path:

```text
public API or typed dialog input
              |
              v
 exact-lifetime identity and per-session lane
              |
              v
   YAML transition + ordered actions
              |
              v
      one canonical state commit
              |
      +-------+--------+
      |                |
      v                v
dialog/transport/   post-commit public and
media effects       observational reporting
```

Standalone transaction-oriented methods that do not own an application
session lifecycle use the dialog/transaction path directly. They do not create
artificial `SessionState` entries merely to appear in the YAML table.

## Normative cleanup invariants

The words **MUST**, **MUST NOT**, **SHOULD**, and **MAY** in this document define
acceptance requirements for the cleanup.

### Single lifecycle authority

- The YAML table and existing state-machine executor **MUST** remain the source
  of truth for legal call, registration, and application-visible session
  lifecycle transitions.
- A transition's guards, next state, ordered actions, retry decision, and
  cleanup decision **MUST** have one authoritative definition.
- API builders, adapters, dialog callbacks, timers, and event handlers **MUST
  NOT** independently decide or write the same lifecycle transition.
- An exact session's state-changing input **MUST** execute under that session's
  existing lane and generation-qualified identity.
- A stale callback, timer, response, or task from a previous session generation
  **MUST NOT** mutate a newer session that reused the same external identifier.
- An event execution **MUST** produce one canonical committed state. Full-state
  rereads, field projection, and selective merge are temporary compensation,
  not the target design.

### Effect boundaries

- State-machine actions **MAY** ask dialog, transaction, transport, or media
  owners to perform effects; those owners **MUST** return typed outcomes rather
  than write session lifecycle state independently.
- Dialog and transaction code **MUST** remain authoritative for SIP wire
  mechanics: dialog identifiers, route sets, remote targets, tags, CSeq,
  transaction correlation, retransmission, ACK/CANCEL/PRACK mechanics, and
  response construction.
- Transport code **MUST** remain authoritative for DNS resolution, connection
  and flow management, wire I/O, and transport recovery.
- Media code **MUST** remain authoritative for SDP/RTP resource operations.
  Whether a resource belongs to a live application session **MUST** be decided
  using the exact session lifetime.
- A lane **MUST NOT** be held during a retry delay, refresh interval, network
  wait, observer wait, or other wall-clock sleep. Existing lifecycle-owned task
  and scheduler mechanisms must deliver a generation-qualified input when the
  delay expires.

### Reporting is not control flow

- Event publication **MUST NOT** be required for request transmission,
  response handling, state progression, timer progression, teardown, or exact
  resource release.
- Causal typed delivery **MUST** use the existing authoritative handler path.
- The event bus **MAY** report sanitized, committed outcomes to application,
  diagnostic, and metrics consumers.
- A full, stalled, closed, absent, or slow observer **MUST NOT** alter SIP wire
  behavior or session outcomes.
- No pre-commit `SessionState` snapshot **MAY** escape as an authoritative
  observation.
- Terminal public events **MUST** remain exactly-once according to the current
  API contract even though internal delivery is simplified.

### Delete compensation only after deleting its cause

- Every cleanup slice **MUST** remove or reduce at least one duplicate writer,
  index, handler, routing hop, reconciliation step, fallback, or staging path.
- A replacement subsystem is not an acceptable cleanup result.
- Compensating code **MUST** remain until all writers or consumers that require
  it have migrated and the relevant race tests pass.
- Static exceptions and allowlists **MUST** shrink monotonically. New entries
  require a documented stop-condition review.
- A legacy path **MUST NOT** remain as a silent fallback after its canonical
  replacement is active.
- Temporary dual wiring is permitted only inside a bounded migration slice,
  with equivalence tests and a deletion condition in that same slice or its
  immediately dependent slice. A release candidate **MUST NOT** contain two
  active authorities for one operation.

## Retained ownership model

| Concern | Authoritative owner | What that owner decides | What it must not own |
|---|---|---|---|
| Call, registration, and application session lifecycle | Runtime YAML plus existing `StateMachine` | Legal transitions, guards, action order, lifecycle retries, terminal state, session cleanup intent | SIP transaction mechanics, wire retransmission, transport I/O, media internals |
| Exact session identity | `SessionRegistry` and generation-qualified handles | Whether a callback or resource belongs to the current lifetime | Protocol state or public event delivery |
| Session state and serialization | `SessionStore`, exact state cell, and per-session lane | Current committed state and ordered mutation for one lifetime | Independent protocol decisions or observer backpressure |
| SIP dialog and transactions | `sip-dialog` dialog/transaction managers | Dialog identity and routing, tags, CSeq, transactions, retransmissions, request/response correlation, ACK/CANCEL/PRACK mechanics | Application session lifecycle or duplicate session snapshots |
| SIP orchestration adapter | Retained `DialogAdapter` as a narrow boundary | Translate typed state-machine requests to one dialog method and typed outcomes back | Mirrored lifecycle truth, duplicate mappings, alternative request construction, silent success |
| Transport | SIP transport layer | DNS, connection/flow state, byte I/O, transport recovery | Session lifecycle transitions |
| Media | Media adapter and media subsystem | SDP/RTP resource creation, negotiation effects, flow, and release | Independent authority over whether a session lifetime is current |
| Application events and diagnostics | Existing public event publishers and observational bus | Report post-commit facts and metrics | Causal signaling, correctness, retry, or cleanup |

The owner of a fact owns its canonical index. Other components may hold a
generation-qualified reference or query the owner; they must not maintain an
independent mutable map with overlapping truth. In particular, dialog-to-
session, session-to-dialog, Call-ID, media-to-session, and timer ownership maps
must each have one canonical mutable owner after their migration slice.

## SIP method ownership matrix

The matrix distinguishes lifecycle decisions from protocol mechanics. A row
may intentionally involve both owners; that is composition, not duplicate
authority.

| SIP method | Application entry/meaning | Lifecycle authority | Dialog/transaction authority | Cleanup boundary |
|---|---|---|---|---|
| **INVITE (initial)** | Start or receive a call and establish its media-bearing session | YAML/state machine owns admission result, call states, offer/answer lifecycle, ordered setup actions, failure, cancellation, and cleanup | Dialog/transaction owns UAC/UAS request and response mechanics, early and confirmed dialogs, tags, routes, CSeq, provisional/final transactions, retransmission, and ACK correlation | Carry all initial request and response context as typed exact-lifetime input; remove API/adapter prewrites and duplicate mapping/state decisions |
| **re-INVITE** | Modify an established session, including hold, resume, and media renegotiation | YAML/state machine owns legality, glare policy decision, session/media lifecycle result, retry intent, and failure reporting | Dialog/transaction owns in-dialog construction, route/CSeq, offer/answer wire exchange, 491 response mechanics, and transaction correlation | Use one options-based send implementation; schedule generation-qualified glare retry outside the lane; no parallel `pending_reinvite` writer |
| **ACK** | Complete INVITE transaction/dialog establishment | YAML/state machine observes only the exact-session lifecycle consequence where one exists | Dialog/transaction owns ACK construction, transmission, duplicate-final handling, 2xx cache interaction, and transaction/dialog correlation | Keep ACK off the event bus hot path; deliver only typed exact-session facts needed by lifecycle; prevent duplicate or stale ACK effects |
| **BYE** | Terminate an established call | YAML/state machine owns local hangup intent, terminal transition, exactly-once cleanup, and public terminal outcome | Dialog/transaction owns BYE construction, routing, transaction, final response, inbound 200 response, and duplicate-wire handling | One component claims terminal release; inbound and outbound races converge on one exact lifetime without duplicate BYE or cleanup |
| **CANCEL** | Cancel a pending initial INVITE | YAML/state machine owns cancellation legality and resulting call lifecycle | Dialog/transaction owns INVITE correlation, CANCEL construction, 200-to-CANCEL, 487-to-INVITE, retransmission, and late-final races | Remove parallel cancel/terminate decisions; preserve provisional-response legality and race-safe final-response handling |
| **UPDATE** | Modify session parameters without changing the dialog | YAML/state machine owns application-visible session/media changes and their legality | Dialog/transaction owns in-dialog request construction, route/CSeq, authentication retry mechanics, and response correlation | One typed staged command and one options-based send; no direct store update before or after dispatch |
| **REGISTER** | Create, refresh, or remove a registration | YAML/state machine owns registration states, authentication lifecycle, 423 retry policy, refresh/unregister intent, expiry, failure, and terminal result; an inbound registrar application owns binding acceptance | Dialog/transaction owns REGISTER construction, transaction correlation, challenge parsing/application, and one wire response materializer; transport owns registered flow | Eliminate projection/reread compensation and duplicate refresh ownership; timers re-enter through an exact generation-qualified lifecycle input; absence of a registrar returns 503 and the retired auto-registrar flag returns 501 rather than fabricating 200 |
| **REFER** | Request transfer and track its application-visible progress | YAML/state machine owns transfer request legality, application transfer lifecycle, retry/failure, and terminal public result | Dialog/transaction owns REFER request/response mechanics and event-package correlation | Replace bus-mediated response commands with exact dialog calls; timers and NOTIFY results target the exact session lifetime |
| **INFO** | Send or receive in-dialog application information, including supported DTMF usage | YAML/state machine owns any configured application-visible session consequence; ordinary send remains an active-state action | Dialog/transaction owns INFO package/body wire mechanics, route/CSeq, authentication retry, and response correlation | Collapse to one options-based request/auth path; inbound delivery is typed and does not mutate lifecycle unless an explicit YAML event requires it |
| **NOTIFY** | Deliver subscription or transfer progress | YAML/state machine owns transfer/application-session progress only where the notification changes that lifecycle | Dialog/transaction owns RFC 6665 dialog, subscription correlation, Event/Subscription-State mechanics, route/CSeq, and response transaction | Keep RFC 6665 mechanics dialog-owned; deliver typed results directly; do not invent a call session for standalone subscriptions |
| **MESSAGE** | Send or receive an instant message | No synthetic session lifecycle for standalone MESSAGE; an existing application's explicitly modeled lifecycle remains YAML-owned | Dialog/transaction owns standalone and in-dialog request construction, route/CSeq where applicable, transaction correlation, authentication retry, and response | Keep OOB MESSAGE direct; share one immutable request/auth implementation; publish inbound message content as an application fact, not as causal control flow |
| **OPTIONS** | Query or report SIP capabilities | No synthetic session lifecycle for standalone OPTIONS | Dialog/transaction owns request/response construction, Allow/Supported/capability headers, route/CSeq where applicable, transaction, authentication retry, and inbound response | Keep OOB OPTIONS direct; share one immutable request/auth implementation; observer availability cannot affect response |
| **SUBSCRIBE** | Create, refresh, or terminate an RFC 6665 subscription | Subscription protocol lifetime remains dialog-managed; it must not be represented as an artificial call `SessionState`. Any separate application-visible lifecycle must be explicitly modeled before YAML can own it | Dialog/transaction owns SUBSCRIBE transaction, subscription dialog, Event/Expires, refresh, authentication, NOTIFY correlation, and termination mechanics | Consolidate direct send/auth/refresh mechanics and exact ownership; do not force the current public API through temporary state-machine sessions |
| **PUBLISH** | Publish event state | No live `rvoip-sip` lifecycle is currently claimed | The retained public facade fails closed because no transaction-backed implementation exists | Leave deferred. Do not add speculative PUBLISH behavior, state rows, or public claims during cleanup |

**Source disposition (2026-07-21):** these boundaries are implemented without
collapsing standards-distinct initial, in-dialog, standalone, and subscription
contexts. Initial INVITE, all REGISTER action shapes, and standalone MESSAGE
have one canonical internal implementation; MESSAGE/OPTIONS/SUBSCRIBE share
standalone auth/transaction mechanics; all session-affecting results re-enter
typed exact lifecycle handling. PUBLISH remains deferred. Wire/RFC/PBX and
performance qualification for this matrix is **PENDING**.

### Cross-method rules

- Authentication changes headers and transaction data for the same immutable
  logical request. An authentication retry must not reconstruct a semantically
  different request through a second code path.
- Request builders may collect input, but they must not become state owners.
- In-dialog request tracking and generation fencing remain in place and are
  consolidated, not replaced.
- A method that changes a call, registration, transfer, or other currently
  modeled application session must re-enter the existing YAML executor.
- A method that is a standalone transaction or dialog protocol must not create
  a temporary application session simply to reuse YAML.
- The generated `state-machine-wiring.md` remains the auditable inventory of
  which methods are state-table, direct, dialog-managed, transport-only, or
  deferred. Its source manifest and generated document must agree.

## Public API compatibility contract

This cleanup has a hard source- and behavior-compatibility constraint for the
supported `rvoip-sip/src/api` surface and its existing root and prelude
reexports.

The cleanup **MUST NOT** change:

- `Endpoint`, `StreamPeer`, `CallbackPeer`, `SessionHandle`, or
  `UnifiedCoordinator` public semantics;
- public method names, signatures, generic bounds, async behavior, return
  types, or error types;
- public builders, option types, defaults, validation behavior, or send/result
  contracts;
- public configuration types or fields, including `Config.state_table_path`;
- public event types, variants, payloads, callback/stream delivery contracts,
  terminal-event uniqueness, or documented ordering;
- supported public traits and their required methods;
- public root and prelude paths used by downstream crates and examples;
- the public `stage_outbound_options` and `dispatch_outbound` two-step
  interface.

The two-step outbound interface may become a compatibility facade over one
guarded internal operation. Ordinary builders should use an atomic internal
path, but existing callers of the public two-step API must continue to compile
and observe the same documented semantics.

Internal mutation hooks, staging structures, debug-string routing, duplicated
maps, and fallback methods may be removed only after proving that they are not
part of the supported public contract. An item being declared `pub` outside
`src/api` is not automatic permission to remove it: root/prelude reachability,
workspace consumers, examples, and external compile fixtures must first be
checked.

Any necessary public API change is a separate, explicitly approved and
versioned project. It is not permitted to enter this cleanup indirectly.

## Standards and interoperability contract

“Legacy code” and “protocol compatibility” are different things.

The cleanup removes obsolete internal paths. It does not remove behavior
needed to comply with IETF SIP standards or interoperate with deployed peers.
Optimizations are permitted only when they remain conforming on the wire and
preserve observable semantics.

The implementation must therefore:

- preserve the currently claimed RFC 3261 core behavior and the supported
  portions of RFC 3262, RFC 3263, RFC 3264, RFC 3515, RFC 3581, RFC 4028, RFC
  4475, RFC 5626, RFC 6086, RFC 7118, and applicable SDP/RTP/SRTP standards;
- preserve RFC 6665 subscription mechanics for SUBSCRIBE/NOTIFY even though
  they are intentionally dialog-owned rather than call-state-machine-owned;
- retain standardized authentication and deployed-peer compatibility needed by
  the supported profile, including supported Digest behavior, unless a
  separate security decision changes that profile;
- preserve transaction timers, retransmissions, routing, CSeq, tags, branch
  handling, duplicate response handling, and dialog matching;
- test legal race behavior such as CANCEL versus final INVITE response, BYE
  versus transport failure, re-INVITE glare, authentication challenges, and
  stale delayed callbacks;
- keep partial and deferred RFC claims honest. Cleanup is not evidence for
  promoting an RFC row by itself;
- map every published standards or interoperability claim to a non-ignored,
  executable test or archived external matrix artifact;
- continue passing Asterisk, FreeSWITCH, SIPp, and strict-UA coverage for the
  retained feature profile.

If deleting a path would remove a required standards behavior, that path is not
deleted until the canonical owner implements and proves the same behavior. No
internal compatibility shim survives merely because it is old; no conforming
wire behavior is discarded merely because it is old.

## Cleanup sequence and dependency rules

The companion implementation plan provides the complete task ledger. All work
must follow these architectural phases in order.

The source cleanup has traversed these phases. Their ordering remains
normative for review and rollback: compensation was removed only after its
competing writer or fallback was removed. Final qualification of the resulting
tree is still Phase 7 work and is **PENDING**.

### Phase 1: Freeze compatibility and inventory authority

- Snapshot the supported public API and compile representative downstream
  consumers before internal deletion.
- Record every lifecycle writer, state replacement, mapping owner, causal event
  route, timer, request constructor, authentication retry path, and fallback.
- Add architecture tests that make duplicate-authority counts and allowlists
  visible.
- Treat the generated wiring manifest as an enforced ownership contract.

### Phase 2: Close exact-session mutation bypasses

- Carry request, SDP, authentication, transaction, response, transfer, and
  timer context as typed executor input.
- Move state-changing API, adapter, helper, callback, and timer work behind the
  existing exact-session lane.
- Make normal builders use one atomic guarded path while retaining the public
  two-step compatibility facade.
- Reject stale generation-qualified work before it can perform an effect.

### Phase 3: Consolidate commit and remove state compensation

- Use one lane-owned working state and one canonical commit per input.
- Make actions return typed state/effect outcomes instead of directly replacing
  store state.
- Remove reconciliation rereads, `RegistrationStateProjection`, selective
  staging merge, and authentication preservation only after their external
  writers are gone.
- Move sleeps and long waits out of the lane using existing lifecycle task
  facilities.

### Phase 4: Remove duplicate routing

- Use typed authoritative dialog-to-session delivery through the existing
  sharded router.
- Install the causal sink before any transport can receive inbound traffic.
- Delete legacy debug-string event handlers and extraction helpers after typed
  parity tests cover their events.
- Replace session-to-dialog bus commands with exact dialog APIs where a direct
  causal result is required.
- Publish only post-commit observational copies.

### Phase 5: Consolidate protocol method implementations

- Retain `DialogAdapter` as a thin boundary and reduce each method to one
  options-based implementation.
- Use one immutable request snapshot across initial send and authentication
  retry.
- Consolidate standalone MESSAGE, OPTIONS, and SUBSCRIBE mechanics without
  synthetic state-machine sessions.
- Select one canonical owner for each mapping and remove adapter mirrors after
  migration.
- Remove false-success, unused, misleading, and fallback methods only after
  zero-caller and public-contract proof.

The completed audit preserves source signatures but removes fabricated
behavior: metadata-only mixer ownership was deleted; unsupported recording,
playback, raw media-allocation, recovery, and non-owning bridge facades fail
closed; mute delegates to the exact live media-core allocation; and all generic
resource-cleanup spellings share one exact dialog/media release implementation.

### Phase 6: Clean YAML and unreachable machinery

- Keep `state_tables/default.yaml`, `YamlTableLoader`, runtime loading, guards,
  actions, and the existing executor architecture.
- Add bidirectional reachability checks among YAML events, states, guards,
  actions, templates, and typed runtime definitions.
- Move only genuinely duplicated lifecycle decisions into the existing table.
- Delete unreachable state/action/event/effect machinery with executable
  evidence.
- Preserve the externally configurable YAML grammar, the
  `Config.state_table_path` field, and valid custom-table behavior. Selection
  of a configured table versus the embedded fallback must be explicit and
  attested; changing invalid-path fallback semantics requires a separate
  compatibility decision.

### Phase 7: Prove independence and release readiness

- Prove identical signaling with healthy, saturated, stalled, closed, and
  absent observers.
- Run deterministic race, resource-drain, RFC wire, PBX, strict-UA,
  performance, split-soak, and monolithic-soak gates.
- Generate reporting and a machine-readable attestation tied to a clean source
  tree, exact binary, YAML, configuration, peer versions, and artifacts.

### Slice rules

Each implementation slice must:

1. name the duplicate authority being removed;
2. identify its canonical retained owner;
3. add or strengthen a failing-before/passing-after test;
4. migrate callers without changing the supported public API;
5. delete the old path and its compensation when safe;
6. demonstrate no regression in conformance or performance;
7. have a rollback boundary that restores one authority, never two.

Large mechanical deletions are allowed only after the relevant vertical slice
has proved behavior. A broad “clean up later” dual-path phase is not allowed.

## Stop conditions and escalation

Stop the affected slice and escalate for architecture review if any of the
following is true:

- completion requires changing a supported `rvoip-sip/src/api` interface,
  public event contract, root/prelude import path, configuration field, or
  downstream source contract;
- completion requires replacing runtime YAML loading, the current state-machine
  executor, `SessionRegistry`, `SessionStore`, the exact-session lane, the
  dialog/transaction core, transport architecture, or media architecture;
- the only proposed solution is a new actor system, mailbox framework, request
  engine, code-generated state architecture, or second session model;
- a supposedly obsolete path has a live caller, an untested standards
  responsibility, or downstream usage that cannot be migrated within the
  approved boundary;
- one operation cannot be assigned a single authoritative owner without a
  product or public-API decision;
- an optimization changes SIP wire semantics, weakens an RFC behavior, or
  removes proven interoperability;
- required behavior can be preserved only by leaving two active lifecycle
  writers or routing paths in the release candidate;
- race tests cannot prove stale-generation rejection or exactly-once terminal
  behavior;
- observer failure still changes signaling after the planned migration;
- a slice exceeds existing beta performance thresholds or introduces retained
  resources after drain;
- evidence demonstrates that a retained foundation is fundamentally incapable
  of the required behavior rather than merely bypassed or duplicated.

Escalation must include the specific evidence, the retained component that is
insufficient, alternatives considered, API and standards effects, and the
smallest proposed scope change. Work must not silently expand into a rewrite.

## Explicit non-goals

This cleanup will not:

- rewrite SIP signaling or replace the existing architecture;
- create a new actor/mailbox concurrency framework;
- replace the runtime YAML model with build-time code generation;
- split the current system into new call, registration, subscription, message,
  or presence engines;
- create temporary sessions for standalone MESSAGE, OPTIONS, or SUBSCRIBE;
- build a new generic request engine;
- redesign, remove, rename, or version the supported public API;
- remove public event variants merely because newer detailed variants exist;
- narrow the configurable YAML grammar;
- move SIP transaction mechanics into the session state machine;
- move application lifecycle decisions into the dialog or transport layers;
- make the event bus part of the signaling hot path;
- add speculative PUBLISH support or claim that its fail-closed public facade
  is an implemented feature;
- expand RFC claims beyond executable evidence;
- remove protocol behavior solely because it is called legacy;
- introduce unrelated feature work, topology changes, or a new release profile;
- impose a new 24-hour beta requirement solely because cleanup was performed.

## Definition of done

Source inspection records the following implementation closure:

- one lane-owned working state and canonical transition commit are present;
- typed transition/response envelopes replace response and adapter prewrites;
- exact handles fence causal ingress, builders, trackers, timers, and cleanup;
- registration projection, snapshot merge compensation, debug-string routing,
  session-to-dialog response bus commands, and adapter mapping mirrors are
  removed;
- REGISTER and initial INVITE use one canonical internal implementation, and
  standalone MESSAGE/OPTIONS/SUBSCRIBE use one shared auth/transaction driver;
- causal dialog ingress is typed while event publication is observational; and
- all 24 audited YAML/Rust compatibility shapes have explicit retained owners.

The architecture cleanup is release-qualified only when all of the following
are true. Every item in this list is **PENDING** until the final clean-tree
Cargo/beta/performance/PBX/attestation run records it; the source findings above
must not be read as a PASS for these gates:

- Every state-changing SIP operation in the ownership matrix has one documented
  authority and one tested path.
- All exact-session mutations are serialized by the retained per-session lane,
  and all delayed inputs are generation-qualified.
- State-machine actions no longer rely on competing direct store writes,
  reconciliation projections, or selective snapshot preservation.
- Debug-string event parsing and duplicate causal bus routes are gone.
- Observer state has no effect on signaling correctness, latency-sensitive
  progression, or cleanup.
- Every dialog, Call-ID, media, session, timer, and transaction mapping has one
  canonical mutable owner.
- Every supported SIP method uses one request construction and authentication
  path for each intentionally distinct dialog context.
- The generated state-machine wiring manifest and runtime reachability tests
  agree.
- The supported public API and downstream compile fixtures are unchanged.
- Fast-response, cancellation, authentication, glare, timer,
  stale-generation, duplicate-final-response, and teardown races pass
  deterministically.
- Drain reports zero retained sessions, dialogs, media resources, receivers,
  timers, and transaction runners.
- No duplicate BYE, CANCEL, terminal event, or final response is observed.
- RFC wire tests and Asterisk, FreeSWITCH, SIPp, and strict-UA matrices pass for
  the retained feature profile.
- Canonical 2,000-CPS evidence, the complete performance matrix, monolithic and
  split soaks, PBX tests, and security gates meet the existing beta thresholds.
- The final report is produced from a clean tree and includes a verifiable
  source/binary/YAML/configuration/peer/artifact attestation.

Meeting these conditions produces the intended result: the existing
state-machine architecture becomes the single lifecycle source of truth,
dialog and transport code remain focused on standards-compliant SIP mechanics,
the event bus remains useful for reporting, and the system keeps its public API
and performance characteristics without another rewrite.
