# rvoip-core — Gap Plan

**Status:** Living document. Captures the delta between the four canonical
design docs in this directory (`PRD.md`, `INTERFACE_DESIGN.md`,
`CONVERSATION_PROTOCOL.md`, `voip-3-conversation-model.md`) and the code
currently in `src/`, plus a phased roadmap to close that delta.

**v1 surface closed:** 2026-05-26. All `[V1]` rows in §3.O and all
phases P1–P12 marked landed. Two `[V1.x]` rows in §3.O remain deferred
per the design docs (inline envelope signature enforcement at adapter
ingress, `rvoip-vcon-postgres` reference store).

## Implementation status

| Phase | Status | Notes |
|---|---|---|
| P1 — Vocabulary lifecycle | **landed** | Live Conversation/Session/Participant registries on Orchestrator; 7 lifecycle methods; auto-end on last-Connection-leave; 17 tests |
| P2 — mute/unmute/PlayAudio/BridgeTo | **landed** | Trait widened with `NotImplemented` defaults; PlaybackHandle + cancel; BridgeTo wired end-to-end |
| P3 — vCon emission | **landed** | DefaultVconBuilder + Session-bound builder + auto-emit `VconReady` on `SessionEnded`; real sha256 content_hash; consumer plugs `rvoip-vcon` signing via custom `VconStore` (cycle in dep graph prevents direct dep) |
| P4 — Messaging | **landed** | send_message_to_conversation fan-out across active legs, list_messages with pagination, mark_message_read, 64KB inline cap |
| P5 — Recording / Transcription / AI | **landed (full)** | `rvoip-harness` crate; 4 provider traits in core; attach_ai/start_recording/start_transcription dispatch with real frame pumps; pause/resume via shared AtomicBool; attach_listener tap (separated-streams default) with abort handle; barge-in via cancel-on-speech + `BargeInDetected` event |
| P6 — Multi-adapter + tenant + capacity | **landed (full)** | `OriginateRequest.transport: Option<Transport>` selector with all 15 call sites updated; per-tenant quota table (sessions / recordings / AI); CapacityReport scheduler |
| P7 — IdentityProvider + signing | **landed (full)** | All 3 missing trait methods (default NotImplemented); `signing` module with JCS + Signature-Input parser + replay cache; `UctpCoordinatorCaps::with_replay_protection` wires the cache into rvoip-uctp's `dispatch_inner` (after the in-reply-to gate so retransmits don't get rejected). P12.6 closed the step-up round-trip: `ConnectionAdapter::send_step_up_request` trait method, `AdapterEvent::StepUpResponse`, `Event::IdentityStepUpRequested`/`IdentityStepUpResponseReceived`, plus matching helper + envelope arm in `UctpCoordinator` and translation in all three substrate adapters (rvoip-quic / -webtransport / -websocket). Stub-adapter round-trip test passes ([tests/p12_step_up_roundtrip.rs](crates/foundation/rvoip-core/tests/p12_step_up_roundtrip.rs)). |
| P8 — Multi-party MP2/MP3c | **landed (full)** | `ActiveSpeakerChanged` event + `notify_active_speaker` helper in core; MP2 wire-side handler already lives in `rvoip-uctp::state::orchestrator_handler`; MP3c `allocate_subscriber_stream` already lives in `rvoip-quic` and `rvoip-webtransport` adapters |
| P9 — Observability | **landed (full)** | `SessionQualityReport` payload on `SessionEnded`, populated by per-Session `QualityAggregator` driven by `AdapterEvent::Quality`; `spawn_media_quality_sampler` periodic emitter; Prometheus global gauges in `capacity_report` (active_connections / bridges / sessions / conversations / recordings / ai_attachments / jitter / loss); `record_usage` + registration normalization helpers. P12.7 closed the OTel gap: optional `otel` feature on `infra-common` wires an OTLP exporter into `logging::setup` when `LoggingConfig.otel_endpoint` is set; `#[tracing::instrument]` decorations on 8 Orchestrator entry points (open / close_conversation, start / end_session, originate / transfer_connection, attach_ai, bridge_connections) produce the named span hierarchy from PRD §10.2. Test [tests/p12_otel_spans.rs](crates/foundation/rvoip-core/tests/p12_otel_spans.rs) captures spans through a custom subscriber. |
| P10 — Closure policy + filters | **landed** | `spawn_idle_closer` driver task; `ConversationFilter` + widened `ConversationStore::list` with default impl falling back to `list_for_tenant` |
| P11 — Feature flags + workspace | **landed (full)** | `[features]` section per INTERFACE_DESIGN §2.2; `vcon-signing` informational (cycle-blocked); `identity-fingerprint-binding` informational (variant stays compiled because rvoip-auth-core matches on it); `harness` informational (rvoip-harness already provides the seam); `rvoip-identity` skeleton crate landed with `BearerProvider`; `rvoip-harness` already landed in P5. P12.3 closed the `rvoip-client` gap: new crate at [crates/rvoip-client](crates/rvoip-client) with `Client` / `SessionHandle` / `InboundEvent` / `Credential` / `CallTarget` surface; feature-gated per-protocol re-exports (`rvoip_client::sip`/`webrtc`/`uctp`); facade-exposed via `rvoip::client::*` when `client` feature is enabled. |

**Tests:** 73 passing across 17 test binaries (was 27 across 5 pre-P1).
**Downstream:** rvoip-sip / rvoip-webrtc / rvoip-quic / rvoip-uctp / rvoip-vcon / rvoip-harness / rvoip-identity / rvoip-webtransport / rvoip-websocket / rvoip-sip-registrar all compile clean.

## Closed in the post-v1 correctness round

- **Per-tenant quota TOCTOU** ✅ Closed — `start_recording` / `attach_ai` now use the DashMap entry's bucket lock to make check-and-increment atomic per tenant.
- **`renegotiate_media` `block_in_place`** ✅ Closed — refactored to direct `.await` on the peer's stream lookup; the DashMap iterator guard is dropped before any await.
- **Bridge transcoder hot-swap ack** ✅ Closed — `TranscoderSwap` carries an optional `ack: oneshot::Sender<()>` that the pump fires after applying the swap; `swap_transcoders` wires + awaits both per-direction acks (1s timeout).
- **rvoip-sip `deferred_guard_wait_for_cancelled_*` failures** ✅ Closed — tests were publishing to the cross-test singleton instead of the test's own `UnifiedCoordinator.global_coordinator`; fixed via a `publish_via(coord, event)` helper.
- **Signing spec validation** ✅ Closed — 10 RFC 8785 JCS vectors landed (empty obj/array, key sort, nested sort, array order, escapes, UTF-8, integers, bools/null, realistic envelope), 4 FIPS-180-4 / RFC 9421 vectors for sha256 + Signature-Input. JCS number-canonicalization out-of-scope per the doc-comment (floats are not JCS-strict).
- **Replay-cache TTL eviction + idle-timer driver** ✅ Closed — explicit edge-case tests in `signing::tests` (TTL elapses → re-insert succeeds) and `tests/p10_idle_closer.rs` (Ephemeral closes after TTL, Persistent stays Open).

## Remaining v1.x follow-ups

All three v1.x deferred items closed in the v2.A + v2.B round — see
the V2 plan section below for what landed and the V2.B test that
proves the scaling fix.

## v2 Architectural Plan — STATUS: LANDED ✅

Both V2.A and V2.B shipped in the post-v1 architectural round.
Brief status follows; details below.

| Item | Status | Key fact |
|---|---|---|
| **V2.A — Carve `rvoip-core-traits`** | ✅ Landed | New sibling crate holds `ids`, `identity` data, `capability`, `connection`, `error`, `stream`, `harness` modules. `rvoip-core` re-exports — call sites unchanged. `auth-core` switched dep. `rvoip-vcon` dropped unused dep. Cycle broken. V2.A.10 `cargo deny` rule landed in P12.5 (`deny.toml` + `.github/workflows/cargo-deny.yml`); rule freezes the current adapter set and prevents new direct-`rvoip-core` deps. Adapter migration to `rvoip-core-traits` (V2.A.7 follow-up) is a separate ratchet — current state is grandfathered via the `wrappers` allow-list. |
| **V2.A.8 — `vcon-signing` real Cargo dep** | ✅ Landed | `rvoip-vcon = { workspace = true, optional = true }` + `vcon-signing = ["dep:rvoip-vcon"]`. Re-exported as `rvoip_core::signed_vcon::*` when feature enabled. |
| **V2.A.9 — `harness` real Cargo dep** | ✅ Landed | `rvoip-harness = { workspace = true, optional = true }` + `harness = ["dep:rvoip-harness"]`. |
| **V2.B — Per-tenant Semaphore admission** | ✅ Landed | `recording_sems` / `ai_sems` `DashMap<TenantId, Arc<Semaphore>>` replaces the counter DashMaps. `try_acquire_owned()` returns the permit; permit lives in handle; Drop releases. Resize-up via `add_permits`; resize-down with held permits returns `InvalidState`. |

**Tests:** rvoip-core 19 test binaries pass; 6 sibling crates 39 binaries pass; 130/130 rvoip-sip lib tests pass.

## v2 Architectural Plan (proposals for the deferred items)

### V2.A — Break the dep cycle via `rvoip-core-traits`

**Goal.** Let `rvoip-core` depend on `rvoip-vcon` and `rvoip-harness` as
real optional Cargo deps (closing items 1 + 2 above), without growing
their bloat into every consumer of `rvoip-core`.

**Root cause.** The cycle is structural: every adapter / store crate
needs the *types* `rvoip-core` defines (`ConnectionId`,
`SessionId`, `IdentityAssurance`, `IdentityProvider`, `Result`,
`RvoipError`, …), but `rvoip-core` itself owns implementation logic
(`Orchestrator`, bridging, lifecycle) that those crates *also* want
to plug into. Right now both live in one crate, so any crate that
imports a type is forced to pull the implementation too.

**Plan.** Carve `rvoip-core-traits` as a new sibling crate holding
*only* the type/trait surface:
- `ids` module (all newtype IDs)
- `error` module (`Result`, `RvoipError`)
- `identity` module (`Identity`, `IdentityAssurance`, `Credential`,
  `IdentityProvider` trait — leave production backends in `rvoip-identity`)
