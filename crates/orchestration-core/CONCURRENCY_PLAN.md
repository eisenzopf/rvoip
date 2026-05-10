# Concurrency & Scalability Plan for `orchestration-core`

**Status:** Draft for review and iteration. No code has been changed.
**Goal:** Decide the architecture before writing code, not after.

---

## 1. Context & Goals

`orchestration-core` is the library for building unified call-center / PBX / IVR platforms on top of `session-core`. It must support three deployment shapes from the same codebase:

- **Human-only call center** — every call ends up at a human agent.
- **AI-only voice IVR** — every call is handled by an AI agent (LLM + STT + TTS).
- **Mixed** — calls may hit AI first, hand off to humans, or bounce; both kinds of agents serve the same queues.

A call also has a fourth disposition independent of agent type: **external SIP transfer** (bridge or REFER to a third-party URI).

**Key principle: AI agents and human agents are peer first-class citizens.** They share `AgentKind::VoiceAi` / `AgentKind::Human`, the same `Agent` struct, the same agent pool, the same queues, the same offer/accept lifecycle, the same skill-routing. The only behavioral split is at the leaves: human agents have `AgentConnector::Sip(...)`; AI agents have `AgentConnector::LocalVoiceAi(...)`. The orchestrator selects between them via the same `assign_next → offer → accept` flow (`examples/ai_only_queue.rs`, `examples/human_queue.rs`, `examples/mixed_ai_human_queue.rs`, `examples/ai_then_human_handoff.rs`).

All four destinations share one orchestration spine — the same `Call`, `Agent`, `Queue`, `Offer` stores, the same routing/assignment logic, the same event bus, the same `UnifiedCoordinator`. Fixing the spine fixes every deployment shape at once.

### Targets

- **Ship:** 1k–5k concurrent sessions on a single instance.
- **Architectural runway:** 10k+ concurrent sessions on a single instance, with a clear path to multi-instance later.
- **Storage:** in-memory now. The trait API must stay viable for Redis/SQL later, but a database does not fix lock contention — we fix the architecture first.

### Observed problem

`tests/perf_active_calls.rs` runs against orchestration-core only (no SIP/RTP) and shows **per-call setup time growing as N grows from 1 → 5 → 10 → 100 → 500**. With session-core's recent fix in commit `a8e41eb6` (priming missed events on subscribe in `events_for_session`), correctness is no longer the bottleneck. The residual N-scaling is structural in `orchestration-core`.

---

## 2. Current State Assessment

The slowdown is six compounding issues on the hot path. They are listed in order of impact.

### 2.1 Six locks across `.await` on every call setup

Every inbound call traverses this lock sequence in `Orchestrator::handle_incoming_call` and `offer_agent` (`src/orchestrator.rs:169`, `:620–671`):

```
calls.write        → insert_call
calls.read         → get_call
calls.write        → update_call (status set)
agents.write       → reserve_capacity (lock 1)
agents.write       → reserve_capacity (lock 2: reservations map)
agents.write       → update_state
offers.write       → insert_offer
calls.write        → update_call
```

Six to eight `tokio::sync::RwLock` acquisitions per call, all held across `await`. Under N concurrent setups, every task queues behind every other on these four locks. At N=500 this is the dominant cost, before any algorithmic O(N) work.

References: `src/store.rs:68–119`, `:178–228`, `:416–453`; `src/orchestrator.rs:169`, `:620–671`.

### 2.2 O(num_agents) scan inside `list_eligible_agents`

`MemoryAgentStore::list_eligible_agents` (`src/store.rs:155–171`) iterates `self.agents.read().await.values()` linearly under the read lock and clones every matching `Agent`. Called from `AssignmentManager::assign_next` (`src/assignment.rs:55–100`) for every assignment attempt, multiplied by the `preferred_kind` retry order. Reads are shared, so reads don't block reads — but every write to the agent store blocks all in-flight scans.

### 2.3 O(total_offers) scan with no `call_id` index

`MemoryAgentOfferStore::list_offers_for_call` (`src/store.rs:443–452`) walks every offer in the system to find the few belonging to one call. Called on every call status transition (e.g. `src/orchestrator.rs:1479`). With 500 active calls × ~3 offers each, that's ~1500 offers scanned every time any call advances state. This is a fundamental indexing flaw, not a tuning issue.

### 2.4 O(queued_calls) scan inside the queue write lock

`MemoryQueueStore::claim_for_agent` (`src/store.rs:357–383`) takes `calls.write().await`, runs `entries.retain(...)` for expiration (full scan), then `entries.iter().position(...)` for skill match (another full scan), all *while holding the write lock*. Every agent's claim against a queue serializes against every other.

### 2.5 `Ordering::SeqCst` on the event sequence counter

`OrchestrationEventBus::emit` (`src/events.rs:190`) does `self.sequence.fetch_add(1, Ordering::SeqCst)` on every emitted event. SeqCst is a global memory barrier across all CPU cores. The counter only needs uniqueness, not cross-thread ordering. With 3–5 events per setup × N concurrent setups, this is unnecessary global synchronization.

### 2.6 Single broadcast channel, slow-subscriber lag, fast event volume

`OrchestrationEventBus` (`src/events.rs:168–199`) has one `broadcast::Sender` for the whole orchestrator. Capacity defaults to 1024. Slow subscribers cause `RecvError::Lagged` and silent event loss; emit-side errors are ignored (`let _ = self.tx.send(...)`).

This bites AI-driven workloads especially hard. Every `apply_voice_ai_action` and every transcript pulse emits events (`VoiceAiTranscript`, `CallStatusChanged` to/from `InVoiceAi` — see `src/orchestrator.rs:1137–1212`). A pure human-to-human call generates ~5–10 lifecycle events; a streaming AI call generates dozens or hundreds. With a single 1024-deep channel, an AI-heavy deployment overflows in seconds.

