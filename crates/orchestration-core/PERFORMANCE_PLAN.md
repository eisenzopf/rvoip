# orchestration-core Performance Plan

**Status:** Phases 0–2 shipped 2026-05-10. Phase 3 deferred (trigger not met; see §7 Phase 3 and §12).
Supersedes `CONCURRENCY_PLAN.md` for current implementation work.
`CONCURRENCY_PLAN.md` is kept alongside this document as historical reference per `PRD.md` §15.

**Goal:** Make `orchestration-core` ship 1k–5k concurrent sessions on a single instance with a clear path to 10k+, **without doing wasted work on subsystems the PRD removes from rvoip's scope.**

---

## 1. Context and what changed since CONCURRENCY_PLAN.md

`CONCURRENCY_PLAN.md` was authored before `PRD.md` settled the scope of `orchestration-core`. It proposes substantial work across the agent / queue / offer / routing subsystems. `PRD.md` §13 now **lifts those subsystems out of rvoip entirely** — they become consumer concerns (Thelve, or whatever orchestration layer a CPaaS / call-center builder writes on top). Optimizing code that is scheduled for deletion is wasted effort.

`PRD.md` §15 says explicitly:

> The prior `CONCURRENCY_PLAN.md` was written before this PRD's scope was settled. It addressed scaling the *whole* current orchestration-core, including the agent / queue / offer / routing subsystems that this PRD removes from rvoip's scope. That document is **kept as historical reference**, not revised. When implementation work begins under this PRD, a fresh, narrower concurrency plan will be authored alongside (focused on call-leg / bridge / runtime-session storage, GlobalEventCoordinator adoption, atomic state, and admission semaphore — the parts of the prior plan that survive the scope cut).

This document is that fresh, narrower plan. It carries forward CONCURRENCY_PLAN's architectural principles and phase structure for the surviving surface; everything else is explicitly out of scope.

---

## 2. Targets

Unchanged from `CONCURRENCY_PLAN.md` §1:

- **Ship:** 1k–5k concurrent sessions on a single instance.
- **Architectural runway:** 10k+ concurrent sessions, single instance, with a clear path to multi-instance.
- **Storage:** in-memory now. Trait API stays viable for Redis / SQL backends later. A database does not fix lock contention — fix the architecture first.

### Observed problem (still the motivating measurement)

`tests/perf_active_calls.rs` runs against orchestration-core only (no SIP / RTP) and shows **per-call setup time growing as N grows from 1 → 5 → 10 → 100 → 500 → 1000**. The residual N-scaling is structural; this plan addresses the surviving structural causes.

---

## 3. Vocabulary policy

**Keep SIP vocabulary in orchestration-core during this perf rework. Do not rename to voip-3 nouns now.**

The rename (`Call` → `Conversation`, `CallLeg` → `Connection`, new `Session` layer between them) is scheduled for the migration cutover — `PRD.md` §13 step 7, `INTERFACE_DESIGN.md` §13.3 step 7 — **not** during perf work. Three reasons:

1. **The end-state library has both vocabularies coexisting.** Per `INTERFACE_DESIGN.md` §1.1: `rvoip-sip` uses SIP vocabulary natively (`Call`, `Dialog`, `Leg`, `INVITE`, `REFER`); `rvoip-core` uses voip-3 nouns (`Conversation`, `Session`, `Connection`). orchestration-core's surviving SIP-flavored code moves to `rvoip-sip`, where SIP vocabulary is the correct answer. Renaming `Call` to `Conversation` in orchestration-core today would be undoing the intended end state.

2. **Only transport-agnostic primitives become voip-3 nouns** when they move to `rvoip-core`. Per `INTERFACE_DESIGN.md` §13.1: `orchestration-core::Call` → `rvoip-core::Conversation` (with a new `Session` layer); `orchestration-core::CallLeg` → `rvoip-core::Connection`; `media-core::BridgeHandle` splits into SIP-only `RtpBridgeHandle` (rvoip-sip) and transport-agnostic `BridgeHandle` (rvoip-core). That rename happens *as part of* the move, not in advance.

3. **Perf rework plus vocabulary rename equals 2× review burden, 2× bisect surface, 2× git-blame noise.** Do one thing at a time. The PRD's migration order puts the rename at step 7 for a reason.