- `capability` module (`CapabilityDescriptor` + negotiation algorithm)
- `connection` / `participant` / `conversation` / `session` /
  `message` / `stream` data structs
- `events` enum (the cross-crate event vocabulary)
- `commands` enum
- `harness` module (the 4 provider traits)
- `vcon` module (`VconBuilderHandle` trait + supporting types)
- `signing` module (JCS + Signature-Input parser + ReplayCache)
- `store` module (the 3 store *traits*; in-memory impls stay in
  `rvoip-core`)

`rvoip-core` keeps:
- `Orchestrator` (the live registries, event bus, dispatch logic)
- `bridge` (frame pump, codec map, cross-bridge handle)
- `subscriptions` (the multi-party routing table)
- The default `MemoryConversationStore` / `MemoryVconStore` / `MemoryMessageStore` impls
- The `DefaultVconBuilder` impl

**New dep graph (no cycle):**

```
rvoip-core-traits         (zero rvoip deps)
   ↑      ↑      ↑     ↑
   │      │      │     │
   │   rvoip-auth-core   │
   │      ↑              │
   │      │              │
   │   rvoip-vcon        │
   │      ↑              │
   │   rvoip-harness     │
   │   rvoip-identity    │
   │                     │
   └─── rvoip-core ──────┘   (depends on rvoip-core-traits +
        ↑                     optionally on rvoip-vcon / rvoip-harness)
        │
    adapters: rvoip-sip / rvoip-webrtc / rvoip-quic / rvoip-uctp /
              rvoip-webtransport / rvoip-websocket
              (depend on rvoip-core-traits + rvoip-core)
```

**Tasks (phased to minimize churn):**

| # | Task | Touches |
|---|---|---|
| V2.A.1 | Create `crates/foundation/rvoip-core-traits/` skeleton with Cargo.toml + empty modules. | new dir |
| V2.A.2 | Move `ids.rs`, `error.rs`, `capability.rs` (pure types), `identity.rs` (trait + types, NOT production impls) into rvoip-core-traits. | move |
| V2.A.3 | Re-export from `rvoip-core` so existing call sites keep `use rvoip_core::ConnectionId`. Aim for zero-churn for downstream. | `rvoip-core/src/lib.rs` |
| V2.A.4 | Move `harness`, `vcon`, `signing` modules to rvoip-core-traits. Re-export from rvoip-core. | move |
| V2.A.5 | Move `conversation`, `session`, `participant`, `message`, `stream`, `connection`, `events`, `commands` data shapes to rvoip-core-traits (orchestrator state stays in rvoip-core). | move |
| V2.A.6 | Move the store *traits* (not impls) into rvoip-core-traits; impls stay in rvoip-core. | split |
| V2.A.7 | Switch `rvoip-auth-core`, `rvoip-vcon`, `rvoip-harness`, `rvoip-identity`, all 6 adapters' Cargo.toml from `rvoip-core` to `rvoip-core-traits` where they only use types. | every adapter Cargo.toml |
| V2.A.8 | Add `rvoip-vcon = { workspace = true, optional = true }` to rvoip-core; activate behind `vcon-signing` feature. Wire `rvoip_vcon::sign_jws` into a default `VconStore` impl gated on the feature. | rvoip-core |
| V2.A.9 | Add `rvoip-harness = { workspace = true, optional = true }`; gate the no-op provider re-exports behind `harness` feature. | rvoip-core |
| V2.A.10 | Workspace cargo-deny rule that fails CI if any adapter crate adds `rvoip-core` as a direct dep (must use `rvoip-core-traits`). | `deny.toml` |
| V2.A.11 | Migration test: build the workspace with default features (signed vCons off), `--features full` (everything on), `--no-default-features` (minimal). | CI |

**Effort.** 2–3 focused days. The move is mechanical but every adapter
crate's `use rvoip_core::…` import lines need to be retargeted (the
re-exports in step V2.A.3 minimize this, but the explicit Cargo.toml
dep swap is per-crate).

**Risk + mitigation.**
- *Risk:* a downstream crate breaks because it depended on a type
  that moved without a re-export. *Mitigation:* re-export everything
  from rvoip-core verbatim until adapters migrate; deprecate the
  re-export in a future version with a 1-release notice.
- *Risk:* cargo-deny rule generates noise during the migration.
  *Mitigation:* land the rule in V2.A.10 only after V2.A.7.
- *Risk:* macro expansion (the `id_type!` macro) doesn't survive the
  move. *Mitigation:* the macro is fully self-contained, no
  external paths needed.

**Acceptance.**
1. `cargo build -p rvoip-core --features vcon-signing` includes
   rvoip-vcon directly, no consumer-supplied glue needed.
2. `cargo build -p rvoip-core --features harness` enables the
   orchestrator's AI/recording methods and brings rvoip-harness's
   no-op providers into scope.
3. `cargo build -p rvoip-core --no-default-features` produces a
   minimal binary surface (no harness methods, no vCon signing).
4. `cargo deny check` passes (no adapter has a direct `rvoip-core`
   dep).
5. All existing tests still pass.

### V2.B — Per-tenant `Semaphore` for true admission control

**Goal.** Replace the DashMap-entry-locked check-and-increment in
`start_recording` and `attach_ai` with per-tenant `Arc<Semaphore>`
so admission is shard-free and contention is bounded.

**Current state.** The v1 atomic-check pattern (`entry().or_insert()`
+ check + bump under the bucket guard) is *correct* but the DashMap
sharded RwLock means concurrent admissions for the same tenant
serialize through one shard's bucket lock. For low qps this is
fine; for >10k qps per tenant the contention starves throughput.

**Plan.**

| # | Task | Approach |
|---|---|---|
| V2.B.1 | Add `recording_sems: DashMap<TenantId, Arc<Semaphore>>` and `ai_sems: DashMap<TenantId, Arc<Semaphore>>` to `Orchestrator`. | struct fields + init |
| V2.B.2 | `set_tenant_quotas` creates / replaces the per-tenant `Semaphore` with the configured cap (resize-up only — resize-down is a separate problem, see "Risk" below). | `set_tenant_quotas` |
| V2.B.3 | `start_recording`: when a tenant has a recording semaphore, call `try_acquire_owned()`. Store the `OwnedSemaphorePermit` in `RecordingHandle`. Permit released by Drop on the handle (i.e. on `stop_recording`). | dispatch + handle |
| V2.B.4 | Same for `attach_ai` + `AiAttachmentHandle`. | dispatch + handle |
| V2.B.5 | Remove the `tenant_active_recordings` / `tenant_active_ai` usize counters once the permits replace them (or keep them for capacity reporting, re-derived from `Semaphore::available_permits`). | cleanup |
| V2.B.6 | Concurrency test: spawn N parallel `start_recording` calls under quota = M < N, assert exactly M succeed and (N − M) fail with `AdmissionRejected`. | new test |

**Effort.** Half-day. Self-contained in orchestrator.rs + 1 test file.

**Risk + mitigation.**
- *Risk:* quota resize-down with active permits has semantic
  ambiguity (do you kill existing recordings? wait for them?
  reject new ones?). *Mitigation:* v2.B only supports resize-up;
  resize-down panics with a `NotImplemented` error and is tracked
  separately as v2.B.1.
- *Risk:* `try_acquire_owned` returns `TryAcquireError::NoPermits`
  immediately under contention even when permits are about to be
  released. *Mitigation:* this is the intended behavior — callers
  that want to wait should use `acquire_owned().await` instead;
  expose both via a `quota_wait: bool` field on the start request.
- *Risk:* the permit's Drop runs on the task ending the recording,
  which may be the abort path — verify the permit isn't leaked when
  the task aborts mid-recording. *Mitigation:* `OwnedSemaphorePermit`
  is `Send` and dropped when the handle struct drops, regardless of
  abort path.

**Acceptance.**
1. Under quota = 5 and 20 concurrent `start_recording` calls, exactly
   5 succeed and 15 return `AdmissionRejected`.
2. After all 5 call `stop_recording`, a new `start_recording` succeeds.
3. `available_permits()` snapshot matches `quota - active` at all
   times.

### V2.C — Suggested ordering + cumulative cost

| Order | Reason |
|---|---|
| V2.B first | Independent, half-day, gives an immediate scaling win. No structural risk. |
| V2.A second | Larger structural change. Best done in isolation so the diff is purely "move types around" without interleaved logic changes. |

**Combined effort estimate:** ~3 days focused work, ~5 days
calendar-time accounting for adapter-crate review cycles.

**Combined outcome:** all three deferred items closed; rvoip-core
becomes feature-flag-gated for real; vCon signing wires directly
without consumer-side glue; per-tenant admission scales linearly.

**Labels:**
- `[V1]` — must ship for v1 (a stated v1 deliverable in the docs).
- `[V1.x]` — planned post-v1; called out by the docs as next-up.
- `[V2]` — explicitly deferred in the docs (SFU multi-party, RoQ/MoQ, etc.).

---

## 1. Purpose & method

### Purpose

This crate is intended to be the **transport-agnostic spine** for RVoIP —
the place where voip-3 nouns become live objects, where adapters
(SIP/WebRTC/UCTP-family) register against a single `Orchestrator`, where
cross-substrate bridging and multi-party routing live, and where the
crate-wide event/command vocabulary is defined. It is the upper edge of
what protocol-specific consumers see; downstream consumers (`Thelve`,
CPaaS products, AI runtimes) build on top of it.

The four design docs together specify:
- PRD — product role, command/event catalog, billing/usage surface.
- INTERFACE_DESIGN — Rust trait shapes, crate layout, feature flags.
- CONVERSATION_PROTOCOL — UCTP wire envelopes, substrate framing,
  capability negotiation algorithm.
- voip-3 — the conceptual model (Conversation/Session/Connection/Stream/
  Participant/Message).

### Method (how this doc was produced)

Three parallel inventories:
1. Spec extraction across all four docs → a consolidated "spec requires"
   checklist of every required type, trait method, event, command,
   behavior, and invariant.