### 2.7 Verdict

The current `orchestration-core` is a correct, well-organized prototype. It is **not suitable as-is** for thousands of concurrent sessions. None of the issues are deal-breakers individually; together they are why setup time grows with N. The design below addresses each.

---

## 3. Architectural Principles for the Redesign

These are the rules we apply to every component. If a proposed change violates one, we reconsider it.

1. **No locks held across `.await` on the call-setup path.** Critical sections must be short, synchronous, and use `parking_lot::Mutex` or atomics. Async work happens between locks, never inside them.
2. **Lock-free where DashMap fits.** Per-key contention only. We stop using `RwLock<HashMap<...>>` for any store.
3. **Indices, not scans.** Every "find by X" lookup we do more than once per call must be O(1) or O(matching). Linear scans on the hot path are bugs.
4. **One event bus for the whole platform.** Reuse `infra-common::events::GlobalEventCoordinator`. Don't build a second one.
5. **Atomic state machines for agents and reservations.** State transitions are CAS operations. Two tasks cannot accidentally reserve the same agent.
6. **Per-queue isolation.** Queues do not contend with each other for matching cycles. A backed-up queue does not slow other queues.
7. **Backpressure at the door, not in the middle.** Bound concurrency at admission via semaphore. Once admitted, calls do not collapse the runtime.
8. **Trait API stays stable for future Redis/SQL.** Nothing we do should require breaking the `CallStore` / `AgentStore` / `QueueStore` / `AgentOfferStore` traits. We can add streaming variants alongside the existing `Vec`-returning methods.

---

## 4. Detailed Design

The design is presented by subsystem, smallest blast radius first. Each subsystem can be reviewed and revised independently.

### 4.1 Event bus — adopt `infra-common::GlobalEventCoordinator`

**Decision:** Replace `orchestration-core/src/events.rs::OrchestrationEventBus` with `infra-common`'s `GlobalEventCoordinator`. Do not maintain a parallel bus.

**Why this is the right call:**

- **Already designed for the workload.** Lock-free publish path: `DashMap::entry().or_insert_with()` then non-blocking `broadcast::send()` (`infra-common/src/events/coordinator.rs:120–149`). No mutexes on the hot path.
- **Per-event-type channels.** Each event type ID has its own `broadcast::Sender<Arc<dyn CrossCrateEvent>>`. A slow consumer of `VoiceAiTranscript` does not lag a consumer of `CallStatusChanged`. The cross-crate registry already partitions session/dialog/media/rtp event families (`infra-common/src/events/cross_crate.rs`).
- **Cross-crate already wired.** `session-core` publishes `DialogToSessionEvent`, `RegistrationEvent`, etc. through this same coordinator (`session-core/src/adapters/dialog_adapter.rs:1665`, `session-core/src/api/unified.rs:1091`). Orchestration-core consuming the same bus means it can subscribe to session-level events directly without re-broadcasting.
- **Distributed runway.** The `EventBusAdapter` trait abstracts in-process vs distributed. NATS/gRPC transports are stubbed but not implemented; when we need multi-instance later, we swap the adapter without touching orchestrator code.
- **Throughput.** Documented in the bus README as ~2.2M events/sec with 5 subscribers on the zero-copy path. Plenty of headroom for AI-heavy event volume.

**What we register:**

Add an `OrchestrationEvent` family to `infra-common/src/events/cross_crate.rs::RvoipCrossCrateEvent`:

```rust
RvoipCrossCrateEvent::Orchestration(OrchestrationEvent)
```

with subtypes: `CallLifecycle` (created/queued/connected/ended), `AgentLifecycle` (state changes, reservations), `VoiceAi` (transcripts, TTS, dialog turns), `Bridge` (created/torn-down). Splitting these into subtype-aware event-type IDs lets a tracing subscriber filter to just `VoiceAi.Transcript` without catching `CallLifecycle` overhead.

**Sequence numbers:** `GlobalEventCoordinator` already provides ordering per event-type via the broadcast channel; we drop the explicit `AtomicU64::SeqCst` counter in `events.rs:190`. If callers need a monotonic per-orchestrator sequence for replay/audit, we add it as `Ordering::Relaxed` on a separate counter — that ordering is fine because the broadcast channel itself establishes happens-before.

**Backpressure / loss:** Inherits the bus's "drop oldest on slow subscriber" semantics. For audit/billing flows that must not lose events, the long-term answer is a persistent sink subscriber that writes to disk/DB. That's outside this plan; the architecture doesn't preclude it.

**Integration cost:** Small. `OrchestrationEventBus` is ~30 lines (`src/events.rs:168–199`) with a handful of `self.events.emit(...)` and `self.events.subscribe()` call sites in `src/orchestrator.rs` and tests. Plan: define the cross-crate enum variant, implement `CrossCrateEvent` for the wrapper, register in `EventTypeRegistry::register_builtin_types()`, swap the field on `Orchestrator`, run tests.

**Open question for review:** Do we want `Orchestrator::events()` to return a thin façade that calls into `GlobalEventCoordinator`, or do we expose `Arc<GlobalEventCoordinator>` directly to callers? Façade keeps API stable; direct exposure keeps things simple. *Recommendation: thin façade.*

### 4.2 State storage — DashMap + secondary indices

**Decision:** Replace all `RwLock<HashMap<K, V>>` in `src/store.rs` with `DashMap<K, V>`. Add the secondary indices we need for O(matching) lookups instead of O(N) scans.

**Stores:**