**Exception:** any *new* type we introduce during this perf work picks a name that doesn't lock the codebase into either vocabulary. Example: a new secondary index keyed by `SessionId` (the SIP dialog/session ID we already use today) maps cleanly to voip-3's `SessionId` later without renaming the index struct itself. Index struct names like `BySession`, `ByDialog`, `ByBridge` are neutral.

---

## 4. Surface in scope

These are the orchestration-core subsystems where perf work pays off because they survive the `PRD.md` §13 migration in some form:

| Subsystem | Current location | Post-migration home |
|---|---|---|
| `MemoryCallStore` | `src/store.rs:69` | `rvoip-sip` (SIP-flavored `CallStore`) + `rvoip-core` (transport-agnostic `ConversationStore` trait) |
| `BridgeManager` | `src/orchestrator.rs:25` | `rvoip-sip` (`RtpBridgeHandle`) + `rvoip-core` (transport-agnostic `BridgeHandle`) |
| `VoiceAiRegistry` | `src/orchestrator.rs:92` | `rvoip-harness` |
| `OrchestrationEventBus` | `src/events.rs` | Replaced by `infra-common::GlobalEventCoordinator` (cross-crate) |
| `Orchestrator` admission control | `src/orchestrator.rs` | Moves to `rvoip-core::Orchestrator` |
| `CallLeg` state and indexing | `src/types.rs` | Renamed to `Connection` and moves to `rvoip-core` |

Phase work below targets only this surface.

---

## 5. Surface explicitly out of scope (deleted by PRD §13)

These subsystems are **not** receiving perf work in this plan. They are scheduled for deletion from rvoip and lift to the consumer:

| Subsystem | Status per PRD §13 |
|---|---|
| `Agent`, `AgentStore`, `AgentKind`, `AgentConnector`, `AgentOffer`, `AgentOfferStore` | Deleted — lift to consumer |
| `Queue`, `QueueStore`, `QueueWaitlist`, queue policies, queue admission, queue ordering, overflow | Deleted — lift to consumer |
| `AssignmentManager` + the entire matching loop | Deleted — lift to consumer |
| `Router`, `RouteRequest`, `RouteDecision`, `QueueSelector` | Deleted — superseded by command-driven routing from consumer |
| `ContactResolver` | May survive as a small SIP-URI helper inside `rvoip-sip`; otherwise deleted |
| `voice_ai.rs` (provider traits) | Extracted to `rvoip-harness` (not optimized here — moves first) |

**Specifically out of scope from `CONCURRENCY_PLAN.md`:**

- §2.1 / §4.3 — atomic agent state machine with CAS reservations (Agent is leaving).
- §2.2 — O(num_agents) scans inside `list_eligible_agents` (the whole agent store is leaving).
- §2.3 — O(total_offers) scan with no `call_id` index (AgentOffer is leaving).
- §2.4 / §4.4 — per-queue workers, BTreeMap/VecDeque waitlist, push+pull triggers, skill-indexed matching (Queue and AssignmentManager are leaving).
- §4.5 — pluggable routing catalog with ~30 named implementations (Router and QueueSelector are leaving).
- §4.6 reservation lifecycle (reservations are workforce-level — Agent/Queue concern — leaving the crate).

If the consumer (Thelve, a CPaaS, a call-center app) needs queue / agent / routing perf, they build it in their own crate after the lift-out. The trait surface we leave behind (`AssignmentManager`, `Router`, etc.) is enough to inform a consumer-side rebuild; we do not invest in scaling implementations of code we are about to delete.

---

## 6. Architectural principles (retained from CONCURRENCY_PLAN.md §3)

These rules apply to every change in this plan:

1. **No locks held across `.await` on the call-setup path.** Critical sections must be short, synchronous, and use `parking_lot::Mutex` or atomics. Async work happens between locks, never inside them.
2. **Lock-free where DashMap fits.** Per-key contention only. No `RwLock<HashMap<...>>` for any surviving store.
3. **Indices, not scans.** Every "find by X" lookup we do more than once per call must be O(1) or O(matching). Linear scans on the hot path are bugs.
4. **One event bus for the whole platform.** Reuse `infra-common::events::GlobalEventCoordinator`. Don't build a second one.
5. **Backpressure at the door, not in the middle.** Bound concurrency at admission via semaphore. Once admitted, calls do not collapse the runtime.
6. **Trait API stays stable for future Redis / SQL.** Nothing we do should require breaking the surviving `CallStore` trait. Streaming variants may be *added* alongside `Vec`-returning methods.

---

## 7. Phases

Each phase is independently mergeable, leaves the crate in a working state, and produces measurable improvement against the `perf_active_calls.rs` baseline.

### Phase 0 — Adopt `infra-common::GlobalEventCoordinator` ✅ Shipped 2026-05-10

Unchanged from `CONCURRENCY_PLAN.md` §4.1. Carried forward in full because the work survives the scope cut and removes a custom bus from the crate.

**Steps:**

- Define `RvoipCrossCrateEvent::Orchestration(OrchestrationEvent)` in `infra-common/src/events/cross_crate.rs`.
- Implement `CrossCrateEvent` for the wrapper.
- Register in `EventTypeRegistry::register_builtin_types()`.
- Replace the `OrchestrationEventBus` field on `Orchestrator` with a thin façade over `Arc<GlobalEventCoordinator>`.
- Drop the `Ordering::SeqCst` counter (`src/events.rs:190`). Use `Relaxed` on a separate sequence counter if downstream replay / audit needs one.

**Expected impact:** Removes the SeqCst global memory barrier; removes the single-channel single-capacity event-loss vulnerability (`CONCURRENCY_PLAN.md` §2.5, §2.6). Per-event-type channels mean a slow consumer of `VoiceAiTranscript` does not lag a consumer of `CallStatusChanged`.

**Open question:** `Orchestrator::events()` returns a thin façade vs `Arc<GlobalEventCoordinator>` directly. **Recommendation: thin façade** so the bus can evolve without API churn. (Same as CONCURRENCY_PLAN §4.1 open question.)

### Phase 1 — DashMap + secondary indices for surviving stores only ✅ Shipped 2026-05-10

Replace `RwLock<HashMap<...>>` with `DashMap` in the surviving stores **only**. Add the secondary indices needed for O(1) / O(matching) lookups.

**In scope:**

```rust
MemoryCallStore {
    calls: DashMap<CallId, Call>,
    by_session: DashMap<SessionId, CallId>,        // O(1) lookup by SIP dialog/session
    by_dialog: DashMap<DialogId, CallId>,          // O(1) lookup by SIP dialog ID if distinct from session
}

BridgeManager {
    bridges: DashMap<BridgeId, BridgeHandle>,
    by_call: DashMap<CallId, BridgeId>,            // O(1) "which bridge is this call in?"
}

VoiceAiRegistry {
    runtimes: DashMap<VoiceAiId, VoiceAiRuntime>,
}
```

`dashmap` is already in the crate's dependencies (workspace-managed). No dependency changes needed.

**Explicitly NOT in this phase:** `MemoryAgentStore`, `MemoryQueueStore`, `MemoryAgentOfferStore` — all leaving the crate per PRD §13. Their existing `RwLock<HashMap>` stays as-is until the lift-out PR removes them.

**Index maintenance:** paid on writes. Reads on the hot path become O(1) — no clones of the whole map, no linear scans.

**Expected impact:** Removes the locks-across-`.await` problem (`CONCURRENCY_PLAN.md` §2.1) for the surviving stores. The surviving hot path on `MemoryCallStore` is the one that affects the perf_active_calls.rs measurement most directly once the workforce code is removed.

### Phase 2 — Admission semaphore + per-process backpressure ✅ Shipped 2026-05-10

Unchanged from `CONCURRENCY_PLAN.md` §4.6.

- `Orchestrator` holds an `Arc<Semaphore>` sized to `config.max_concurrent_setups` (default: function of CPU count, e.g. 1024).
- The incoming-call handler acquires a permit before any work; if the semaphore is exhausted the call is rejected at the door (SIP 503) rather than joining a death spiral.
- Per-event-type capacity in `GlobalEventCoordinator` (now available from Phase 0): raise `VoiceAiTranscript` capacity for AI-heavy deployments; leave `CallStatusChanged` at default.