2. Implementation map of every public item in `src/`, including stubs,
   `NotImplemented` markers, and `// not yet` comments.
3. Test/example audit of the 27 tests across `tests/` and the single
   `examples/` binary.

Each gap below is grounded in a specific file/line in `src/` (or its
absence) and a specific section of one or more design docs.

---

## 2. Current state snapshot

### Solid (built and exercised)

| Area | Status | Where |
|---|---|---|
| voip-3 vocabulary structs | Data shapes complete | `conversation.rs`, `session.rs`, `connection.rs`, `participant.rs`, `message.rs`, `identity.rs`, `stream.rs` |
| Command catalog | 25 variants, full per PRD §10 | `commands.rs` |
| Event catalog | ~40 variants | `events.rs` |
| `ConnectionAdapter` trait | 16 async methods | `adapter.rs` |
| `IdentityProvider` trait | 6 methods (3 short of spec) | `identity.rs:146–154` |
| `IdentityAssurance` gradient | 5 standard variants + `DtlsFingerprint` | `identity.rs:50–81` |
| `CapabilityDescriptor` + §8.1 negotiation | Full wire shape + algorithm | `capability.rs:134–402` |
| `VconBuilderHandle` trait + supporting types | Trait + types defined | `vcon.rs` |
| `ConversationStore` / `VconStore` | Traits + in-memory impls | `store/*.rs` |
| Cross-transport bridging | Full frame-pump + hot-swap + DTMF auto-route | `orchestrator.rs:800–934`, `bridge/frame_pump.rs` |
| Multi-party MP1 + MP3a | Subscription table + fanout dispatch | `subscriptions.rs`, `orchestrator.rs:324–404` |
| Adapter event normalization | Per-adapter loop, optional cross-crate publish | `orchestrator.rs:136–154`, `441–585` |

### Stubs and holes flagged in code

| What | Where | Returns |
|---|---|---|
| `Orchestrator::mute` / `unmute` | `orchestrator.rs:761–779` | `NotImplemented` |
| `InboundAction::BridgeTo` dispatch | `orchestrator.rs:600` | `NotImplemented` |
| `ConnectionAdapter::allocate_subscriber_stream` default | `adapter.rs:167–176` | `NotImplemented` |
| `originate_connection` adapter selection | `orchestrator.rs:606–622` | First-registered adapter (no transport selector) |
| `VconStore::put` | `store/vcon_store.rs` | Mock handle, no signing |
| `VconStore::list_for_session` | `store/vcon_store.rs` | Always `vec![]` |
| `VconRef` population | `vcon.rs:23–30` | Always `None` per source comment |

### Tests in place (27 across 5 files)

- `bridge_pump.rs` (5) — frame passthrough, self-bridge reject,
  unknown-connection reject, already-bridged reject, unbridge teardown.
- `dtmf_auto_route.rs` (2) — cross-bridge DTMF forwarding, no-forward when
  unbridged.
- `fanout_frame.rs` (7) — multi-subscriber delivery, cross-session
  isolation, empty fanout, kind-mismatch skip, unsubscribe halts, A3 auth
  event regression, A2 publisher registry cleanup.
- `orchestrator_dispatch.rs` (3) — registration + dispatch smoke, missing
  adapter error, duplicate registration reject.
- `subscriptions.rs` (9) — add/remove/query/idempotency/isolation/drop-
  session.

No `#[ignore]` or stub tests.

### Workspace dependents (validate the seam)

`rvoip-core` is depended on by `rvoip-sip`, `rvoip-webrtc`, `rvoip-quic`,
`rvoip-uctp`, `rvoip-websocket`, `rvoip-webtransport`, `orchestration-core`,
`rvoip-vcon`. All adapter crates implement `ConnectionAdapter` and register
against a shared `Orchestrator`.

---

## 3. Gap inventory

### A. Vocabulary lifecycle behavior `[V1]` — BLOCKER

**What's missing.** `Conversation`, `Session`, `Participant` are pure data
structs. No constructors that drive state transitions, no enforcement of
the state-machine (`Initiating → Active → Ending → Ended`/`Failed`), no
methods to add/remove participants or connections at runtime. The 7
lifecycle Commands (`OpenConversation`, `CloseConversation`,
`StartSession`, `EndSession`, `JoinSession`, `LeaveSession`,
`RouteInboundConnection`/Accept→Session) have no Orchestrator methods.

**Where in code.** `conversation.rs`, `session.rs`, `participant.rs` (all
data-only). `orchestrator.rs` has 25 public methods but none of them open
a Conversation, start a Session, or move a participant.

**Spec source.** PRD §10 (command catalog), INTERFACE_DESIGN §3
(type definitions with state machines), CONVERSATION_PROTOCOL §7
(session lifecycle), voip-3 §6 (containment & lifecycle).

**Why it blocks the rest.** vCon emission (B) needs Sessions as live
objects. Messaging (C) needs Conversations. Recording (D) needs Sessions
to attach to. Closure policy enforcement (K) needs Conversation lifecycle
events. Capacity quotas (G) need to count active Sessions per tenant.

### B. vCon emission as a load-bearing path `[V1]` — BLOCKER

**What's missing.**
- No `Session::vcon_handle()` accessor (INTERFACE_DESIGN §3.9).
- No default `VconBuilderHandle` implementor bound to a live Session.
- No auto-collection of parties from `ParticipantJoined`, dialogs from
  `connection.ready` / `connection.update`, analyses from
  `TranscriptTurn`, attachments from SIP signaling / STIR-SHAKEN.
- No auto-emit of `Event::VconReady` on `SessionEnded`.
- `VconStore::put` is a mock; no signing, no JWE, no fingerprint hash.
- `VconRef` is always `None` per the source comment.

**Spec source.** PRD §7 ("vCon ALWAYS emitted per Session"),
CONVERSATION_PROTOCOL §7.6 (`recording.vcon-ready` envelope),
INTERFACE_DESIGN §3.9 (builder shape) + §11.4 (`VconStore` trait).

### C. Messaging operations `[V1]`

**What's missing.**
- `Orchestrator::send_message` operates on a single Connection. No
  Conversation-level fan-out: same logical message to SIP MESSAGE +
  WebRTC DataChannel + UCTP envelope.
- No `list_messages(conversation_id, filter, page_cursor)`.
- No `mark_message_read(message_id)`.
- `MessageDelivered` and `MessageRead` event variants exist but are
  never emitted.
- No attachment policy (inline ≤64KB vs OOB URL per PRD §10.4).

**Spec source.** PRD §10.4 (messaging commands), INTERFACE_DESIGN §10
(send/recv/history), CONVERSATION_PROTOCOL §9.

### D. Recording & Transcription `[V1]`