```rust
MemoryCallStore {
    calls: DashMap<CallId, Call>,
    by_session: DashMap<SessionId, CallId>,        // O(1) get_call_by_session
}

MemoryAgentStore {
    agents: DashMap<AgentId, Agent>,
    by_state: DashMap<AgentState, DashSet<AgentId>>,    // O(1) "all available agents"
    by_skill: DashMap<SkillId, DashSet<AgentId>>,       // O(1) "agents with skill X"
    by_queue: DashMap<QueueId, DashSet<AgentId>>,       // O(1) "agents serving queue Q"
    // No separate reservations map — reservation lives on Agent (see §4.3)
}

MemoryAgentOfferStore {
    offers: DashMap<AgentOfferId, AgentOffer>,
    by_call: DashMap<CallId, DashSet<AgentOfferId>>,    // O(offers-for-call) list_offers_for_call
    by_agent: DashMap<AgentId, DashSet<AgentOfferId>>,  // O(offers-for-agent)
}

MemoryQueueStore {
    queues: DashMap<QueueId, QueueState>,
    // QueueState owns the queue's worker; see §4.4
}
```

Index maintenance is bounded — a few `insert`/`remove` calls per state change. The cost is paid on writes, not on every read.

**`Agent.capacity.reserved_calls` becomes `AtomicU32`.** No lock to bump the counter. Combined with the atomic state machine in §4.3 this eliminates the agent store's contribution to lock contention.

### 4.3 Agent state machine — atomic CAS reservations

**The hard problem:** at 10k concurrent sessions with universal-worker agents (one agent serves multiple queues), two queues' matching loops will race to reserve the same agent at the same instant. We must guarantee exactly one wins, without holding a lock across the SIP / AI-runtime setup that follows.

**Decision:** Agent state is an `AtomicU8` on `Agent`. Reservation is a CAS.

```rust
pub struct Agent {
    pub id: AgentId,
    pub kind: AgentKind,
    pub connector: AgentConnector,
    pub skills: HashSet<SkillId>,
    pub queues: HashSet<QueueId>,
    pub state: AtomicU8,                  // AgentState::{Available, Reserved, Busy, Wrapup, Logout}
    pub reserved_for: AtomicCell<Option<(CallId, ReservationId)>>,
    pub capacity_used: AtomicU32,
    pub capacity_max: u32,
    // ...
}
```

**Reservation flow:**

```
1. compute candidates = by_state[Available] ∩ ⋂ by_skill[s] for s in required
2. for each candidate (ranked by selector):
     if agent.state.compare_exchange(Available, Reserved, AcqRel, Acquire).is_ok() {
         agent.reserved_for.store(Some((call_id, res_id)));
         update by_state index: remove from Available, add to Reserved
         emit AgentReserved event
         return Ok(reservation)
     }
   // CAS lost — another queue grabbed this agent. Try next candidate.
3. if exhausted: return None (call stays queued)
```

Two key properties:
- **Mutual exclusion** without a global lock. Only the CAS winner proceeds.
- **The actual SIP `make_call` / AI runtime startup happens *after* the CAS, outside any lock.** The agent is in the `Reserved` state during setup; if the agent rejects or times out, we CAS Reserved → Available.

**Capacity for multi-call agents** (some agents take multiple calls — typical for AI runtimes that can handle many sessions): instead of binary state, use `capacity_used.fetch_add(1, AcqRel)` and check < `capacity_max`. If over-incremented, decrement and retry. This is a counter-CAS pattern.

**Reservation timeout:** on reservation, we put the `ReservationId` in a `tokio_util::time::DelayQueue` keyed by expiry. A single per-orchestrator expiry task drains the DelayQueue and CAS-rolls Reserved → Available for any reservation that the agent didn't accept. No per-reservation timer task needed; one DelayQueue handles all of them.

**Why not keep a separate `reservations` map?** The current code's `reservations: RwLock<HashMap<ReservationId, ReservationRecord>>` doubles the lock surface and is redundant with `agent.reserved_for`. Drop it. If we ever need to enumerate reservations across agents (rare, e.g. for an admin UI), iterate `by_state[Reserved]`.

**Open question for review:** Does any agent legitimately have `capacity_max > 1` for human agents (call-waiting / multi-line phones)? If yes, we need richer per-agent state than `AtomicU8`. *Default assumption: humans have capacity_max=1, AI has capacity_max=N. Confirm.*

### 4.4 Queuing and routing — per-queue worker, push + pull, skill-aware

This is the section that most needs scrutiny. It is the heart of any call-center engine.

**Goals:**
- Match calls to agents in O(matching) time, not O(num_agents) or O(num_queued).
- Multiple queues do not block each other.
- An agent serving multiple queues can be reserved by exactly one of them at a time.
- Calls and agents arrive in any order; both events trigger matching.
- Fairness, priority, and skill matching are all first-class.

#### 4.4.1 Queue data structure

Each queue has:

```rust
pub struct QueueState {
    pub config: Queue,                                 // policy, skills, etc.
    pub waiting: parking_lot::Mutex<QueueWaitlist>,    // short critical sections only
    pub worker_tx: mpsc::UnboundedSender<MatchTrigger>,
    pub depth_gauge: AtomicU32,
}

pub struct QueueWaitlist {
    by_priority: BTreeMap<Priority, VecDeque<QueuedCall>>,
    by_call: HashMap<CallId, (Priority, /* index hint */)>,  // O(1) cancel
    expirations: BinaryHeap<(Expiry, CallId)>,
}
```

`BTreeMap<Priority, VecDeque>` gives us O(log P) priority operations and O(1) FIFO within priority. The `by_call` map gives O(1) cancellation. The `expirations` heap means we no longer scan to expire; we pop until the head is in the future.

`parking_lot::Mutex` is correct here: the critical section is a few map operations, never an `await`. Per-queue mutex means queues don't contend with each other.

#### 4.4.2 Per-queue matching worker