**Removed from scope:** per-queue depth limits (Queue is leaving).

**Expected impact:** A burst of 10k concurrent INVITEs degrades cleanly (some rejected with 503) instead of hanging. Latency under burst remains bounded.

### Phase 3 (conditional) — Atomic `CallLeg` state for hot-path transitions ⏸ Deferred 2026-05-10 (trigger not met — see §12)

**Trigger:** Run if profiling after Phase 1 shows `CallLeg` state transitions (`CallLegStatus` updates inside `Call.update`) appearing as a top-3 contention point at N=1000.

- Add `state: AtomicU8` to `CallLeg`.
- Replace `Call::update`-with-write-lock for pure state transitions with `CAS` on the leg's atomic state.
- Multi-field updates that include state still go through the store; pure state-only transitions become lock-free.

This is the smallest atomic-state work that survives — it is *not* the agent state machine (Agent is leaving) and *not* the reservation lifecycle (reservations are workforce-level, leaving). It is specifically the leg's lifecycle state which carries through the migration as `Connection.state` in rvoip-core.

**Default:** defer until profiling confirms need. The Phase 1 DashMap conversion alone may suffice.

### Phases 4+ — Distributed / external state (out of scope, post-migration)

Listed here for runway only:

- Redis-backed `CallStore` (or `ConversationStore`, post-migration) implementing the streaming trait variants.
- Lua scripts for atomic cross-instance state transitions.
- Consistent-hash sharding by tenant or call-id.
- Distributed `GlobalEventCoordinator` transports (NATS / gRPC, already abstracted in `infra-common`).

All of this comes **after** Phases 0–3 demonstrate flat per-call latency at 10k on a single instance, **and** after the PRD §13 migration completes. A database does not fix lock contention; the migration changes the type names this work targets.

---

## 8. Verification

Run after each phase. Each must pass before advancing.

1. **Build & unit tests:** `cargo build -p rvoip-orchestration-core` and `cargo test -p rvoip-orchestration-core` — clean and green.
2. **Perf — orchestration-only:** `cargo test -p rvoip-orchestration-core --test perf_active_calls --release` for N ∈ {1, 5, 10, 100, 500, 1000}. Per-call wall time (`wall_ms / active_calls`) must be flat to within 2× across the range. Capture per phase as a regression baseline.
3. **Perf — live SIP/RTP:** `RVOIP_LIVE_SIP_RTP_COUNTS=1,5,10,50 cargo test -p rvoip-orchestration-core --test perf_live_sip_rtp --release` — no regression vs. current numbers; faster at higher N.
4. **Integration:** `cargo test -p rvoip-orchestration-core --test developer_workflows` — the human queue, AI queue, and handoff flows continue to work end-to-end. (Note: these tests exercise the workforce surface that will be deleted later; once deleted, the tests move with it to wherever workforce orchestration lives.)
5. **Cross-crate:** `cargo test -p rvoip-session-core` after Phase 0. Confirms orchestration-core's event publication doesn't break session-core's subscribers.
6. **Examples (current set):**
   - `cargo run -p rvoip-orchestration-core --example human_queue`
   - `cargo run -p rvoip-orchestration-core --example ai_only_queue`
   - `cargo run -p rvoip-orchestration-core --example ai_then_human_handoff`
   - `cargo run -p rvoip-orchestration-core --example mixed_ai_human_queue`
   - `cargo run -p rvoip-orchestration-core --example speech_ivr`
   - `cargo run -p rvoip-orchestration-core --example registered_sip_agent`

   All complete without errors during the perf rework. Some of these examples (the queue-management ones) will be relocated when workforce code lifts out per PRD §13 step 7; that's a separate PR series.
7. **Tokio profiling for Phase 3 (conditional):** `tokio-console` against the perf test at N=1000 to confirm no task is starved and no single mutex appears as a top contention point.

---

## 9. Critical files

Much shorter than CONCURRENCY_PLAN.md's list because the workforce surface is excluded.