**What's missing.**
- No `Orchestrator::start_recording` / `stop_recording` / `pause` /
  `resume` methods (commands exist; dispatcher doesn't).
- No `RecordingSink` trait beyond the `enum RecordingSink { File, Url }`
  shape in commands.rs — that's a sink *descriptor*, not a sink trait
  that produces bytes.
- No `AsrProvider` / `TtsProvider` / `DialogManager` traits in code; PRD
  §6 + INTERFACE_DESIGN §2.1 place them in `rvoip-harness`, a crate that
  does not yet exist in the workspace.
- `RecordingId` newtype exists (`ids.rs`) but is never allocated by the
  orchestrator.
- No dual-ASR plumbing: spec requires separate ASR for dialog vs
  transcription when in-process AI is active.

**Spec source.** PRD §6 (recording + transcription), INTERFACE_DESIGN
§2.1 (harness crate split), PRD §10.5 (recording commands).

### E. AI harness wiring `[V1]`

**What's missing.**
- No `Orchestrator::attach_ai` / `attach_listener` / `detach` methods.
- No tap topology builder (separated streams default, mixed on request).
- No barge-in primitive (cancel-TTS-on-speech), no filler/backchannel
  hook.
- No way for a `DialogManager` to assert "stop current TTS" or "play this
  next" against a Connection.

**Spec source.** PRD §6 (AI harness), INTERFACE_DESIGN §6 (adapter +
harness interplay).

### F. Per-connection media control gaps `[V1]`

**What's missing.**
- `Orchestrator::mute` and `unmute` return `NotImplemented`
  (`orchestrator.rs:761–779`). `ConnectionAdapter` has no `mute`/`unmute`
  methods at all.
- `PlayAudio` command exists; no `Orchestrator::play_audio`, no adapter
  trait method, no cancellation handle.
- `InboundAction::BridgeTo` returns `NotImplemented`
  (`orchestrator.rs:600`); inbound-gateway pattern (accept + bridge to
  outbound in one go) is unbuilt.

**Spec source.** PRD §10.3 + §10.6, INTERFACE_DESIGN §6.

### G. Multi-adapter dispatch, tenant scoping, capacity `[V1]`

**What's missing.**
- `originate_connection` picks the first registered adapter; once two
  adapters register for different transports this is wrong. Need a
  `transport: Transport` field on `OriginateRequest` (and a route-by-URI
  fallback for SIP/WebRTC URIs).
- `TenantId` is carried in every Command but the Orchestrator never
  reads it. All Conversation/Session/Connection lookups are global. Two
  tenants can step on each other's IDs.
- No per-tenant quota layer: PRD §11 specifies max concurrent calls,
  recordings, AI sessions per tenant. Only a process-wide admission
  semaphore exists.
- `CapacityReport` event variant exists; nothing emits it on schedule.

**Spec source.** PRD §11 (capacity & quotas), INTERFACE_DESIGN §6
(originate API).

### H. Identity provider completion + per-request signing `[V1]` trait, `[V1.x]` production impl

**What's missing.**
- `IdentityProvider` trait is 6 methods; INTERFACE_DESIGN §8 specifies 9.
  Missing: `register_agent_key(IdentityId, Jwk)`,
  `verify_signature(IdentityId, SignatureHeaders, &[u8])`,
  `derive_dtls_fingerprint(IdentityId)`.
- `ConnectionAdapter::verify_request_signature` exists but no
  canonicalization helpers — RFC 9421 Signature/Signature-Input parsing,
  JSON Canonical Form (RFC 8785) for non-HTTP envelopes, Hardt
  `Signature-Key` / `Signature-Agent` headers.
- Step-up auth flow (`identity.step-up-request` /
  `identity.step-up-response` per CONVERSATION_PROTOCOL §6) has no
  orchestrator method, no event, no envelope handler. The
  `IdentityAssuranceChanged` event exists but nothing drives it.
- No replay-protection cache for envelope IDs (spec calls for 5 min
  default per CONVERSATION_PROTOCOL §5.5).

**Spec source.** INTERFACE_DESIGN §8 (provider trait + RFC 9421),
CONVERSATION_PROTOCOL §5.5 + §5.6 (signing + assurance gradient).

### I. Multi-party MP2 + MP3c `[V1]`

**What's missing.**
- MP1 (subscription table) and MP3a (fanout dispatch) are done.
- MP2: UCTP wire-side coordinator handler for `stream.subscribe` /
  `stream.unsubscribe` envelopes that calls
  `Orchestrator::add_subscription` / `remove_subscription`. Deferred per
  source comments in `subscriptions.rs`.
- MP3c: `allocate_subscriber_stream` default returns `NotImplemented`;
  UCTP-family adapters (rvoip-quic, rvoip-webtransport) need real
  implementations that allocate per-subscriber egress streams.
- No active-speaker advisory (CONVERSATION_PROTOCOL §6 `stream.active-
  speaker` envelope).

**Spec source.** CONVERSATION_PROTOCOL §7.7 (multi-party routing) + §6
(envelope catalog), INTERFACE_DESIGN §10 (multi-party model).

### J. Observability surface `[V1]`

**What's missing.**
- `SessionEnded { session_id, at }` (`events.rs:42–45`) is missing the
  `SessionQualityReport` payload required by PRD §10.2 (MOS, packet
  loss, jitter, RTT, codec, bitrate, talk%, silence%, PDD, ring time,
  setup time, hangup reason).
- `TranscriptTurn` event — spec requires per-turn ASR events with
  stream_id, speaker, text, confidence, is_final, assigned provider.
  Confirm presence in `events.rs` (partial read showed up to line 120);
  add if absent.
- `UsageRecord` event variant exists; no aggregation pipeline,
  no per-session counter, no per-tenant rollup.
- `RegistrationChanged` / `RegistrationHeartbeat` exist; no
  normalization layer that consumes events from a registrar (`rvoip-sip-
  registrar`) and re-emits them in core's vocabulary.
- `MediaQuality` event exists; no periodic sampling cadence specified or
  driven.
- OpenTelemetry tracing not threaded through. Prometheus metrics
  partial: three `_total` counters present; the global gauges PRD §11
  calls for (active calls, calls/sec, admission rejects, per-tenant
  gauges, provider error rates, runtime memory) absent.

**Spec source.** PRD §10 (event catalog) + §11 (observability),
INTERFACE_DESIGN §5.

### K. Conversation persistence & closure policy `[V1]`

**What's missing.**
- `MemoryConversationStore` is fine for dev, but the closure-policy
  enforcer is absent: `ConversationPolicy::Ephemeral { idle_close_secs }`
  is a data field with no timer driving it. Per PRD §11, Ephemeral
  Conversations close N seconds after the last Session ends + no new
  Messages arrive.
- The `ConversationStore::list_for_tenant` filter is the only filter.
  PRD §10 mentions filtering by Participant / Identity / state / time
  range.

**Spec source.** PRD §11 (closure policy), INTERFACE_DESIGN §11.1 (store
filter shape).

### L. Feature flags & workspace layout `[V1]` for flags; `[V1]` decision on crate spin-offs

**What's missing.**
- `Cargo.toml` has no `[features]` section at all. INTERFACE_DESIGN §2.2
  specifies `default = [uctp, sip, rtp, media, vcon, identity]` plus
  optional `webrtc`, `aauth-experimental`,
  `identity-fingerprint-binding`, `harness`, `client`, `full`.
- Sibling crates expected by §2.1 but absent: `rvoip-identity`,
  `rvoip-harness`, `rvoip-client`.
- `rvoip-vcon` exists as a crate but `MemoryVconStore` doesn't bridge
  to its signing/encryption.

**Spec source.** INTERFACE_DESIGN §2.1 (crate layout) + §2.2 (features).

### M. Test coverage gaps `[V1]`

Today's 27 tests are deep on bridging, fanout, subscriptions, dispatch.
**No tests for:**
- Conversation lifecycle (open/close, Ephemeral idle close,
  ConversationOpened/Closed event emission).
- Session lifecycle (start/end/join/leave, ParticipantJoined/Left
  emission, state transitions).
- vCon emission (VconReady fires on SessionEnded, snapshot contains
  expected parties/dialogs).
- Messaging end-to-end (send to Conversation, history pagination,
  cross-substrate fan-out, MessageDelivered/Read).
- `negotiate_streams` algorithm (`capability.rs:363`) — currently no
  direct test of the §8.1 logic.
- Identity assurance transitions (step-up request/response flow,
  IdentityAssuranceChanged emission).
- `CapabilityDescriptor` JSON round-trip against the full §8 wire shape.
- Multi-adapter originate dispatch by transport.
- Tenant isolation (two tenants can't see each other's Conversations).

### N. Explicitly deferred items

| Item | Label | Source |
|---|---|---|
| `rvoip-websocket` substrate | `[V1.x]` | INTERFACE_DESIGN §2.4 |
| AAuth production (gated `aauth-experimental`) | `[V1.x]` | INTERFACE_DESIGN §2.4 + §8.5 |
| RFC 9421 default-on per-request signing | `[V1.x]` | INTERFACE_DESIGN §2.4 |
| DTLS-SRTP fingerprint binding default-on | `[V1.x]` | INTERFACE_DESIGN §8.4 |
| `conversation.update` for policy change | `[V1.x]` | CONVERSATION_PROTOCOL §7.1 |
| Multi-party UCTP beyond N=2 via SFU | `[V2]` | PRD §5 + INTERFACE_DESIGN §2.4 |
| SIP-over-QUIC / RoQ / MoQ adapters | `[V2]` | INTERFACE_DESIGN §2.5 |

---

## 3.O — Cross-doc items not previously tracked here

Audit pass (2026-05-26) against the four design docs surfaced
commitments that no earlier phase or status row referenced. Each row
below is grounded in an INTERFACE_DESIGN / PRD / CONVERSATION_PROTOCOL
section and a confirmed absence (or thin presence) in the workspace
as of this writing. The `[V1]` items are addressed by the new P12
phase below; `[V1.x]` items are tracked here for visibility but
remain deferred per the design docs.

| # | Item | Label | Status | Spec source | Closer |
|---|---|---|---|---|---|
| 3.O.1 | `session-core` absorbed into `rvoip-sip` | `[V1]` | ✅ done — no `session-core` crate exists; rename to `rvoip-sip` already in place. Residual doc / re-export artifacts in the unbuilt `crates/rvoip/` facade are cleaned up by P12.2. | INTERFACE_DESIGN §13.1; PRD §13 | P12.1 |
| 3.O.2 | `orchestration-core` deleted / renamed to `rvoip` facade | `[V1]` | ✅ done — `crates/orchestration-core/` deleted (all examples were workforce-shaped, lifted to consumers per PRD §5). `crates/rvoip/` is now a real workspace member: `src/lib.rs` re-exports `Orchestrator`/`Config` from `rvoip-core`, voip-3 nouns from `rvoip-core-traits`, and feature-gated modules `sip`/`webrtc`/`uctp`/`harness`/`vcon`/`identity`/`client`. `Cargo.toml` has the §2.2 `[features]` table (`default = [uctp, sip, vcon, identity]` + `webrtc` / `harness` / `client` / `aauth-experimental` / `identity-fingerprint-binding` / `full`). `cargo build -p rvoip --no-default-features`, `cargo build -p rvoip` (default), `cargo build -p rvoip --features full`, and `cargo build --workspace` all clean. | INTERFACE_DESIGN §13.3 step 7; PRD §13 | P12.2 |
| 3.O.3 | `rvoip-client` crate (`Client`, `SessionHandle`, `InboundEvent`) | `[V1]` | ✅ done — scaffolded crate at [crates/rvoip-client](crates/rvoip-client) with v1 public surface (`Client::connect` substrate dispatch by URL scheme; `SessionHandle` with accept/reject/end/hold/resume/mute/send_dtmf method stubs; `InboundEvent::{IncomingSession, Message, AssuranceChanged, Disconnected}`; per-protocol re-exports via feature flags). Per-substrate dial wiring (`Client::call` actually sending an INVITE / `session.invite` envelope) is left for follow-up as consumers exercise it — surface is stable. 4 tests passing; deny.toml clean (`rvoip-client` depends on `rvoip-core-traits` only, not `rvoip-core`). | INTERFACE_DESIGN §2.1, §15 | P12.3 |
| 3.O.4 | Hello-world sketches §16.2 / §16.3 / §16.4 | `[V1]` | ✅ done — three new sketches at [crates/rvoip/examples/sip_webrtc_bridge.rs](crates/rvoip/examples/sip_webrtc_bridge.rs) (§16.2, features `sip,webrtc`), [uctp_only_server.rs](crates/rvoip/examples/uctp_only_server.rs) (§16.3, features `uctp,vcon,identity`), [full_thelve_shape.rs](crates/rvoip/examples/full_thelve_shape.rs) (§16.4, features `full`). Each compiles under its declared feature set and runs an event-pump loop reaching `main()` against a stub Orchestrator. (§16.1 `sip_only_orchestrator.rs` from before remains.) | INTERFACE_DESIGN §16 | P12.4 |
| 3.O.5 | `cargo deny` rule preventing adapter→`rvoip-core` direct dep | `[V1]` | ✅ done — `deny.toml` + `.github/workflows/cargo-deny.yml` landed (2026-05-26). Rule freezes current adapter dep set (rvoip-sip / -webrtc / -quic / -webtransport / -websocket / -uctp / -identity grandfathered via `wrappers` allow-list); new adapter crates default-banned from direct `rvoip-core` deps. | GAP_PLAN V2.A.10 | P12.5 |
| 3.O.6 | Step-up auth envelope round-trip in adapters | `[V1]` | ✅ done — `request_step_up` now dispatches into the adapter and emits `Event::IdentityStepUpRequested`; peer's response surfaces as `AdapterEvent::StepUpResponse` → `Event::IdentityStepUpResponseReceived`; consumer calls `complete_step_up` → `IdentityAssuranceChanged`. UctpCoordinator handles inbound `identity.step-up-response` and exposes `send_step_up_request` helper. All three UCTP substrate adapters forward the event. 3 stub-adapter tests passing. | CONVERSATION_PROTOCOL §5.8 | P12.6 |
| 3.O.7 | OpenTelemetry span hierarchy (Session / Connection / AI turn / transfer) | `[V1]` | ✅ done — `infra-common` exposes `otel` feature wiring OTLP exporter into `logging::setup` (`LoggingConfig.otel_endpoint`); 8 Orchestrator entry points decorated with `#[tracing::instrument]` (open / close_conversation, start / end_session, originate / transfer_connection, attach_ai, bridge_connections). Span hierarchy verified by `tests/p12_otel_spans.rs`. | INTERFACE_DESIGN §5; PRD §10.2 | P12.7 |
| 3.O.8 | DTMF & `connection.quality` wired through `ConnectionAdapter` end-to-end | `[V1]` | ✅ done (SIP) / ✅ done (WebRTC quality) / deferred (WebRTC DTMF). **SIP:** `SipAdapter::translate_api_event` (crates/sip/rvoip-sip/src/adapter.rs) now has explicit arms for `ApiEvent::DtmfReceived` → `AdapterEvent::Dtmf` and `ApiEvent::MediaQualityChanged` → `AdapterEvent::Quality`. **WebRTC:** `WebRtcAdapter::spawn_quality_emitter` walks routes every 5 s and emits per-Connection `AdapterEvent::Quality` from each stream's `webrtc_stats_snapshot()`. **WebRTC DTMF (P12.8.1 follow-up):** PT 101 frames currently flow through `media::pump` as `MediaFrame{payload_type: Some(101)}` but no RFC 4733 decoder runs on them; needs a pump-side decoder + event channel. UCTP-family adapters were already wired in P5 / P9. Orchestrator-side translation covered by existing `tests/dtmf_auto_route.rs` and `tests/p9_quality_aggregator.rs`. | INTERFACE_DESIGN §2.4 production row; CONVERSATION_PROTOCOL §7.5, §10.3 | P12.8 |
| 3.O.9 | Inline envelope signatures enforced at adapter boundary | `[V1.x]` | JCS + verify primitives in `signing.rs` exist; required-signed policy not gated at adapter ingress | CONVERSATION_PROTOCOL §5.5.1 | deferred |
| 3.O.10 | `rvoip-vcon-postgres` reference store | `[V1.x]` | crate absent; lean per PRD §14.2 #8 was "ship as optional crate" | INTERFACE_DESIGN §11.5 | deferred |

---

## 4. Phased roadmap

11 phases, ~61 tasks. Each phase declares: goal, files
created/modified, public API sketch, acceptance criteria, test
additions. Phases are ordered to maximize value per unit work and avoid
rework; dependencies are called out.

### P1 — Vocabulary lifecycle becomes live `[V1]` — BLOCKER

**Goal.** `Conversation`, `Session`, `Participant` become live objects
the Orchestrator owns and mutates. The 7 lifecycle Commands gain
dispatcher methods. State transitions are enforced.

**Files.**
- Modify: `conversation.rs`, `session.rs`, `participant.rs`,
  `orchestrator.rs`, `events.rs`.
- New: `src/state/conversation_state.rs`, `src/state/session_state.rs`
  (move state-machine logic out of the data shapes).

**Public API sketch.**
```rust
impl Orchestrator {
    pub async fn open_conversation(
        &self,
        tenant_id: TenantId,
        policy: ConversationPolicy,
        metadata: HashMap<String, String>,
    ) -> Result<ConversationId>;

    pub async fn close_conversation(
        &self,
        id: ConversationId,
        force: bool,
    ) -> Result<()>;

    pub async fn start_session(
        &self,
        conversation_id: ConversationId,
        medium: SessionMedium,
        invitees: Vec<ParticipantId>,
    ) -> Result<SessionId>;

    pub async fn end_session(
        &self,
        session_id: SessionId,
        reason: EndReason,
    ) -> Result<()>;

    pub async fn join_session(
        &self,
        session_id: SessionId,
        participant_id: ParticipantId,
        kind: ParticipantKind,
        role: ParticipantRole,
    ) -> Result<()>;

    pub async fn leave_session(
        &self,
        session_id: SessionId,
        participant_id: ParticipantId,
    ) -> Result<()>;
}
```

Plus two registries inside the Orchestrator:
- `conversations: DashMap<ConversationId, Arc<RwLock<Conversation>>>`.
- `sessions: DashMap<SessionId, Arc<RwLock<Session>>>` (with reverse
  index `sessions_by_connection: DashMap<ConnectionId, SessionId>`).

**Tasks (12).**
1. Add Conversation/Session/Participant registries to `Orchestrator`.
2. Implement `open_conversation` + emit `ConversationOpened`.
3. Implement `close_conversation` + emit `ConversationClosed`.
4. Implement `start_session` + emit `SessionStarted`.
5. Implement `end_session` + emit `SessionEnded`.
6. Implement `join_session` + emit `ParticipantJoined`.
7. Implement `leave_session` + emit `ParticipantLeft`.
8. Wire `RouteInboundConnection::Accept { session_id, participant_id }`
   to attach the inbound Connection to its Session.
9. Enforce state transitions (e.g. reject `start_session` on
   `Closed` Conversation, reject `join_session` on `Ended` Session).
10. Maintain `Session::connections` map and `Session::participants` set
    as Connections come/go.
11. Idle-timer scaffolding for `ConversationPolicy::Ephemeral` (driver
    lands in P10; this phase just plumbs the "last activity" timestamp).
12. Reverse-index lookup helpers for `session_of(connection_id)`.

**Acceptance.**
- `OpenConversation → StartSession → JoinSession (×N) → EndSession →
  CloseConversation` flow returns clean IDs and fires the 5 expected
  events in order.
- State-machine violations (start a Session on a Closed Conversation,
  join an Ended Session) return `RvoipError::InvalidState`.
- `cargo test -p rvoip-core --test conversation_lifecycle` passes.

**Tests.**
- `tests/conversation_lifecycle.rs` — open/close, double-close idempotent
  with `force=true`, close rejects when force=false and active Sessions
  exist.
- `tests/session_lifecycle.rs` — start/end/join/leave, state transitions,
  events emitted in order, ParticipantJoined/Left fired.

---

### P2 — Mute, PlayAudio, BridgeTo cleanup `[V1]`

**Goal.** Clear the `NotImplemented` shrapnel on existing surfaces. Low
risk, mostly trait widening.

**Files.**
- Modify: `adapter.rs` (add trait methods), `orchestrator.rs`
  (wire dispatch), `events.rs` (PlayAudio cancellation event).
- Test additions in `tests/per_connection_control.rs`.

**Public API sketch.**
```rust
#[async_trait]
pub trait ConnectionAdapter: Send + Sync {
    // ... existing methods ...

    async fn mute(
        &self,
        conn: ConnectionId,
        direction: MuteDirection,
    ) -> Result<()> { Err(RvoipError::NotImplemented("mute")) }

    async fn unmute(
        &self,
        conn: ConnectionId,
        direction: MuteDirection,
    ) -> Result<()> { Err(RvoipError::NotImplemented("unmute")) }

    async fn play_audio(
        &self,
        conn: ConnectionId,
        source: AudioSource,
    ) -> Result<PlaybackHandle> { Err(RvoipError::NotImplemented("play_audio")) }
}

pub struct PlaybackHandle {
    pub id: PlaybackId,
    cancel: oneshot::Sender<()>,
}

impl PlaybackHandle {
    pub fn cancel(self) -> Result<()> { /* fire oneshot */ }
}

impl Orchestrator {
    pub async fn mute(&self, conn: ConnectionId, dir: MuteDirection) -> Result<()>;
    pub async fn unmute(&self, conn: ConnectionId, dir: MuteDirection) -> Result<()>;
    pub async fn play_audio(&self, conn: ConnectionId, src: AudioSource)
        -> Result<PlaybackHandle>;
}
```

**Tasks (4).**
1. Add `mute`/`unmute`/`play_audio` to `ConnectionAdapter` trait with
   `NotImplemented` defaults. Update example adapters in tests.
2. Wire `Orchestrator::mute`/`unmute` to dispatch (replace
   `NotImplemented` returns at `orchestrator.rs:761–779`).
3. Add `Orchestrator::play_audio` + `PlaybackHandle` + `PlaybackId`
   newtype in `ids.rs`.
4. Implement `InboundAction::BridgeTo` at `orchestrator.rs:600`:
   originate outbound leg then call `bridge_connections(inbound,
   outbound)`. Emit `ConnectionsBridged`.

**Acceptance.**
- `Orchestrator::mute(conn, Send)` no longer returns `NotImplemented`;
  test adapter records the call.
- `InboundAction::BridgeTo` originates + bridges + emits
  `ConnectionsBridged` in one shot.
- `PlayAudio` returns a handle whose `cancel()` aborts the playback.

**Tests.**
- `tests/per_connection_control.rs` — mute/unmute round-trip,
  PlayAudio + cancel, BridgeTo end-to-end with two stub adapters.

---

### P3 — vCon emission load-bearing `[V1]` — BLOCKER (depends on P1)

**Goal.** Every Session produces a `VconReady` event when it ends. The
in-flight builder collects parties/dialogs/analyses/attachments
automatically as the Session progresses.

**Files.**
- New: `src/vcon_builder.rs` (default `VconBuilderHandle` implementor
  bound to a Session).
- Modify: `session.rs` (add `vcon_handle()` accessor), `orchestrator.rs`
  (auto-collect on lifecycle events, auto-emit on SessionEnded),
  `vcon.rs` (populate `VconRef::Local` when stored),
  `store/vcon_store.rs` (real `put` for the in-memory variant; wire
  optional `rvoip-vcon` for signing/encryption).
- Possibly: thin shim crate dep on `rvoip-vcon`.

**Public API sketch.**
```rust
impl Session {
    pub fn vcon_handle(&self) -> Arc<dyn VconBuilderHandle>;
}

pub struct DefaultVconBuilder {
    inner: Mutex<VconSnapshot>,
}
impl VconBuilderHandle for DefaultVconBuilder { /* ... */ }

impl Orchestrator {
    pub async fn finalize_vcon(&self, session_id: SessionId)
        -> Result<VconHandle>;
    // Called automatically on SessionEnded; exposed for tests / forced
    // mid-session snapshots.
}
```

**Tasks (6).**
1. Implement `DefaultVconBuilder` and bind one per Session at
   `start_session`.
2. Add `Session::vcon_handle()` accessor returning an `Arc<dyn
   VconBuilderHandle>`.
3. Wire auto-collection: `ParticipantJoined` → `add_party`; stream
   established → `add_dialog`; `TranscriptTurn` (when it exists, J3)
   → `add_analysis`.
4. On `SessionEnded`, snapshot the builder, hand to `VconStore::put`,
   emit `Event::VconReady { vcon_handle: VconHandle, ... }`.
5. Replace `MemoryVconStore::put` mock URL with a real local URL +
   sha256 content hash. Populate `VconRef::Local { uuid }`.
6. Optional feature `vcon-signing` gates a wire to `rvoip-vcon` for JWS
   sign + JWE encrypt.

**Acceptance.**
- Ending any Session fires exactly one `VconReady` event with a
  resolvable handle.
- `VconStore::get(handle).await?.unwrap()` returns bytes whose sha256
  matches `handle.content_hash`.
- `VconSnapshot.parties.len() == session.participants.len()`.

**Tests.**
- `tests/vcon_emission.rs` — Session with 2 participants emits VconReady
  with both parties; snapshot survives `VconStore::get`; ending without
  any explicit `add_*` still produces a valid (mostly-empty) vCon.

---

### P4 — Messaging operations `[V1]` (depends on P1)

**Goal.** Messages become a first-class Conversation operation with
history, receipts, and cross-substrate delivery.

**Files.**
- New: `src/messaging.rs` — message fan-out planner, history pager.
- Modify: `orchestrator.rs` (new methods), `message.rs` (no shape
  change), `store/conversation_store.rs` (history pagination cursor),
  `adapter.rs` (clarify `send_message` semantics — single-substrate
  hop only).

**Public API sketch.**
```rust
impl Orchestrator {
    pub async fn send_message_to_conversation(
        &self,
        conversation_id: ConversationId,
        message: Message,
    ) -> Result<MessageId>;

    pub async fn send_message_to_connection(
        &self,
        connection_id: ConnectionId,
        message: Message,
    ) -> Result<MessageId>;  // renamed from existing `send_message`

    pub async fn list_messages(
        &self,
        conversation_id: ConversationId,
        filter: MessageFilter,
        page: PageCursor,
    ) -> Result<MessagePage>;

    pub async fn mark_message_read(
        &self,
        message_id: MessageId,
        by_participant: ParticipantId,
    ) -> Result<()>;
}

pub struct MessageFilter {
    pub from_participant: Option<ParticipantId>,
    pub content_types: Option<Vec<ContentType>>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
}

pub struct PageCursor { /* opaque */ }
pub struct MessagePage {
    pub messages: Vec<Message>,
    pub next: Option<PageCursor>,
}
```

**Tasks (5).**
1. Rename existing `Orchestrator::send_message` →
   `send_message_to_connection` (back-compat re-export `send_message`).
2. Implement `send_message_to_conversation`: enumerate Connections per
   Participant, dispatch via per-Connection `send_message`, aggregate
   results, emit `MessageSent` once + `MessageDelivered` per substrate.
3. Implement `list_messages` with cursor pagination against the
   ConversationStore.
4. Implement `mark_message_read` → emit `MessageRead`.
5. Attachment policy: inline if `body.len() ≤ 64 * 1024`; otherwise
   require `attachments[]` and a URL pointer.

**Acceptance.**
- A Conversation with 3 Connections (SIP + WebRTC + UCTP via stub
  adapters) delivers one logical message to all 3 with 1 `MessageSent` +
  3 `MessageDelivered` events.
- History pagination returns the correct slice; cursor round-trips.
- Oversized inline body returns `RvoipError::AttachmentRequired`.

**Tests.**
- `tests/messaging.rs` — fan-out, pagination, read receipts,
  oversized-inline rejection.

---

### P5 — Recording + Transcription + AI harness `[V1]` (depends on P1, P3)

**Goal.** Recording and transcription become Orchestrator operations
with clean provider trait surfaces. AI harness gets attach/detach +
barge-in primitives.

**Files.**
- New crate: `crates/extensions/rvoip-harness/` containing trait surfaces
  (`AsrProvider`, `TtsProvider`, `DialogManager`, `RecordingSink`) +
  no-op default implementations + cancellation primitives.
- Modify: `orchestrator.rs` (attach/detach methods + recording dispatch),
  `commands.rs` (rename `RecordingSink` data enum to `RecordingSinkRef`
  to disambiguate from the trait), `events.rs` (add `BargeInDetected`).

**Public API sketch.**
```rust
// in rvoip-harness
#[async_trait]
pub trait AsrProvider: Send + Sync {
    async fn open_stream(&self, conn: ConnectionId, config: AsrConfig)
        -> Result<Box<dyn AsrStream>>;
}
#[async_trait]
pub trait TtsProvider: Send + Sync {
    async fn synthesize(&self, text: &str, voice: Option<&str>)
        -> Result<Box<dyn TtsPlayback>>;
}
#[async_trait]
pub trait DialogManager: Send + Sync {
    async fn turn(&self, transcript: &TranscriptTurn) -> Result<DialogAction>;
}
#[async_trait]
pub trait RecordingSink: Send + Sync {
    async fn write(&self, frame: MediaFrame) -> Result<()>;
    async fn close(&self) -> Result<RecordingArtifact>;
}

// in rvoip-core
impl Orchestrator {
    pub async fn attach_ai(
        &self,
        connection_id: ConnectionId,
        provider_ref: String,
        config: HashMap<String, String>,
    ) -> Result<AiAttachmentId>;

    pub async fn attach_listener(
        &self,
        target: ListenerTarget,
        sink: ListenerSink,
    ) -> Result<ListenerId>;

    pub async fn detach(&self, attachment: AttachmentRef) -> Result<()>;

    pub async fn start_recording(
        &self,
        target: RecordingTarget,
        sink_ref: RecordingSinkRef,
    ) -> Result<RecordingId>;
    pub async fn stop_recording(&self, id: RecordingId) -> Result<RecordingArtifact>;
    pub async fn pause_recording(&self, id: RecordingId) -> Result<()>;
    pub async fn resume_recording(&self, id: RecordingId) -> Result<()>;

    pub async fn start_transcription(
        &self,
        target: RecordingTarget,
        provider_ref: String,
    ) -> Result<TranscriptionId>;
    pub async fn stop_transcription(&self, target: RecordingTarget) -> Result<()>;
}
```

**Tasks (9).**
1. Create `crates/extensions/rvoip-harness` skeleton (Cargo.toml, lib.rs, 4 traits,
   no-op default impls).
2. Add provider registry to `Orchestrator` (name → Arc<dyn AsrProvider>
   etc.) populated via builder API.
3. Implement `attach_ai` — opens ASR stream + spawns dialog loop +
   wires TTS playback back into Connection.
4. Implement `attach_listener` — taps frames into RecordingSink without
   mixing (separated streams default).
5. Implement `detach` — cancels the tap/loop and emits Detached event.
6. Implement `start_recording` / `stop_recording` / `pause` / `resume` —
   per-tap MediaFrame route into `RecordingSink::write`. Allocate
   `RecordingId`.
7. Implement `start_transcription` / `stop_transcription` — same shape
   as recording but feeds `AsrProvider` and emits `TranscriptTurn`.
8. Dual-ASR: when AI is attached on a Connection and transcription is
   also active, allocate two ASR streams (different provider refs OK).
9. Barge-in primitive: when ASR yields speech during TTS playback,
   cancel the active `PlaybackHandle` and emit `BargeInDetected`.

**Acceptance.**
- AI attach drives a synthetic ASR→Dialog→TTS loop in tests; barge-in
  cancels mid-TTS playback.
- Recording with `RecordingSink::write` collects N MediaFrames; stop
  produces an artifact with the expected byte count.
- Dual-ASR mode produces two independent `TranscriptTurn` streams.

**Tests.**
- `tests/recording.rs` — start/stop/pause/resume, frame count,
  pause-skip semantics.
- `tests/ai_harness.rs` — attach drives a stub Dialog; barge-in cancels
  TTS; detach cleans up.

---

### P6 — Multi-adapter dispatch, tenant scoping, capacity `[V1]`

**Goal.** Multiple adapters can register for different transports
without conflict; per-tenant Conversations/quotas are isolated;
capacity is reported on schedule.

**Files.**
- Modify: `adapter.rs` (transport selector on `OriginateRequest`),
  `orchestrator.rs` (tenant index + quota table + capacity scheduler),
  `config.rs` (per-tenant quota config).
- New: `src/tenant.rs` (per-tenant registry + quota state),
  `src/capacity.rs` (admission + scheduler).

**Public API sketch.**
```rust
pub struct OriginateRequest {
    // ... existing ...
    pub transport: Transport,  // NEW
}

pub struct TenantQuotas {
    pub max_concurrent_sessions: Option<usize>,
    pub max_concurrent_recordings: Option<usize>,
    pub max_concurrent_ai_sessions: Option<usize>,
}

impl Orchestrator {
    pub fn set_tenant_quotas(&self, tenant: TenantId, q: TenantQuotas);

    pub async fn capacity_report(&self) -> CapacityReport;
    // Also: periodic emit on configurable cadence.
}
```

**Tasks (4).**
1. Add `transport: Transport` to `OriginateRequest`. Update
   `originate_connection` to look up adapter by transport (replace the
   "first registered" hack at `orchestrator.rs:606–622`).
2. Add tenant registries: `conversations_by_tenant: DashMap<TenantId,
   DashSet<ConversationId>>`. Enforce in all lookups.
3. Implement per-tenant quota check on `start_session`,
   `start_recording`, `attach_ai`. Reject with `AdmissionRejected("quota:
   sessions")` etc.
4. Capacity scheduler: `tokio::spawn` a periodic task that emits
   `Event::CapacityReport` on a configurable interval (default 30s).

**Acceptance.**
- Two adapters registered for `Sip` and `WebRtc`; originate with each
  transport routes to the right adapter.
- Tenant A's Conversation IDs invisible to Tenant B's
  `list_conversations`.
- Exceeding `max_concurrent_sessions` returns admission error.
- CapacityReport fires every 30s with correct counts.

**Tests.**
- `tests/multi_adapter.rs` — two-transport dispatch.
- `tests/tenant_isolation.rs` — cross-tenant invisibility, quota
  enforcement.
- `tests/capacity.rs` — scheduler fires events, counts are accurate.

---

### P7 — IdentityProvider completion + signing helpers `[V1]` trait, `[V1.x]` impls

**Goal.** Identity surface matches INTERFACE_DESIGN §8 spec. Per-request
signing has canonicalization helpers ready for adapters to use.

**Files.**
- Modify: `identity.rs` (add 3 missing trait methods),
  `adapter.rs` (clarify verify_request_signature contract),
  `orchestrator.rs` (step-up flow methods).
- New: `src/signing.rs` (RFC 9421 + JCS helpers, replay cache),
  `src/identity/step_up.rs`.

**Public API sketch.**
```rust
#[async_trait]
pub trait IdentityProvider: Send + Sync {
    // ... existing 6 ...
    async fn register_agent_key(&self, id: IdentityId, key: Jwk) -> Result<()>;
    async fn verify_signature(
        &self,
        id: IdentityId,
        sig: SignatureHeaders,
        body: &[u8],
    ) -> Result<IdentityAssurance>;
    async fn derive_dtls_fingerprint(&self, id: IdentityId)
        -> Result<Option<DtlsFingerprint>>;
}

pub mod signing {
    pub fn canonical_envelope(env: &serde_json::Value) -> Vec<u8>;  // RFC 8785 JCS
    pub fn parse_signature_input(headers: &SignatureHeaders) -> Result<SignatureSpec>;
    pub fn verify(spec: &SignatureSpec, pubkey: &Jwk, body: &[u8]) -> Result<()>;

    pub struct ReplayCache { /* envelope.id LRU, 5min default */ }
    impl ReplayCache {
        pub fn check_and_record(&self, envelope_id: &str) -> Result<()>;
    }
}

impl Orchestrator {
    pub async fn request_step_up(
        &self,
        connection_id: ConnectionId,
        required: IdentityAssuranceRequirement,
    ) -> Result<()>;
    pub async fn complete_step_up(
        &self,
        connection_id: ConnectionId,
        credential: Credential,
    ) -> Result<IdentityAssurance>;
}
```

**Tasks (5).**
1. Add the 3 missing `IdentityProvider` methods with default
   `NotImplemented`.
2. Implement `signing` module: JCS canonicalization, Signature-Input
   parsing, EdDSA verify, replay cache.
3. Implement `request_step_up` / `complete_step_up` — emit
   `IdentityAssuranceChanged` on success.
4. Wire envelope handlers in adapter crates (UCTP family) to use the
   replay cache.
5. Document the v1 vs v1.x split: trait is v1; default-on signing is
   v1.x.

**Acceptance.**
- IdentityProvider trait has all 9 methods.
- `signing::canonical_envelope` produces byte-for-byte JCS output for
  the §8.3 fixture.
- Step-up flow: lower-assurance Connection requests upgrade, supplies
  credential, assurance changes, event fires.

**Tests.**
- `tests/identity_assurance.rs` — step-up flow against a stub provider.
- `tests/signing.rs` — JCS fixture from RFC 8785; EdDSA verify against
  known-good vectors; replay cache rejects duplicate IDs.

---

### P8 — Multi-party MP2 + MP3c `[V1]`

**Goal.** UCTP-family adapters can complete the multi-party path. The
wire-side handler for subscribe/unsubscribe envelopes calls into
Orchestrator. Per-subscriber stream allocation is real.

**Files.**
- Modify: `adapter.rs` (require `allocate_subscriber_stream` to be
  implemented in UCTP-family adapters), `orchestrator.rs` (active-
  speaker advisory).
- New work in `rvoip-quic`, `rvoip-webtransport`: wire envelope handlers,
  implement `allocate_subscriber_stream`.

**Tasks (3).**
1. Implement `stream.subscribe` / `stream.unsubscribe` envelope
   handlers in the UCTP adapter coordinator. Call
   `Orchestrator::add_subscription` / `remove_subscription`.
2. Implement `allocate_subscriber_stream` in `rvoip-quic` and
   `rvoip-webtransport` (allocate per-subscriber datagram-keyed egress
   stream with `stream_local_id` rewriting).
3. Active-speaker advisory: optional periodic emit of
   `stream.active-speaker` envelope based on RTP audio level extension
   per CONVERSATION_PROTOCOL §6.

**Acceptance.**
- 3-party UCTP session: each subscriber receives only frames from
  publishers it's subscribed to.
- `stream.active-speaker` envelopes fire on speaker change.

**Tests.**
- Existing fanout tests pass with real UCTP adapter (not stub).
- New `tests/multi_party_wire.rs` exercising envelope handlers end-to-
  end.

---

### P9 — Observability surface `[V1]` (depends on P1)

**Goal.** All v1 observability signals defined in PRD §10 + §11 are
emitted with full payloads.

**Files.**
- Modify: `events.rs` (extend `SessionEnded`, add `TranscriptTurn` if
  absent, add `SessionQualityReport` struct).
- New: `src/quality.rs` (SessionQualityReport aggregator),
  `src/usage.rs` (UsageRecord aggregator), `src/tracing.rs` (OTel
  spans), `src/metrics.rs` (Prometheus gauges).

**Tasks (6).**
1. Define `SessionQualityReport` struct (MOS, packet_loss_pct, jitter_ms,
   rtt_ms, codec, bitrate_bps, talk_pct, silence_pct, pdd_ms,
   ring_time_ms, setup_time_ms, hangup_reason). Extend `Event::SessionEnded`
   to carry it.
2. Confirm `TranscriptTurn` event exists; add if missing with stream_id,
   speaker (participant_id), text, confidence, is_final, provider_ref,
   started_at, ended_at.
3. Implement UsageRecord aggregator: per-session counters
   (PSTN_minutes, ASR audio-sec, TTS chars, LLM tokens, recording
   bytes/duration, transfer count). Emit on session end + periodic
   15min for long calls. Separate event channel from operational.
4. Registration normalization: subscribe to `rvoip-sip-registrar`
   events via infra-common, re-emit as `RegistrationChanged` /
   `RegistrationHeartbeat`.
5. Periodic MediaQuality cadence: per-Connection 1Hz default,
   configurable.
6. OTel/Prometheus pass: one span per Session (children per
   Connection/AI turn/transfer); add the global gauges PRD §11 calls
   for.

**Acceptance.**
- Closing a Session emits `SessionEnded` with a non-default
  `SessionQualityReport`.
- `UsageRecord` events arrive on a separate broadcast channel and
  aggregate per tenant.
- Prometheus endpoint scrape shows active_calls, calls_per_sec, etc.

**Tests.**
- `tests/quality_report.rs` — SessionEnded carries non-zero MOS when
  frames flowed.
- `tests/usage_record.rs` — counters match observed traffic in a stub
  session.

---

### P10 — Closure policy + richer store filters `[V1]` (depends on P1)

**Goal.** `ConversationPolicy::Ephemeral` actually closes idle
Conversations. ConversationStore supports the filter shapes PRD §10
calls for.

**Files.**
- Modify: `store/conversation_store.rs` (widen trait + memory impl),
  `orchestrator.rs` (idle-timer driver).

**Public API sketch.**
```rust
#[async_trait]
pub trait ConversationStore: Send + Sync {
    // ... existing 4 ...
    async fn list(
        &self,
        filter: ConversationFilter,
    ) -> Result<Vec<Conversation>>;
}

pub struct ConversationFilter {
    pub tenant: Option<TenantId>,
    pub participant: Option<ParticipantId>,
    pub identity: Option<IdentityId>,
    pub state: Option<ConversationState>,
    pub opened_since: Option<DateTime<Utc>>,
    pub opened_until: Option<DateTime<Utc>>,
}
```

**Tasks (3).**
1. Idle-timer driver: per Conversation a tokio task watches
   last_activity_at; when policy is `Ephemeral { idle_close_secs }` and
   `Instant::now() - last_activity > N + no active Sessions + no recent
   Messages`, call `close_conversation`.
2. Widen `ConversationStore` trait with `list(filter)` (back-compat:
   keep `list_for_tenant` as a default that calls `list`).
3. Update `MemoryConversationStore` to implement the new filter.

**Acceptance.**
- Ephemeral Conversation with `idle_close_secs = 1` closes ~1s after
  last activity in tests.
- `list({ state: Open, participant: P })` returns only matches.

**Tests.**
- `tests/closure_policy.rs` — idle timer fires; activity resets.
- `tests/store_filters.rs` — every filter dimension exercised.

---

### P11 — Feature flags, workspace layout, test polish `[V1]`

**Goal.** Cargo features match INTERFACE_DESIGN §2.2; sibling crates
exist or are explicitly stubbed; the v1 test surface is complete.

**Files.**
- Modify: `Cargo.toml` (add `[features]`), workspace `Cargo.toml`
  (register new crates).
- New: `crates/identity/rvoip-identity/` skeleton, `crates/rvoip-client/`
  skeleton (deferred internals but present for layering).

**Tasks (4).**
1. Add `[features]` to `crates/foundation/rvoip-core/Cargo.toml`:
   ```toml
   [features]
   default = ["uctp", "sip", "rtp", "media", "vcon", "identity"]
   uctp = []
   sip = []
   rtp = []
   media = []
   vcon = []
   identity = []
   webrtc = []
   aauth-experimental = []
   identity-fingerprint-binding = []
   harness = ["dep:rvoip-harness"]
   client = []
   full = ["webrtc", "harness", "client", "aauth-experimental",
           "identity-fingerprint-binding"]
   ```
2. Spin off `rvoip-harness` (done in P5) and `rvoip-identity` (provider
   trait stays in core for layering; backends live here).
3. Wire `rvoip-vcon` as optional dep behind `vcon-signing` feature.
4. Doc pass: update `lib.rs` rustdoc + this GAP_PLAN.md status table to
   reflect what landed.

**Acceptance.**
- `cargo build -p rvoip-core --no-default-features` compiles.
- `cargo build -p rvoip-core --features full` compiles.
- `cargo test --workspace` passes.

---

### P12 — Close cross-doc gaps `[V1]`

**Goal.** Close the eight `[V1]` items surfaced by the 2026-05-26
audit pass (§3.O.1 – §3.O.8). Each task is independently shippable;
order is suggested, not required.

**Tasks (8).**

1. **P12.1 Carve `session-core` into `rvoip-sip`.** Move
   `crates/session-core/src/` into `crates/sip/rvoip-sip/src/session/`,
   update workspace `Cargo.toml`, retarget every `session_core::*`
   import. Delete `crates/session-core/`. Spec: INTERFACE_DESIGN §13.1.
   *Acceptance:* workspace builds; no remaining `use session_core::`
   in the tree; `crates/session-core/` does not exist.

2. **P12.2 Wind down `orchestration-core`; rename to `rvoip`
   facade.** Lift workforce-shaped code (queue, agent, routing —
   already documented as consumer concerns in PRD §13) into example
   code under `crates/rvoip/examples/` or delete outright. Merge the
   surviving spine (bridge management, call lifecycle SIP-shaped
   bits) into `rvoip-sip`. Rename `orchestration-core` →  `rvoip`
   facade per INTERFACE_DESIGN §13.3 step 7. *Acceptance:*
   `crates/orchestration-core/` does not exist; `cargo build
   -p rvoip` is the facade build.

3. **P12.3 Create `crates/rvoip-client/`.** New crate with `Client`,
   `connect`, `call`, `send_message`, `incoming`, `conversations`,
   `close` methods; `SessionHandle` with accept/reject/end/hold/
   resume/mute/send_dtmf/streams/events; `InboundEvent` enum per
   INTERFACE_DESIGN §15.2. Re-export per-protocol native client
   surfaces at `rvoip::sip::client::*`, `rvoip::webrtc::client::*`,
   `rvoip::uctp::client::*` per §15.3. *Acceptance:* `cargo build
   -p rvoip-client --features client` succeeds; client-side smoke
   example places a UCTP call against a local Orchestrator.

4. **P12.4 Hello-world sketches §16.2 / §16.3 / §16.4.** Add three
   new `examples/` binaries:
   - `examples/sip_webrtc_bridge.rs` (§16.2, ~100 lines)
   - `examples/uctp_only_server.rs` (§16.3, ~150 lines)
   - `examples/full_thelve_shape.rs` (§16.4, ~300 lines)
   Each runs end-to-end against the corresponding feature flag set.
   *Acceptance:* all four sketches compile with their declared
   feature flag set and execute their `main()` against stub adapters.

5. **P12.5 Land `deny.toml` with the V2.A.10 rule.** Create
   `/Volumes/D2-2019/Developer/rvoip/deny.toml` with a
   `[bans]` section that fails CI if any of `rvoip-sip`,
   `rvoip-webrtc`, `rvoip-quic`, `rvoip-webtransport`,
   `rvoip-websocket`, `rvoip-uctp` declares a direct
   `rvoip-core` dependency (they must use `rvoip-core-traits`).
   Wire `cargo deny check` into the CI workflow. *Acceptance:*
   `cargo deny check bans` passes on a clean workspace; planting a
   regression dep makes it fail.

6. **P12.6 Wire step-up auth envelope round-trip.** In
   `rvoip-uctp::state::orchestrator_handler`: on `identity.step-up
   -request`, hold the offending envelope server-side; on
   `identity.step-up-response`, verify the credential via
   `IdentityProvider::authenticate`, emit
   `IdentityAssuranceChanged`, replay the held envelope. Time out
   held envelopes per CONVERSATION_PROTOCOL §5.8 with `error 403-1`
   on failure. *Acceptance:* end-to-end test where a Connection at
   `Identified` requests step-up to `UserAuthorized`, supplies a
   passkey credential, and the originally-blocked `session.invite`
   succeeds.

7. **P12.7 OpenTelemetry span hierarchy.** Add an `otel` module to
   `rvoip-core` that opens one span per Session at `SessionStarted`
   and closes it at `SessionEnded`. Open child spans per
   Connection lifecycle, per AI dialog turn, per transfer.
   `tracing_opentelemetry` is the natural integration point so
   existing `tracing::info!` records become structured events on
   the span. Spec: PRD §10.2; INTERFACE_DESIGN §5. *Acceptance:*
   exporting to a local Jaeger / Tempo instance shows nested
   spans for a synthetic 3-party Session with a mid-call transfer.

8. **P12.8 DTMF & `connection.quality` adapter wiring.** Confirm
   `rvoip-sip` emits `AdapterEvent::DtmfReceived` for RFC 2833 +
   SIP INFO and consumes `Orchestrator::send_dtmf` to put DTMF on
   the wire; same for `rvoip-webrtc` (RFC 4733). Confirm
   per-Connection `AdapterEvent::Quality` snapshots flow at the
   spec'd cadence from `rvoip-sip` (RTCP-XR) and `rvoip-webrtc`
   (`RTCStatsReport`) into the `QualityAggregator`. *Acceptance:*
   `tests/dtmf_through_adapters.rs` exercises round-trip for both
   adapters; `tests/quality_through_adapters.rs` asserts non-zero
   MOS aggregation from synthetic adapter quality events.

**Effort estimate.** P12.1/.2 are mechanical workspace surgery
(~1 day each). P12.3 is the largest (~3 days, new public API).
P12.4 ~1 day. P12.5 ~half-day. P12.6 ~1 day (test setup
dominates). P12.7 ~1 day. P12.8 ~1 day per adapter (~2 days). Total
~10–12 focused days; order them per dependency / risk preference.

**Risk + mitigation.**
- *Risk:* P12.2 destabilizes downstream consumers that import from
  `orchestration_core::*`. *Mitigation:* land P12.1 first (it
  removes the largest source of orchestration-core surface); ship
  a one-release-cycle of re-exports from the new home before
  deleting.
- *Risk:* P12.7 OTel integration creates async-runtime spaghetti
  with `tracing_subscriber` already configured by consumers.
  *Mitigation:* expose the OTel layer as opt-in; don't install a
  global subscriber from inside `rvoip-core`.

---

## 5. Out-of-scope (deferred per design docs)

The following are listed for tracking only; **no work is proposed** in
this plan:

- `rvoip-websocket` substrate `[V1.x]` — INTERFACE_DESIGN §2.4.
- AAuth production (behind `aauth-experimental`) `[V1.x]` — §8.5.
- RFC 9421 default-on signing `[V1.x]` — §2.4.
- DTLS-SRTP fingerprint binding default-on `[V1.x]` — §8.4.
- `conversation.update` for policy change `[V1.x]` —
  CONVERSATION_PROTOCOL §7.1.
- Multi-party UCTP beyond N=2 via SFU adapter `[V2]` — PRD §5.
- SIP-over-QUIC / RoQ / MoQ adapters `[V2]` — INTERFACE_DESIGN §2.5.

When any of these are taken up, append a new phase to §4.

---

## 6. Open questions

These are decisions only the maintainer can make. Each materially
changes one or more phases above.

1. **`rvoip-harness` spin-off vs in-crate** (affects P5).
   - Spec (INTERFACE_DESIGN §2.1) calls for a separate crate. Proposed
     plan assumes spin-off.
   - Alternative: keep harness trait surface in `rvoip-core` behind
     `harness` feature flag, accept the coupling for v1, defer the
     split to v1.x.
   - **Recommendation:** spin off; trait surface is small and the seam
     is the whole point.

2. **`rvoip-identity` spin-off** (affects P7, P11).
   - Spec says spin off. Trait stays in `rvoip-core::identity`; backends
     live in `rvoip-identity`.
   - Open: do we ship `rvoip-identity` empty in v1 (placeholder for
     OAuth2/DPoP/OIDC impls in v1.x) or wait until the first real
     backend lands?
   - **Recommendation:** ship the crate empty with a no-op
     `BearerProvider`; gives consumers an import path that doesn't
     churn.

3. **Tenant scoping model** (affects P6).
   - Is v1 single-tenant per Orchestrator process (and `tenant_id` is
     just for logging/billing), or true multi-tenant data isolation in
     one process?
   - PRD §10 implies multi-tenant; INTERFACE_DESIGN §11.1 reinforces
     `list_for_tenant`. Proposed plan assumes multi-tenant.
   - If single-tenant: drop the tenant registries in P6, keep just the
     quota table.

4. **`rvoip-vcon` production wiring timing** (affects P3, P11).
   - Wiring `MemoryVconStore` → `rvoip-vcon` signing/encryption — is
     this `[V1]` (block on it for the v1 push) or `[V1.x]`?
   - Proposed plan: trait + emission load-bearing in v1 (P3); signing
     gated behind `vcon-signing` feature, default impl is in-memory
     unsigned. Production signing/encryption is `[V1.x]`.
   - If maintainer wants signed-by-default: promote to `[V1]` and
     extend P3 with 2–3 more tasks.

---

## 7. Verification checklist for the whole plan

When each phase lands, verify:
- `cargo test -p rvoip-core` — all existing tests still pass.
- New tests for the phase pass.
- `cargo build -p rvoip-core --no-default-features` still compiles.
- Updated `examples/sip_only_orchestrator.rs` runs to completion.
- Where externally observable: cross-crate smoke test against
  `rvoip-sip` + at least one UCTP-family adapter shows the new flow
  end-to-end.
- Status table in §2 of this doc updated to reflect landed work.