```rust
pub enum MatchTrigger {
    CallArrived,           // a call entered this queue
    AgentAvailable(AgentId),// an agent serving this queue went Available
    Tick,                  // periodic: re-attempt failed matches
}

async fn queue_worker(queue_id: QueueId, mut rx: mpsc::Receiver<MatchTrigger>, ...) {
    while let Some(_trigger) = rx.recv().await {
        // Coalesce: drain any further pending triggers in the channel (try_recv loop).
        while rx.try_recv().is_ok() {}
        // Run the matching cycle until no more matches are possible right now.
        loop {
            if !try_match_one(&queue_id).await { break; }
        }
    }
}
```

One task per queue. Triggers are debounced by draining the channel before re-running the match. The match cycle does at most one CAS per candidate per call — bounded work.

**Why per-queue tasks instead of a global matching loop:** isolation. A backed-up queue with thousands of waiting calls runs its own loop; other queues are not slowed. With 50 queues we have 50 lightweight tokio tasks, each idle until triggered.

#### 4.4.3 Push + pull triggers

Two events trigger matching:

- **Call enqueue (push):** `enqueue_call(call_id, queue_id)` inserts into the waitlist and sends `MatchTrigger::CallArrived` to the queue's worker.
- **Agent available (push, fan-out):** when `Agent.state` transitions to `Available`, look up `by_queue[agent.queues]` and send `MatchTrigger::AgentAvailable(agent.id)` to each queue's worker. First queue to CAS-win takes the agent; others see the CAS fail and skip.

There is no separate "pull" — `assign_next_call(queue_id)` for tests/ops becomes a thin wrapper that synchronously drains pending matches, equivalent to sending a trigger and awaiting the next idle.

**Race handling:** the agent-available signal goes to all subscribed queues at once. They race on the agent's CAS. The queue with the highest-priority waiting call that the agent can serve "should" win, but we don't enforce that strictly — fairness across queues is the responsibility of agent-pool design (skills, queue assignments). At single-orchestrator scale this is the right tradeoff.

#### 4.4.4 Skill matching — set intersection on indices

`AgentEligibilityRequest` already exists (`src/store.rs:155–171`); we just change the implementation.

```rust
fn list_candidates(req: &AgentEligibilityRequest) -> impl Iterator<Item = AgentId> {
    // Smallest skill set first (least intersection cost).
    let mut required: Vec<_> = req.required_skills.iter()
        .map(|s| store.by_skill.get(s))
        .collect();
    required.sort_by_key(|set| set.len());
    let candidates = intersect_sets(required);
    candidates
        .filter(|id| store.by_state.get(&AgentState::Available).map_or(false, |s| s.contains(id)))
        .filter(|id| !req.excluded_agent_ids.contains(id))
        .filter(|id| req.preferred_kind.map_or(true, |k| store.agents.get(id)?.kind == k))
}
```

Smallest-set-first intersection, lazy evaluation, no clones, no scans of irrelevant agents. The user-provided `selector: Arc<dyn AgentSelector>` still ranks the resulting candidates — that abstraction stays.

#### 4.4.5 Priority and fairness

Within a queue, priority is enforced by the `BTreeMap` ordering. Across queues, fairness is left to deployment configuration: an agent's `queues` set is its own preference, and the agent-available signal goes to all of them. If a deployment needs strict cross-queue fairness, that's a future feature (weighted fair queueing, queue priority levels).

**SLA / wait-time tracking:** each `QueuedCall` carries `enqueued_at`. The worker emits an `OrchestrationEvent::WaitTimeExceeded` when a call sits past its SLA. This is just an event — actual escalation policy is the application's job.

#### 4.4.6 Cancellation, requeue, transfers

- **Cancel:** caller hangs up while queued. `cancel_queued(call_id)` looks up `by_call[call_id]`, removes from the waitlist's VecDeque, decrements depth.
- **Requeue:** AI agent transfers to a different queue. `apply_voice_ai_action(VoiceAiAction::TransferToQueue { queue_id })` removes from current bridge, inserts into target queue.
- **Universal worker requeue:** a human's wrap-up timer fires → state → Available → fan-out signal to their queues.

All three are small, bounded operations on the per-queue mutex.

#### Open questions for review

1. **Per-queue task vs work-stealing pool.** Per-queue tasks are simple but use one tokio task per queue (cheap, but visible). Alternative: a fixed pool of N matcher tasks pulling from a shared queue-of-queues. Per-queue is simpler; I recommend starting there.
2. **Fairness across queues sharing agents.** Today (and proposed): first-come-first-served at the agent CAS. Acceptable? If a deployment has VIP and Standard queues sharing agents, do we need explicit priority?
3. **Cross-queue priority.** Is "queue A's priority-9 call beats queue B's priority-5 call when both want the same agent" a requirement? If yes, agents need a global priority comparator across the queues they serve, which complicates the trigger model.
4. **AI agent capacity.** A single AI runtime may serve many concurrent calls. Is `capacity_max` per agent or per runtime? If per runtime, multiple `Agent` records sharing one `VoiceAiId` need to share capacity.

### 4.5 Pluggable routing & matching algorithms

A real call center stands or falls on its routing logic. The runtime in §4.4 makes matching fast; this section makes matching **expressive**. The goal is for an integrator to pick the right algorithm — or chain several together — based on skill, customer, time, history, and business rules, without forking the crate.

The good news: the foundations are already in place in `src/traits.rs`. We have `Router`, `QueueSelector`, and `ContactResolver`. We extend them with a few more decision points and ship a catalog of built-in implementations so the common cases are turn-key.

#### 4.5.1 Decision points in a call's life

A call passes through up to seven decision points. Each is a pluggable trait. Each has a sane default so the simple case stays simple.