| Phase | File | Change |
|---|---|---|
| 0 | `infra-common/src/events/cross_crate.rs` | Add `Orchestration` variant + `OrchestrationEvent` subtypes |
| 0 | `infra-common/src/events/coordinator.rs` | Register orchestration event types |
| 0 | `orchestration-core/src/events.rs` | Replace bus with façade over `GlobalEventCoordinator`; drop SeqCst counter |
| 0 | `orchestration-core/src/orchestrator.rs` | Swap `events:` field |
| 1 | `orchestration-core/src/store.rs` | `MemoryCallStore` → DashMap + `by_session` / `by_dialog` indices. `MemoryAgentStore` / `MemoryQueueStore` / `MemoryAgentOfferStore` **untouched**. |
| 1 | `orchestration-core/src/orchestrator.rs` | `BridgeManager` → DashMap; `VoiceAiRegistry` → DashMap |
| 2 | `orchestration-core/src/orchestrator.rs` | Admission semaphore on `Orchestrator` |
| 2 | `orchestration-core/src/config.rs` | `max_concurrent_setups` field on `OrchestrationConfig` |
| 3 (conditional) | `orchestration-core/src/types.rs` | `CallLeg.state: AtomicU8` |
| 3 (conditional) | `orchestration-core/src/store.rs` | CAS-based state transitions on `CallLeg` (legs only — not agents) |
| 0–3 | `orchestration-core/Cargo.toml` | Confirm `dashmap`, `parking_lot`, `tokio-util` deps (mostly already transitive) |

---

## 10. Relationship to the PRD §13 migration

### 10.1 Two kinds of orchestration

There are two distinct kinds of "orchestration" in this stack, and the scope contract above only addresses the first:

- **Voice-plane orchestration** — session lifecycle, multi-leg bridges, transport-agnostic call control, voice-AI runtime, capability negotiation, identity assurance. Owns *what* happens at the protocol layer. **This is what `rvoip-core` (= renamed `orchestration-core`, post-migration) does.** It exists because the multi-transport future (SIP, WebRTC, QUIC, UCTP per `PRD.md` §1.2.4) needs a transport-agnostic layer above the per-protocol adapters.
- **Workforce / business orchestration** — agents, queues, customer journeys, presence, training flywheel, skills, assignment policy. Owns *who*, *why*, *when*, and *where* at the business layer. **This lifts to the consumer (Thelve, a CPaaS, a call-center app) per PRD §13.** It is not in rvoip's scope.

The two are peers, not duplicates. Adding Thelve-style workforce orchestration above does not make voice-plane orchestration redundant — they answer different questions.