| # | Decision point | Trait | Today | Plan |
|---|---|---|---|---|
| 1 | **Pre-route enrichment** — fetch CRM/account data before routing | `CallEnricher` | (none) | New trait, optional, default `NoopEnricher` |
| 2 | **Route** — call → destination (queue / agent / SIP / reject) | `Router` | exists | Keep, expand catalog, support composition |
| 3 | **Queue admission** — accept, overflow, callback, reject | `QueueAdmissionPolicy` | implicit | New trait; default `AcceptOrRejectFull` |
| 4 | **Eligibility** — which agents can take this call | `EligibilityFilter` | hard-coded in `list_eligible_agents` | Pluggable filter, default = current skill+state |
| 5 | **Selection** — among eligibles, who wins | `QueueSelector` | exists (`FirstAvailableSelector`) | Keep, expand catalog, allow per-queue selector |
| 6 | **Queue ordering** — how calls are ranked in the queue | `QueueOrdering` | implicit FIFO | New trait; defaults FIFO, priority, EWT-aware |
| 7 | **Overflow** — what to do when no match available | `OverflowPolicy` | (none) | New trait; default `WaitInQueue` |

All seven traits live in `src/traits.rs` next to existing ones. Each is `async_trait`, `Send + Sync`, returns `Result<...>`. None of them block on locks; they consume immutable inputs and return decisions.

#### 4.5.2 Trait surface (proposed signatures)

The new traits, sketched. Existing traits unchanged in shape.

```rust
// (1) Pre-route enrichment — let CRM/IVR/AI data attach to the call before routing.
#[async_trait]
pub trait CallEnricher: Send + Sync {
    async fn enrich(&self, request: &mut RouteRequest) -> Result<()>;
}

// RouteRequest already carries `metadata: HashMap<String, String>`. We add structured
// fields for the common cases so a CrmEnricher writes once, every router can read.
pub struct CallContext {
    pub vip_tier: Option<u8>,
    pub language: Option<String>,
    pub account_id: Option<String>,
    pub last_agent_id: Option<AgentId>,    // for sticky routing
    pub intent: Option<String>,            // from upstream IVR
    pub customer_value_score: Option<f32>, // for value-based routing
    pub custom: HashMap<String, String>,   // free-form
}

// (3) Queue admission — examine queue depth, EWT, business rules; accept or divert.
#[async_trait]
pub trait QueueAdmissionPolicy: Send + Sync {
    async fn admit(&self, ctx: AdmissionContext) -> Result<AdmissionDecision>;
}
pub enum AdmissionDecision {
    Admit,
    Overflow { to: QueueId },
    OfferCallback,
    Reject { reason: String },
}

// (4) Eligibility — replaces hard-coded skill/state filters in list_eligible_agents.
#[async_trait]
pub trait EligibilityFilter: Send + Sync {
    async fn is_eligible(&self, agent: &Agent, call: &QueuedCall, ctx: &EligibilityContext) -> bool;
}

// (6) Queue ordering — pluggable comparator that decides the next call to match.
#[async_trait]
pub trait QueueOrdering: Send + Sync {
    fn next(&self, waiting: &QueueWaitlist, now: DateTime<Utc>) -> Option<&QueuedCall>;
}

// (7) Overflow — invoked when the queue worker exhausts candidates.
#[async_trait]
pub trait OverflowPolicy: Send + Sync {
    async fn on_no_match(&self, call: &QueuedCall, queue: &Queue) -> Result<OverflowAction>;
}
pub enum OverflowAction {
    Continue,
    MoveToQueue(QueueId),
    OfferToAgent(AgentId),
    DialSipUri(String),
    OfferCallback,
    Hangup { reason: String },
}
```

These are the seams. Anything more exotic (predictive routing, ML-based pairing) is implemented by a customer's own type that satisfies one of these traits.

#### 4.5.3 Built-in algorithm catalog

We ship a library of common implementations under `crate::routing::*`. Picking from this list should cover ~90% of deployments without custom code.

**Routers (`crate::routing::routers`):**

| Algorithm | Use case |
|---|---|
| `StaticRouter` *(exists)* | Always returns the same decision; for tests / single-purpose IVR |
| `MapByDnisRouter` | Route by called number (DID) — typical for multi-tenant IVR |
| `IvrSelectionRouter` | Route by IVR digit / intent collected upstream |
| `LanguageRouter` | Route by detected/declared language |
| `BusinessHoursRouter` | Time-based with fallback (after-hours → voicemail/AI) |
| `CrmDataRouter` | Use enriched `CallContext` (VIP tier, account, intent) to pick queue/agent |
| `CompositeRouter` | Chain of routers; first non-fallthrough wins |
| `PercentageAllocationRouter` | Split by percentage (A/B testing) |
| `FallbackRouter` | Try primary; on failure try secondary |

**Selectors (`crate::routing::selectors`):**

| Algorithm | Use case |
|---|---|
| `FirstAvailableSelector` *(exists)* | Simple FIFO match — default |
| `LongestIdleSelector` | Agent-fairness — agent who's been idle longest wins |
| `RoundRobinSelector` | Even distribution across the candidate set |
| `RandomSelector` | Pure random (good baseline; eliminates ordering bias) |
| `LeastBusySelector` | Lowest `capacity_used / capacity_max` ratio (load balancing) |
| `HighestSkillMatchSelector` | Best-fit on skill proficiency levels |
| `StickyAgentSelector` | Return caller to `CallContext.last_agent_id` if available, else fall back |
| `PerformanceSelector` | Highest performance score (CSAT, AHT) — feed scores via `Agent.metrics` |
| `WeightedScoreSelector` | User-supplied function `Fn(&Agent, &QueuedCall) -> f64`; pick max |
| `KindPreferenceSelector` | Prefer `AgentKind::Human` first, fall through to `AgentKind::VoiceAi` (or vice versa) |
| `FallbackSelector` | Try primary selector, fall through to secondary if no match |

**Queue orderings (`crate::routing::ordering`):**

| Algorithm | Use case |
|---|---|
| `FifoOrdering` | Default — head of `BTreeMap<Priority, VecDeque>` |
| `PriorityOrdering` | Strict priority levels, FIFO within priority |
| `WeightedFairOrdering` | Fair share across priority bands (avoid starvation) |
| `SlaPriorityOrdering` | Bump calls approaching SLA breach to head |
| `CallerValueOrdering` | Sort by `CallContext.customer_value_score` |

**Admission policies (`crate::routing::admission`):**

| Algorithm | Use case |
|---|---|
| `AcceptOrRejectFull` | Default — admit until depth limit, then 503 |
| `OverflowToQueue` | Spill to alternate queue when full |
| `EwtBasedAdmission` | Reject if estimated wait time > threshold |
| `BusinessHoursAdmission` | After-hours → callback or AI |

**Overflow policies (`crate::routing::overflow`):**

| Algorithm | Use case |
|---|---|
| `WaitInQueue` | Default — keep waiting |
| `MoveToQueueAfter(Duration)` | Promote to alt queue if wait exceeds threshold |
| `OfferCallbackAfter(Duration)` | Capture number, hang up, callback when agent free |
| `FailoverToAi` | If no human available, route to AI agent |
| `FailoverToSip` | Forward externally |

This catalog is intentionally generous. Implementations are small (most under 50 lines) and mechanical; what matters is having clean trait boundaries so customers don't have to fork.

#### 4.5.4 Customer-aware routing — plumbing data through

The `RouteRequest.metadata: HashMap<String, String>` field already exists (`src/traits.rs:23`). Today it's free-form. We promote the common keys to a structured `CallContext` (sketched in §4.5.2):

```rust
pub struct RouteRequest {
    pub call_id: CallId,
    pub from: String,
    pub to: String,
    pub sip_call_id: Option<String>,
    pub caller_identity: CallerIdentity,
    pub priority: CallPriority,
    pub context: CallContext,                 // NEW — typed, shared across decision points
    pub metadata: HashMap<String, String>,    // remains for free-form
}
```

`CallContext` is filled by the `CallEnricher` (decision point 1). A typical chain:

```
SIP INVITE arrives
  → CrmEnricher looks up the caller ID, fills vip_tier, account_id, last_agent_id
  → IvrEnricher (if pre-IVR) fills intent
  → Router reads context.vip_tier, picks vip-queue
  → AdmissionPolicy reads context.customer_value_score, accepts even if queue is over depth
  → Selector reads context.last_agent_id, picks sticky agent
```

`CallContext` is read-only after enrichment. Decision points downstream read but do not mutate it. This keeps the data flow predictable.

#### 4.5.5 Composition patterns

The catalog stays small because composition does the heavy lifting.

```rust
// Language-aware, VIP-prioritized router with after-hours fallback.
let router = CompositeRouter::new()
    .first(BusinessHoursRouter::new(business_hours, after_hours_fallback))
    .then(VipRouter::new(vip_queue))
    .then(LanguageRouter::new(language_to_queue_map))
    .fallback(StaticRouter::new(RouteDecision::Queue { queue_id: default_queue }));

// Sticky agent with longest-idle fallback, preferring humans.
let selector = FallbackSelector::new(
    StickyAgentSelector::new(),
    KindPreferenceSelector::new(AgentKind::Human, LongestIdleSelector::new()),
);

// SLA-driven queue with weighted fair ordering across two priority bands.
let ordering = SlaPriorityOrdering::wrap(WeightedFairOrdering::new(weights));
```

The trait shapes are small enough that wrapping/chaining is straightforward.

#### 4.5.6 Per-queue configuration

Today `AssignmentManager` holds a single global `selector` (`src/assignment.rs:25, 50`). Real call centers configure per-queue policy: VIP queue uses `HighestSkillMatchSelector`, support queue uses `LongestIdleSelector`, etc.

We move policy to `Queue`:

```rust
pub struct Queue {
    pub id: QueueId,
    pub name: String,
    pub required_skills: HashSet<SkillId>,
    pub policy: QueuePolicy,
    pub selector: Option<Arc<dyn QueueSelector>>,        // None → orchestrator default
    pub ordering: Option<Arc<dyn QueueOrdering>>,        // None → FIFO
    pub admission: Option<Arc<dyn QueueAdmissionPolicy>>,// None → AcceptOrRejectFull
    pub overflow: Option<Arc<dyn OverflowPolicy>>,       // None → WaitInQueue
    pub eligibility: Option<Arc<dyn EligibilityFilter>>, // None → default skill+state
}
```

The orchestrator builder accepts default implementations; per-queue overrides take precedence. This is the lever an integrator pulls to shape behavior per business unit, time-of-day, or product line.

#### 4.5.7 Configuration & hot-reload

Routing policy changes more often than code. Two postures, both worth supporting:

- **Static / typed (Rust)** — set selectors and ordering at build time on the `OrchestratorBuilder`. Fast, type-safe, no surprises.
- **Configured (TOML/YAML/JSON)** — a `RoutingConfig` deserializes into trait objects via a registry of named implementations (`"longest_idle"` → `LongestIdleSelector`). Lets ops change weights and percentages without recompiling.

Hot-reload is out of scope for this iteration but the trait-object indirection (`Arc<dyn QueueSelector>`) means a swap-in-place is mechanically simple later (e.g., via `arc-swap`).

#### 4.5.8 What this is *not*

To keep the scope honest:

- **Not** a full predictive-routing / ML-pairing engine (Afiniti-style behavioral pairing). The trait surface is an extension point; the actual ML model is a customer concern.
- **Not** an outbound dialer. Predictive/preview/progressive dialing has its own state machine. Out of scope here.
- **Not** a forecasting / WFM module. Capacity planning belongs upstream.

These are deliberately out of scope so the routing surface stays focused and the work is finishable.

#### 4.5.9 Open questions for review

1. **Should `EligibilityFilter` replace or layer on top of `list_eligible_agents`?** Replacing it gives one canonical extension point; layering keeps the index-based fast path. *My recommendation: layer — index-based filter narrows to candidates, then `EligibilityFilter::is_eligible` runs per candidate for the slow/custom checks.*