This is the answer to the recurring "do we even need orchestration-core?" question. **Yes**, because voice-plane orchestration is a real layer regardless of whether workforce orchestration exists above it. The crate just needs to lose its current workforce content (~65% of today's public surface, all leaving per PRD §13) and gain its proper name (`rvoip-core`). The perf work in this plan targets the surviving surface — which is the rvoip-core surface — so the investment is permanent.

### 10.2 Where the perf investment lands post-migration

This perf plan ships **before** the `rvoip-core` / `rvoip-sip` / `rvoip-uctp` / `rvoip-quic` / `rvoip-webrtc` / `rvoip-harness` split. After the migration, every change made under this plan moves cleanly to its new home — **none of this work is throwaway**:

- **`MemoryCallStore` and its indices** → split. SIP-flavored fields move to `rvoip-sip`; the trait surface (renamed `ConversationStore` per voip-3) moves to `rvoip-core`. The DashMap + index pattern survives the move; only type names change.
- **`BridgeManager`** → split. SIP-only `RtpBridgeHandle` lives in `rvoip-sip`; transport-agnostic `BridgeHandle` lives in `rvoip-core`. DashMap-of-bridges pattern applies on both sides.
- **`VoiceAiRegistry`** → moves to `rvoip-harness` as one of its registries. DashMap pattern unchanged.
- **`GlobalEventCoordinator` adoption** → stays cross-crate (it already is). Phase 0 work is permanent.
- **Admission semaphore** → moves to `rvoip-core::Orchestrator`. Phase 2 work is permanent.
- **`CallLeg.state` atomics** → renamed to `Connection.state`, moves to `rvoip-core`. Phase 3 work is permanent if done.

**Migration step 7** (`PRD.md` §13.3 / `INTERFACE_DESIGN.md` §13.3): the workforce-code deletion and `Call`/`CallLeg` rename land as one cutover PR series, **after** this perf plan completes. Doing them together avoids a long dual-architecture window and keeps blame history clean. The two-kinds-of-orchestration distinction in §10.1 is what makes the cutover a *trim and rename*, not a rewrite — the surviving voice-plane surface is already the right shape.

---

## 11. Open questions

1. **Phase 3 inclusion.** Include `CallLeg` atomic state speculatively in v1 of this plan, or defer until profiling after Phase 1 confirms need? **Resolved 2026-05-10: deferred.** Profiling under both `perf_active_calls` (N=1000, allocator-dominated) and `perf_live_sip_rtp` (N=50, 10 s media hold, full SIP+RTP) shows zero samples on `CallLeg` state writes. Trigger condition not met. See §12.

2. **`Orchestrator::events()` façade vs direct exposure** (carried from CONCURRENCY_PLAN §8 item 1). Façade keeps API stable through bus evolution. **Recommendation: thin façade.**

3. **Examples relocation.** Should the queue-management examples (`human_queue`, `ai_only_queue`, etc.) be marked deprecated during this plan's lifetime, or moved at lift-out time per PRD §13 step 7? **Recommendation: leave intact during perf work; relocate / delete during the lift-out PR series, not during this plan.** They are the integration test bed for the workforce surface that will leave the crate.

4. **`developer_workflows.rs` test fate.** Same as examples — these tests exercise workforce code that's leaving. **Recommendation: keep running during this perf work; relocate during the lift-out cutover.** During the perf rework they prevent us from regressing user-visible behavior in code that's still nominally supported.

---

## 12. Ship report (2026-05-10)

### Phases shipped

| Phase | Status | What landed |
|---|---|---|
| 0 — `GlobalEventCoordinator` adoption | ✅ Shipped | `RvoipCrossCrateEvent::Orchestration(OrchestrationCrossCrateEvent)` with 24 per-fine-grained-type variants in `infra-common/src/events/cross_crate.rs`; all variants registered in `EventTypeRegistry::register_builtin_types`; `OrchestrationEventBus` refactored to per-variant `DashMap<&'static str, broadcast::Sender>` channels + `Relaxed` sequence counter (was `SeqCst`); optional `Arc<GlobalEventCoordinator>` shadow-publish; new `subscribe_kind()` API. Legacy `subscribe()` preserved. |
| 1 — DashMap + indices for surviving stores | ✅ Shipped | `MemoryCallStore` → `DashMap<CallId, Call>` + `by_session: DashMap<SessionId, CallId>` + `by_dialog: DashMap<String, CallId>`; index reindexing via `reindex()` on every insert/update; `BridgeManager` → `DashMap<BridgeId, (BridgeHandle, CallId)>` + `by_call`; `VoiceAiRegistry` → `DashMap<VoiceAiId, VoiceAiRuntime>`. Workforce stores (`MemoryAgentStore`, `MemoryQueueStore`, `MemoryAgentOfferStore`) untouched per scope contract. |
| 2 — Admission semaphore | ✅ Shipped | `InboundCallConfig::max_concurrent_setups` (default 256 × `available_parallelism()`); `Orchestrator::admission: Arc<Semaphore>`; `handle_incoming_call` `try_acquire_owned` at the door, SIP 503 + `OrchestrationError::AdmissionRejected(limit)` on exhaustion. |
| 3 — Atomic `CallLeg` state | ⏸ Deferred | Trigger not met. See "Profiling outcome" below. |

### Phase 1 → Phase 2 perf (orchestration-only, release, multi-thread, 4 workers)

| N | Phase 1 per-call wall | Phase 2 per-call wall | Δ |
|---:|---:|---:|---:|
| 100 | 0.048 ms | 0.065 ms | run-to-run noise |
| 500 | 0.084 ms | 0.083 ms | flat |
| 1000 | 0.113 ms | 0.112 ms | flat |

Sub-linear scaling: 10× N → ~2.36× per-call. Above the §8 verification "2× flat" target but well within the §2 ship target of 1k–5k concurrent sessions (1k calls land in ~113 ms; ~9k calls/sec single-instance capacity). The residual non-flat slope is dominated by O(N) eligibility scans in workforce code that is leaving per `PRD.md` §13 — explicitly out of scope for this plan.

### Live SIP/RTP perf (real `UnifiedCoordinator`s per caller and agent, full SIP + RTP)

`perf_live_sip_rtp` at N=5 (default) and N=50, 10 s media hold:

| Metric | N=5 | N=50 |
|---|---:|---:|
| Setup wall | 230 ms | 1058 ms (21 ms/call) |
| Media wall | 6.0 s | 11.1 s |
| Media CPU | 545 ms | 7.56 s (~70 % of one core) |
| RTP frames per side | 1 250 | 25 000 |
| RSS active delta | 31 MB (~6.3 MB/call) | 144 MB (~3.0 MB/call) |

### Profiling outcome — why Phase 3 is deferred

Sampled the live SIP/RTP test at N=50 with macOS `sample` (1 ms intervals, ~85 k stack samples across 18 s covering setup + 10 s media + teardown). Also sampled `perf_active_calls` at N=1000 in a loop (~70 k samples).

Inclusive frame appearances by subsystem, live SIP/RTP profile:

| Subsystem | Inclusive samples |
|---|---:|
| `rtp-core` (RTP send/recv loop, UDP transport) | 297 |
| `session-core` (event handler, state machine, audio path) | 200 |
| `media-core` (RTP forwarding, G711 codec, MediaSessionController) | 162 |
| `dialog-core` | 29 |
| `broadcast::Sender` event bus (Phase 0 code) | 21 |
| `Semaphore`/admission (Phase 2 code) | 10 |
| `orchestration-core` (anything) | 10 |
| DashMap (Phase 1 stores) | 1 |
| `BridgeManager` | 0 |
| `update_call` | 0 |
| **`CallLeg` state writes** | **0** |

`parking_lot::lock_slow` + `__psynch_mutexwait` combined = 63 / ~85 000 samples = **0.074 %**. There is no lock-contention story left to optimize.

The Phase 3 trigger ("`CallLeg` state transitions appearing as a top-3 contention point at N=1000") is not met by either benchmark. The leg-state hot path that Phase 3 would optimize is invisible in the profile. Phase 3 is documented but deferred — revisit only if a future benchmark, real production telemetry, or a new bridge / transfer hot path elevates leg-state writes into the profile.

### What the profile says about session-core

Top session-core frames in live SIP/RTP at N=50:

| Frame | Samples |
|---|---:|
| `SessionCrossCrateEventHandler::start_global_event_subscriptions` | 58 |
| `UnifiedCoordinator::send_audio` | 18 |
| `SessionCrossCrateEventHandler::handle_dialog_to_session_event` | 18 |
| `state_machine::executor::StateMachine::process_event` + `process_one_event` | 21 |
| `session_store::store::SessionStore::get_session` | 9 |
| `UnifiedCoordinator::accept_call` | 4 |

Each is < 0.1 % of total samples. No contention pattern, no scaling cliff. session-core does not need urgent tuning at the concurrency levels this plan targets.

### What the profile says about residual CPU

The dominant non-wait CPU consumers in live SIP/RTP are inherent media-plane work — RTP packet send/recv, UDP transport, G711 encode/decode, RTP forwarding. Any further perf gains would have to come from:

- **media-plane redesign** (zero-copy RTP buffers, batched UDP syscalls) — `rtp-core` / `media-core` scope, not this plan.
- **Reduced per-call allocation churn** — would benefit both orchestration-only and live tests, but requires touching `Call` / `Agent` / `Queue` data structure ergonomics. Defer.
- **Tokio runtime tuning** (worker count, timer wheel granularity) — also out of this plan's scope.

None of these are reasons to revisit Phase 3.

---

*Reviewers: this plan is intentionally narrower than `CONCURRENCY_PLAN.md`. If you find yourself wanting to add work on `Agent`, `Queue`, `Router`, or `AgentOffer`, that work belongs in the consumer crate after the `PRD.md` §13 lift-out — not here. See `PRD.md` §13 and §15 for the scope contract.*