2. **Per-call selector override.** Should `RouteDecision::Queue` allow a one-shot selector override (e.g., "this call is from a VIP, force `HighestSkillMatchSelector`")? *My recommendation: yes — it's a small change to add an optional `Arc<dyn QueueSelector>` to the variant, and it preserves per-queue defaults.*

3. **Trait registry for config-driven setup.** Worth shipping a "named factory" registry (`"longest_idle" → impl QueueSelector`)? *My recommendation: yes for the built-in catalog only; custom types must be registered by the integrator.*

4. **Where does `CallContext` get populated for outbound calls?** The current model is inbound-driven. For predictive dialer / outbound IVR, who fills `last_agent_id`, `account_id`? *Open — depends on whether we ship an outbound module later.*

5. **Sticky agent fallback timeout.** If `StickyAgentSelector` finds the last agent but they're not Available, how long do we wait before falling through? Per-call, per-queue, or global? *My recommendation: per-queue, configurable, default 0 (immediate fallback).*

### 4.6 Reservation lifecycle

Bringing §4.3 and §4.4 together, the reservation lifecycle is:

```
Available --(CAS by queue worker)--> Reserved --(offer accepted)--> Busy --(call ends)--> Wrapup --(timer)--> Available
                                          |
                                          +--(offer rejected/timeout)--> Available
```

Each transition is a single CAS on `Agent.state`. Each transition emits an event on the GlobalEventCoordinator. The DelayQueue handles reservation timeout in one place.

This eliminates the current dual-write (`agents.write` + `reservations.write`) hazard at `src/store.rs:178–228`.

### 4.6 Backpressure and admission control

**Per-orchestrator semaphore on call setup.** `Orchestrator` holds an `Arc<Semaphore>` sized to `config.max_concurrent_setups` (default: a function of CPU count, e.g. 1024). The incoming-call handler acquires a permit before doing any work; if the semaphore is exhausted, the call is rejected at the door (SIP 503 / equivalent) instead of joining a death spiral inside the orchestrator.

**Per-queue depth limit.** `QueueState.depth_gauge` is bounded by `config.max_queue_depth`. New calls beyond the limit are rejected with `OrchestrationError::QueueFull` (or routed to overflow per policy). Already partially supported; we make it strict.

**Event bus capacity.** Per-event-type capacity in `GlobalEventCoordinator` defaults to 10,000 (`infra-common/src/events/bus.rs:37`). For AI-heavy deployments, raise transcript-event capacity specifically, leave call-lifecycle at default. Sized per type, not globally.

### 4.7 Trait API stability

We do not break the `CallStore`, `AgentStore`, `QueueStore`, `AgentOfferStore` traits in this plan. Internal implementation changes only.

For the future Redis/SQL backend, we will *add* (not replace) streaming variants:

```rust
fn list_eligible_agents_stream(&self, req: AgentEligibilityRequest)
    -> Pin<Box<dyn Stream<Item = Result<Agent>>>>;
```

so a remote backend can apply filters server-side. In-memory implementations satisfy the streaming variant by yielding from the indices. This is out of scope for the in-memory-first work but it is what we keep in mind so we don't paint ourselves into a corner.

---

## 5. Phases

The work is split so that each phase is independently mergeable, leaves the crate in a working state, and produces measurable improvement.

### Phase 0 — Adopt GlobalEventCoordinator (small, isolated)

- Define `RvoipCrossCrateEvent::Orchestration(OrchestrationEvent)` in `infra-common/src/events/cross_crate.rs`.
- Implement `CrossCrateEvent` for the wrapper.
- Register in `EventTypeRegistry::register_builtin_types()`.
- Replace `OrchestrationEventBus` field with a façade over `Arc<GlobalEventCoordinator>` in `Orchestrator`.
- Drop `events.rs::SeqCst` counter (use Relaxed if we still want a sequence number).
- All existing tests still pass.

**Expected impact:** Removes issues 2.5 and 2.6. Eliminates one custom event bus from the crate.

### Phase 1 — DashMap + indices (the meat)

- Replace `RwLock<HashMap<...>>` in all four stores with `DashMap`.
- Add secondary indices: `by_session`, `by_state`, `by_skill`, `by_queue`, `by_call`, `by_agent`.
- Index maintenance on every state-change path.
- Make `Agent.capacity_used` an `AtomicU32`.
- All existing tests still pass.

**Expected impact:** Removes issue 2.1 (locks across `.await`) and 2.2/2.3 (O(N) scans).

### Phase 2 — Atomic agent state machine + reservation rework

- Add `AtomicU8` state to `Agent`.
- Replace `reserve_capacity` / `activate_capacity` with CAS-based transitions.
- Drop the separate `reservations` map.
- DelayQueue-based reservation timeout.
- All existing tests still pass.

**Expected impact:** Removes the agent-store double-lock hazard. Two queues racing for the same agent are correct without holding any lock.

### Phase 3 — Per-queue workers + skill-indexed matching

- Add `QueueWaitlist` with `BTreeMap<Priority, VecDeque>` + `BinaryHeap` expirations.
- Spawn a `queue_worker` per queue, fed by `mpsc::UnboundedSender<MatchTrigger>`.
- Trigger on enqueue and on agent-available (fan-out).
- Reimplement `list_eligible_agents` as smallest-set-first index intersection.
- Removes issue 2.4 (queue-write lock scans).
- Existing API of `assign_next_call` becomes a wrapper that drains pending matches.

**Expected impact:** Per-queue isolation; matching is O(matching candidates), not O(all agents) × O(all queued).

### Phase 4 — Backpressure

- Per-orchestrator setup-admission semaphore.
- Strict per-queue depth limits with overflow event.
- Per-event-type capacity tuning (raise transcript channels for AI-heavy use).

**Expected impact:** A burst of 10k concurrent INVITEs degrades cleanly (some rejected with 503) instead of hanging.

### Phase 5+ — Distributed / external state (future, not now)

Listed for runway, not for this iteration:

- `RedisCallStore` / `RedisAgentStore` implementing the streaming trait variants.
- Lua scripts for atomic cross-orchestrator agent CAS.
- Consistent-hash sharding of orchestrator instances by tenant or call-id.
- Distributed transports for `GlobalEventCoordinator` (NATS or gRPC; already abstracted, just need real implementations).

These come **after** Phases 0–4 demonstrate flat per-call latency at 10k on a single instance. A database does not fix lock contention.

---

## 6. Verification

End-to-end tests, run after each phase. Each must pass before advancing.

1. **Build & unit tests:** `cargo build -p orchestration-core` and `cargo test -p orchestration-core` — clean and green.
2. **Perf — orchestration-only:** `cargo test -p orchestration-core --test perf_active_calls --release` for N ∈ {1, 5, 10, 100, 500, 1000}. Per-call wall time (`wall_ms / active_calls`) must be flat to within 2× across the range. We capture this number per phase as a regression baseline.
3. **Perf — live SIP/RTP:** `RVOIP_LIVE_SIP_RTP_COUNTS=1,5,10,50 cargo test -p orchestration-core --test perf_live_sip_rtp --release` — no regression vs. current numbers; faster at higher N.
4. **Cross-crate integration:** `cargo test -p session-core` after the GlobalEventCoordinator change. Confirms orchestration-core's event publication doesn't break session-core's own subscribers.
5. **Examples:**
   - `cargo run -p orchestration-core --example human_queue`
   - `cargo run -p orchestration-core --example ai_only_queue`
   - `cargo run -p orchestration-core --example ai_then_human_handoff`
   - `cargo run -p orchestration-core --example mixed_ai_human_queue`
   - `cargo run -p orchestration-core --example speech_ivr`
   - `cargo run -p orchestration-core --example registered_sip_agent`

   All complete without errors. These exercise the three deployment shapes.
6. **Tokio profiling for Phase 4:** `tokio-console` against the perf test at N=1000 to confirm no task is starved and no single mutex appears as a top contention point.

---

## 7. Critical Files

The list of files this plan touches, by phase. None of these files need to break their public API.

| Phase | File | Change |
|---|---|---|
| 0 | `infra-common/src/events/cross_crate.rs` | Add `Orchestration` variant |
| 0 | `infra-common/src/events/coordinator.rs` | Register orchestration event types |
| 0 | `orchestration-core/src/events.rs` | Replace bus with façade over GlobalEventCoordinator |
| 0 | `orchestration-core/src/orchestrator.rs` | `events:` field swap; drop SeqCst |
| 1 | `orchestration-core/src/store.rs` | DashMap, secondary indices |
| 1 | `orchestration-core/src/types.rs` | `Agent.capacity_used: AtomicU32` |
| 2 | `orchestration-core/src/types.rs` | `Agent.state: AtomicU8` |
| 2 | `orchestration-core/src/store.rs` | CAS-based reserve/activate; drop reservations map |
| 2 | `orchestration-core/src/orchestrator.rs` | Reservation lifecycle uses CAS |
| 3 | `orchestration-core/src/store.rs` | `QueueWaitlist`, expirations heap |
| 3 | `orchestration-core/src/assignment.rs` | Per-queue workers, index-aware matching |
| 3 | `orchestration-core/src/orchestrator.rs` | Trigger fan-out on agent-available |
| 4 | `orchestration-core/src/orchestrator.rs` | Admission semaphore |
| 4 | `orchestration-core/src/config.rs` | `max_concurrent_setups`, queue depth limits |
| 0–4 | `orchestration-core/Cargo.toml` | Add `dashmap`, `parking_lot`, `crossbeam-utils`, `tokio-util` (some likely already transitive) |

---

## 8. Open Questions for Review

These are decisions where I have a recommendation but want explicit alignment before code is written.

1. **Event bus façade vs direct exposure.** Should `Orchestrator::events()` return a thin façade or `Arc<GlobalEventCoordinator>` directly?
   *My recommendation: thin façade so we can evolve the bus without API churn.*

2. **Agent capacity semantics.** Do human agents ever have `capacity_max > 1`? If yes, the atomic state machine in §4.3 needs a counter, not a binary state.
   *My default: humans = 1, AI = N.*

3. **Cross-queue fairness for shared agents.** If a VIP queue and a Standard queue share agents, do we need strict priority across queues? My current design is FCFS at the agent CAS.
   *My recommendation: ship FCFS first; revisit if real workloads need it.*

4. **AI runtime capacity sharing.** Multiple `Agent` records may share one `VoiceAiId` (one runtime, multiple "agent" personas). Should capacity be per-agent or per-runtime?
   *Open — depends on how customers want to model AI agents.*

5. **Queue worker model.** One tokio task per queue (simple, scales to ~thousands of queues) vs. a worker pool (more complex, better at very high queue counts).
   *My recommendation: per-queue task, until we have evidence we need otherwise.*

6. **Phase ordering.** Phase 0 (event bus) is small and isolated; Phase 1 (DashMap) is the highest-impact lift; Phase 2 (atomic state) and Phase 3 (per-queue workers) are bigger. Is this the right order, or should we attempt Phase 1 + Phase 2 together since they touch similar files?
   *My recommendation: 0 → 1 → 2 → 3 → 4, with each phase independently mergeable.*

7. **Test strategy for new perf characteristics.** Do we want a perf regression check in CI (e.g., a threshold on `wall_ms / active_calls` at N=500), or keep perf as a manual gate?
   *Open — depends on CI budget.*

---

*Reviewers: please mark up sections inline, especially §4.3 (atomic state) and §4.4 (queuing/routing), which carry the most novel design and the biggest risk.*
