# Carve rvoip-sip + rvoip-core skeleton

**Status:** Planning. Not yet started. Companion to `PRD.md`, `INTERFACE_DESIGN.md`, and `PERFORMANCE_PLAN.md`.

**Scope:** PRD §13.3 steps 1–2 only. Workforce deletion (step 7), `rvoip-uctp` (step 3), `rvoip-quic`/`rvoip-webtransport` (step 4), `rvoip-webrtc` (step 5), `rvoip-harness` extraction (step 6) are out of scope for this PR series.

---

## 1. Context

Phases 1+2 of `PERFORMANCE_PLAN.md` shipped on 2026-05-10 (DashMap stores + secondary indices, admission semaphore, `GlobalEventCoordinator` adoption). Phase 3 was deferred (profiling proved the trigger condition wasn't met). The crate is hardened enough for the next move: turning `orchestration-core` into the future `rvoip` workspace by **renaming `session-core` to `rvoip-sip`, adding a SIP B2BUA/server surface to it, and creating a transport-agnostic `rvoip-core` skeleton.**

Why now and why this shape:

- `session-core`'s public `api/*` (`UnifiedCoordinator`, `StreamPeer`, `CallbackPeer`, `Endpoint`, `SessionHandle`, `RegistrationHandle`) is **battle-tested against Asterisk and FreeSWITCH** as both a SIP softphone client and a partial B2BUA. We do not re-implement it. We **rename the crate**, keep `api/*` public unchanged, and add a `server/*` module on top for the gateway-only helpers (bridge registry, AOR contact resolution, REFER/attended-transfer mechanics) currently embedded inside `orchestration-core::orchestrator`. This matches PRD §13 (`session-core` is "absorbed into `rvoip-sip`") and `INTERFACE_DESIGN.md` §15.3.
- `rvoip-core` exists to host the **transport-agnostic spine** so future `rvoip-webrtc`, `rvoip-quic`, `rvoip-webtransport` adapters slot in without reshaping `rvoip-sip`. Per `INTERFACE_DESIGN.md` §7.5 it is intentionally small — a few hundred lines of types, traits, and dispatch. The spine carries: voip-3 vocabulary types (`Conversation`, `Session`, `Connection`, `Stream`, `Message`, `Participant`), the `ConnectionAdapter` trait, the cross-transport `BridgeConnections` primitive, the `Orchestrator` entry point, and trait surfaces for identity / persistence.
- **Routing and workforce orchestration stay above rvoip** in Thelve (the canonical consumer per `~/Developer/Rudeless/docs/thelve-interaction-plane-architecture.md`). rvoip exposes the *hooks* (commands: `RouteInboundConnection`, `OriginateConnection`, `BridgeConnections`, `TransferConnection`; events: `ConnectionInbound`, `ConnectionConnected`, `RegistrationChanged`); Thelve writes the *policy* (who answers, how to queue, capacity, customer continuity).
- **Workforce code (`Agent` / `Queue` / `Router` / `AgentOffer` / `AssignmentManager`) is NOT touched in this PR series.** It stays in `orchestration-core` per PRD §13 step 7 and is deleted in a separate cutover. Same for the queue-themed examples and `developer_workflows.rs` — they get a `#[deprecated]` annotation and keep running.

---

## 2. Layering rule (critical — read first)

`api::UnifiedCoordinator` (renamed from `session-core::api::UnifiedCoordinator`) is the **only** surface that talks to `dialog-core` and `media-core`. It already does this and is the FreeSWITCH/Asterisk-validated code path. We do NOT add a second access path to those crates.

```
rvoip-core::Orchestrator           (cross-transport command surface)
        │
        ▼
rvoip-sip::adapter::SipAdapter     (impl rvoip-core::ConnectionAdapter)
        │
        ▼
rvoip-sip::api::UnifiedCoordinator (renamed session-core; battle-tested)
        │
        ├──► dialog-core            (signaling)
        └──► media-core             (RTP, BridgeHandle)

rvoip-sip::server::*               (B2BUA helpers — also calls api::UnifiedCoordinator,
                                    NOT dialog-core/media-core directly)
```

That means:

- `rvoip-sip::adapter::SipAdapter` (the new `ConnectionAdapter` impl) calls `api::UnifiedCoordinator::make_call`, `accept_call`, `hangup`, `bridge`, `send_refer`, `register`, etc. It does not call `dialog-core` or `media-core` directly.
- `rvoip-sip::server::*` (the new B2BUA helpers) calls `api::UnifiedCoordinator` for the actual SIP/RTP work. The "server" module adds *coordination* on top of the api surface — it does not re-implement what api/ already does. Specifically:
  - `server::bridge` creates SIP-to-SIP bridges by calling `api::UnifiedCoordinator::bridge(session_a, session_b)` — which already returns a `media-core::BridgeHandle`. We do not wrap or re-implement `media-core::BridgeHandle`. The "server" module's contribution is the surrounding lifecycle (bridge ID assignment, call-to-bridge index, teardown watch) and the `SipBridgeStrategy` adapter so `rvoip-core::Orchestrator` can invoke this fast path.
  - `server::contact_resolver` is pure SIP-AOR-to-Contact-URI lookup against `registrar-core`. No dialog-core or media-core.
  - `server::transfer` calls `api::UnifiedCoordinator::send_refer` / `transfer_attended` / `accept_refer`. No dialog-core directly.
  - `server::b2bua` (optional convenience) is pattern glue — receive INVITE event → call `api::UnifiedCoordinator::accept_call` → call `api::UnifiedCoordinator::make_call` for the outbound leg → call `api::UnifiedCoordinator::bridge` to join them. All through api/.
- `rvoip-core` has zero knowledge of `dialog-core` or `media-core`. It depends on `rvoip-media` (later) for transcoding primitives but not on the SIP-specific media plumbing.

Why this rule: bypassing `UnifiedCoordinator` to reach `dialog-core` or `media-core` directly would re-introduce a path we already proved out and tested. Two paths means two state machines that can drift. The proven path is the only path.

---

## 3. Decision matrix — what goes where

| Concern | Lives in | Rationale |
|---|---|---|
| **Cross-transport `BridgeConnections(a, b)` command + handle** | `rvoip-core` | Per `INTERFACE_DESIGN.md` §10.2: pumps frames between two `MediaStream`s; transport-agnostic; transcoder inserted by `rvoip-media` when codecs differ. Returns `rvoip-core::BridgeHandle`. For the SIP-only same-codec case, the Orchestrator dispatches to `rvoip-sip::server::SipBridgeStrategy` (which calls `api::UnifiedCoordinator::bridge` underneath) instead of the generic frame-pump. |
| **SIP↔SIP same-codec bridge (RTP-direct fast path)** | `api::UnifiedCoordinator::bridge()` (unchanged), invoked via `rvoip-sip::server::SipBridgeStrategy` | Already exists and is proven. `api::UnifiedCoordinator::bridge(session_a, session_b)` returns a `media-core::BridgeHandle`. The new `SipBridgeStrategy` is just a thin function that the Orchestrator calls when both Connections are SIP and codecs match. No media-core wrapping happens in rvoip-sip. |
| **`BridgeManager` (registry of active bridges)** | `rvoip-core` | Tracks bridges regardless of transport. Today lives at `crates/orchestration-core/src/orchestrator.rs:25` with `DashMap<BridgeId, (BridgeHandle, CallId)>` + `by_call` index (Phase 1 work). The DashMap pattern carries forward unchanged. **In this PR series** the stored handle stays as `media_core::BridgeHandle` — current shape, just relocated. A `BridgeKind` enum (to discriminate SIP fast-path vs. cross-transport bridge) is deferred until cross-transport bridging actually lands. Avoids introducing a one-variant enum. |
| **Routing decisions ("who takes this call")** | Thelve (out of rvoip) | PRD §13: `Router`/`RouteRequest`/`RouteDecision`/`QueueSelector` deleted from rvoip in step 7. rvoip emits `ConnectionInbound` with all the SIP-layer facts Thelve needs (From/To/PAI/Diversion); Thelve issues `RouteInboundConnection { connection_id, action: Accept/Reject/Bridge/Originate-and-bridge }`. |
| **SIP-layer routing hooks** | `rvoip-sip::server` | Thin pieces of "routing" that are SIP-protocol-level concerns: SIP URI parsing, `ContactResolver`/`RegistrarContactResolver` (AOR → live Contact URI), `ContactSource` metadata. Lifted from `crates/orchestration-core/src/traits.rs:122`. |
| **B2BUA transfer orchestration (which leg to REFER, target URI build, blind vs attended, transferor session linkage)** | `rvoip-sip::server::transfer` | The actual SIP REFER mechanics already live in `api::UnifiedCoordinator` (`send_refer`, `accept_refer`, `reject_refer`, `send_refer_with_replaces`, `make_transfer_leg`, `set_transferor_session`, `dialog_identity`) and stay there. `server::transfer` is the *B2BUA-side helper layer on top* — picking which leg, resolving the target via `ContactResolver`, choosing blind vs attended. Lift source: the dispatch shape at `crates/orchestration-core/src/orchestrator.rs:1225-1286` (which calls into UnifiedCoordinator helpers — that path itself is unchanged). The cross-transport `TransferConnection` command lives in `rvoip-core` and dispatches into the SIP adapter. |
| **`ConnectionAdapter` trait (the cross-transport seam)** | `rvoip-core` | `INTERFACE_DESIGN.md` §6. `rvoip-sip::SipAdapter` implements it; future `rvoip-webrtc`, `rvoip-quic` will too. Defining it now (with one impl) locks the seam shape. |
| **`MediaStream` trait** | `rvoip-core` | `INTERFACE_DESIGN.md` §3.6. SIP impl is `RtpMediaStream` inside `rvoip-sip`. |
| **Battle-tested `api/*` surface (`UnifiedCoordinator`, `StreamPeer`, `CallbackPeer`, `Endpoint`)** | `rvoip-sip::api` (renamed crate, module stays public) | The proven softphone + B2BUA-primitive surface. UnifiedCoordinator's `make_call` / `accept_call` / `bridge` / `send_refer` / `register` are what FreeSWITCH/Asterisk integration tests exercise. **Sole path** to `dialog-core` and `media-core` from rvoip-sip. Imports change from `use rvoip_session_core::api::*` to `use rvoip_sip::api::*`; otherwise the surface is unchanged. |
| **SIP-internal modules (`state_machine`, `state_table`, `session_store`, `auth`, `session_registry`, `types`)** | `rvoip-sip` (private — `pub(crate)`) | Already crate-private in session-core today. No public-API change. |
| **`adapters` module** | `rvoip-sip` (`#[doc(hidden)] pub`) | Currently `#[doc(hidden)] pub` in session-core (not strict `pub(crate)`). Visibility unchanged on rename. |
| **Voice-AI (`voice_ai.rs`)** | Stays in `orchestration-core` for now (extracted in PRD §13.3 step 6) | Not extracted to `rvoip-harness` yet. |
| **Workforce: `Agent` / `Queue` / `Router` / `AgentOffer` / `AssignmentManager`** | Stays in `orchestration-core` (deleted in PRD §13.3 step 7) | Per the scope contract. Keeps `developer_workflows.rs` and queue-themed examples running. |
| **Identity / `IdentityProvider` trait** | `rvoip-core` (skeleton only) | Trait surface, no implementations. Not load-bearing in this PR. |
| **`ConversationStore` / `VconStore` traits** | `rvoip-core` (skeleton only, in-memory default) | `INTERFACE_DESIGN.md` §11. Not wired into the SIP path yet. |

The two layering rules from `INTERFACE_DESIGN.md` §18 we commit to and enforce mechanically:

1. `rvoip-core` never imports an adapter crate. Adapters depend on `rvoip-core`, not the other way round. Enforced by `cargo deny` or workspace lint.
2. SIP-vocabulary symbols stay inside `rvoip-sip`; voip-3 vocabulary stays inside `rvoip-core`. Enforced by clippy lint or doc-test. **Note:** in this PR series rvoip-sip is the only adapter, so rule 2 effectively says "no SIP leakage into rvoip-core public surface." When `rvoip-webrtc` lands (PRD §13.3 step 5) the rule re-broadens to the full INTERFACE_DESIGN §18 rule 1 form: no symbol from one adapter family appears in another adapter's public surface.

---

## 4. Step 1 — Create `rvoip-core` skeleton

**Goal:** A new crate with types, traits, and an `Orchestrator` shell that compiles. No adapters registered yet. No behavior change in the SIP path.

New crate at `crates/rvoip-core/` (added to workspace `members` and `default-members`).

Module layout:

```
rvoip-core/src/
├── lib.rs                  // re-exports of the public surface
├── ids.rs                  // ConversationId, SessionId, ConnectionId, StreamId, MessageId,
│                           // ParticipantId, IdentityId, DeviceId, BridgeId, TenantId
├── conversation.rs         // Conversation type + ConversationState, ConversationPolicy
├── session.rs              // Session type + SessionState, SessionMedium
├── connection.rs           // Connection type + ConnectionState, Direction, Transport enum
├── stream.rs               // MediaStream trait (channel-based per §3.6) + StreamKind
├── message.rs              // Message + MessageOrigin, MessageRecipients, ContentType
├── participant.rs          // Participant + ParticipantKind, ParticipantRole
├── identity.rs             // Identity, Device, IdentityAssurance gradient,
│                           // IdentityProvider trait, Credential enum
├── adapter.rs              // ConnectionAdapter trait + AdapterKind enum,
│                           // OriginateRequest, RejectReason, EndReason, TransferTarget
├── capability.rs           // CapabilityDescriptor, CapabilityIntersection, NegotiatedCodecs
├── commands.rs             // OpenConversation, RouteInboundConnection, OriginateConnection,
│                           // BridgeConnections, TransferConnection, EndConnection, ...
├── events.rs               // Events (ConnectionInbound, ConnectionConnected,
│                           // ConnectionsBridged, RegistrationChanged, ...) wired through
│                           // infra-common::GlobalEventCoordinator (reuse Phase 0 work)
├── bridge.rs               // BridgeHandle, BridgeManager, bridge tokio task
│                           // (cross-transport pump; calls down to rvoip-media later for
│                           // transcoding, falls through to adapter fast-path when both
│                           // ends are the same transport with matching codecs)
├── store/
│   ├── conversation_store.rs  // ConversationStore trait + MemoryConversationStore
│   ├── vcon_store.rs          // VconStore trait + MemoryVconStore (skeleton)
│   └── mod.rs
├── orchestrator.rs         // Orchestrator entry: register(adapter), route_inbound_connection,
│                           // originate_connection, bridge_connections, transfer_connection,
│                           // end_connection, hold/resume, mute, send_message, send_dtmf
├── error.rs                // RvoipError + Result alias
└── config.rs               // Config (admission semaphore size, transcoding pairs,
                            // identity provider, conversation store, vcon store)
```

What lands in code:

- All structs/traits compile but have minimal stub bodies. `Orchestrator::register` returns `Err(NotImplemented)` until step 2 wires SIP in.
- `BridgeManager` carries forward the DashMap-of-bridges + `by_call` index pattern from `crates/orchestration-core/src/orchestrator.rs:25` (Phase 1 work survives).
- `Orchestrator::admission: Arc<Semaphore>` carries forward Phase 2 work — same defaults (`max_concurrent_setups` = 256 × `available_parallelism()`).
- `events.rs` adds a new `RvoipCrossCrateEvent::Core(RvoipCoreCrossCrateEvent)` variant in `crates/infra-common/src/events/cross_crate.rs` from day one. Rvoip-core's event vocabulary (the new cross-transport `Connection*` / `Bridge*` / `Conversation*` events) lives on `RvoipCoreCrossCrateEvent`, NOT on the legacy `OrchestrationCrossCrateEvent`. Rationale: `orchestration-core` is deleted in PRD §13.3 step 7, and we don't want `RvoipCrossCrateEvent::Orchestration(...)` lingering as a misnomer in the rvoip-core spine. The existing `Orchestration(...)` variant stays in place as long as orchestration-core does (workforce events keep their home). Phase 0's `GlobalEventCoordinator` plumbing is reused unchanged.
- `IdentityProvider`, `ConversationStore`, `VconStore` traits compile with in-memory defaults; no production impls in this PR.
- Cargo features (per `INTERFACE_DESIGN.md` §2.2): scaffold but only `sip` and `media` are meaningful in this PR.

Critical files to study while implementing:

- `crates/orchestration-core/src/orchestrator.rs:25-90` — current `BridgeManager` shape (lift verbatim with rename)
- `crates/orchestration-core/src/orchestrator.rs:1021-1086` — `bridge_agent_offer` (the bridge-creation flow; the cross-transport `BridgeConnections` command in `rvoip-core` follows this shape but takes two `ConnectionId`s instead of two call legs)
- `crates/orchestration-core/src/events.rs` — existing event bus + per-variant DashMap (Phase 0 work to re-host)
- `crates/infra-common/src/events/cross_crate.rs` — where new `RvoipCrossCrateEvent` variants land

---

## 5. Step 2 — Rename `session-core` to `rvoip-sip` and add the server surface

**Goal:** `rvoip-sip` is a SIP-complete library — usable as a SIP client (softphone) via the proven `api/*` surface, OR as a SIP server / B2BUA / gateway via the new `server/*` surface, OR as an adapter inside `rvoip-core::Orchestrator` via the new `SipAdapter`. After this step, the SIP path runs through `Orchestrator → SipAdapter → api::UnifiedCoordinator → dialog-core` end-to-end.

This is a **rename + add**, not a rewrite. The proven internals stay unchanged.

Final layout at `crates/rvoip-sip/`:

```
rvoip-sip/src/
├── lib.rs                     // re-exports: api, server, adapter, types, errors
├── adapter.rs                 // NEW: SipAdapter implements rvoip-core::ConnectionAdapter
│                              // dispatches to api::UnifiedCoordinator
│
├── api/                       // UNCHANGED from session-core/api (PUBLIC, battle-tested)
│   ├── unified                //   UnifiedCoordinator (make_call, accept_call, hangup,
│   │                          //                       bridge, send_refer, register, ...)
│   ├── stream_peer            //   StreamPeer (sequential test API)
│   ├── callback_peer          //   CallbackPeer (reactive server API + CallHandler trait)
│   ├── endpoint               //   Endpoint (simplified softphone interface)
│   ├── handle                 //   SessionHandle, RegistrationHandle
│   ├── audio                  //   AudioStream send/recv
│   ├── events                 //   Event enum, EventReceiver
│   ├── lifecycle              //   lifecycle snapshots
│   ├── registration           //   Registration types
│   └── ... (whatever api/ has today, unchanged)
│
├── server/                    // NEW: SIP B2BUA / gateway helpers
│   │                          // ALL functions in this module call api::UnifiedCoordinator
│   │                          // for actual SIP/RTP work. They do NOT call dialog-core or
│   │                          // media-core directly. The "server" module is coordination
│   │                          // glue on top of api/, not a parallel access path.
│   ├── mod.rs
│   ├── bridge.rs              //   SipBridgeStrategy: a function the rvoip-core
│   │                          //   Orchestrator invokes when both Connections are SIP.
│   │                          //   Internally calls api::UnifiedCoordinator::bridge(a, b),
│   │                          //   which returns a media-core::BridgeHandle that the
│   │                          //   Orchestrator stores in its BridgeManager registry.
│   │                          //   No new media-core wrapping; api/* already does it.
│   ├── transfer.rs            //   B2BUA-side transfer ORCHESTRATION (decide which leg
│   │                          //   to REFER, build target URI from ContactResolver, choose
│   │                          //   blind vs attended, manage transferor session linkage).
│   │                          //   The REFER mechanics themselves (send_refer, accept_refer,
│   │                          //   transfer_attended, etc.) already exist in
│   │                          //   api::UnifiedCoordinator and are NOT re-implemented —
│   │                          //   server::transfer just calls into them. Lifts the
│   │                          //   dispatch shape from orchestration-core/src/orchestrator
│   │                          //   .rs:1225-1286 (VoiceAiAction transfer match arm +
│   │                          //   transfer_call helper) into a reusable module.
│   ├── contact_resolver.rs    //   ContactResolver trait, RegistrarContactResolver,
│   │                          //   StaticContactResolver. Calls registrar-core directly
│   │                          //   for AOR -> Contact URI lookup. No dialog-core / media-
│   │                          //   core involvement (this is signaling-metadata, not
│   │                          //   media). (Lifted from orchestration-core/src/
│   │                          //   traits.rs:122-197.)
│   └── b2bua.rs               //   Optional convenience: SipB2bua wires the canonical
│                              //   pattern in ~10 lines — subscribe to incoming INVITE
│                              //   events from api::Event, call api::UnifiedCoordinator::
│                              //   accept_call, call api::UnifiedCoordinator::make_call
│                              //   for the outbound leg, call api::UnifiedCoordinator::
│                              //   bridge to join them. All through api/. Not required
│                              //   for using api/* directly.
│
├── adapters/                  // PRIVATE (was session-core/adapters)
│   ├── dialog_adapter.rs      //   wraps dialog-core
│   ├── media_adapter.rs       //   wraps media-core
│   ├── registration_adapter.rs
│   ├── srtp_negotiator.rs
│   └── cross_crate.rs         //   cross-crate event handler (orchestrates infra-common)
│
├── state_machine/             // PRIVATE (was session-core/state_machine)
│   ├── executor.rs
│   ├── actions.rs
│   ├── guards.rs
│   ├── effects.rs
│   └── helpers.rs
│
├── state_table/               // PRIVATE (was session-core/state_table)
│   ├── builder.rs
│   ├── loader.rs
│   ├── transitions.rs
│   └── default.yaml           //   RFC 3261 state transitions
│
├── session_store/             // PRIVATE (was session-core/session_store)
│   ├── store.rs               //   multi-session DashMap (Phase 1 work survives)
│   └── state.rs               //   SessionState
│
├── auth/                      // PRIVATE (was session-core/auth) — digest auth
├── session_registry.rs        // PRIVATE
├── types.rs                   // PUBLIC: CallState, FailureReason, MediaState, TransferStatus
│                              // (already public in session-core)
└── errors.rs                  // PUBLIC: SessionError + Result
```

What's new vs `session-core`:

- `adapter.rs` — `SipAdapter` implementing `rvoip_core::ConnectionAdapter`:
  - `originate(req)` → `api::UnifiedCoordinator::make_call(...)` (or `make_call_with_auth` / `make_call_with_pai` per req)
  - `accept(conn)` → `api::UnifiedCoordinator::accept_call(...)`
  - `reject(conn, reason)` → `api::UnifiedCoordinator::reject_call(...)`
  - `end(conn, reason)` → `api::UnifiedCoordinator::hangup(...)`
  - `hold/resume/transfer/send_dtmf/send_message/renegotiate_media` → existing `UnifiedCoordinator` methods
  - `streams(conn)` → returns `Vec<Arc<dyn MediaStream>>` wrapping the SIP RTP streams
  - `subscribe_events()` → forwards `api::Event` then normalizes into `rvoip-core` event vocabulary
  - `verify_request_signature(...)` → returns `IdentityAssurance::Anonymous` for v1 (per `INTERFACE_DESIGN.md` §6)
- `server/bridge.rs` — `SipBridgeStrategy`: a function `bridge(api: &UnifiedCoordinator, a: SessionId, b: SessionId) -> Result<media_core::BridgeHandle>` that calls `api::UnifiedCoordinator::bridge(a, b)`. The result handle is stored by `rvoip-core::Orchestrator` in its `BridgeManager` registry. No re-wrapping of `media-core::BridgeHandle`; the existing return type is what the Orchestrator stores.
- `server/transfer.rs` — B2BUA-side transfer orchestration helpers (decide which leg to REFER, build the target URI via `ContactResolver`, pick blind vs attended, manage transferor session linkage). The actual REFER mechanics (`send_refer`, `accept_refer`, `reject_refer`, `send_refer_with_replaces`, `make_transfer_leg`, `set_transferor_session`, `dialog_identity`) already live in `api::UnifiedCoordinator` and are NOT re-implemented; `server/transfer` just calls into them. Lift source: the dispatch shape at `crates/orchestration-core/src/orchestrator.rs:1225-1286` (`VoiceAiAction` transfer match arm + `transfer_call` helper).
- `server/contact_resolver.rs` — `ContactResolver` trait + `RegistrarContactResolver` + `StaticContactResolver` lifted verbatim from `crates/orchestration-core/src/traits.rs:122-197`. Talks to `registrar-core` (signaling metadata only — no dialog-core / media-core access). The `Router` and `QueueSelector` traits in that same source file are NOT moved (they stay in orchestration-core, deleted in PRD §13.3 step 7).
- `server/b2bua.rs` (optional) — convenience helper that wires the common B2BUA pattern (inbound INVITE → originate onward → bridge) entirely through `api::UnifiedCoordinator`. Validates the `server/*` surface stands on its own (a SIP-only consumer can use rvoip-sip without rvoip-core involvement) by composing api/ calls.

What's NOT new:

- The `api/*` surface is **unchanged in shape**. Everything that was `pub` in `session-core::api` is still `pub` at `rvoip_sip::api`. Downstream code changes one import path: `use rvoip_session_core::api::*` → `use rvoip_sip::api::*`.
- All session-core internal modules keep their existing visibility on rename: `state_machine`, `state_table`, `session_store`, `auth`, `session_registry`, `types` stay `pub(crate)`; `adapters` stays `#[doc(hidden)] pub` (its current state — not strict `pub(crate)`).
- The session-core test suite (35 integration tests under `tests/`) moves with the crate; they pass after the rename.

The two-layer surface for consumers:

- **Pure-SIP consumer** (carrier, SIP softphone vendor, SIP-only call center) does `use rvoip_sip::api::*` (softphone style) or `use rvoip_sip::server::*` (B2BUA style) and never touches `rvoip-core`.
- **Cross-transport consumer** (Thelve, future CPaaS) does `use rvoip_core::*` and registers the SIP adapter: `orchestrator.register(SipAdapter::new(SipConfig { ... }))?;`. Same `Orchestrator` later registers WebRTC/QUIC/UCTP adapters when those crates exist.

---

## 6. Migration order (small atomic commits)

The two big steps above each break into smaller commits to keep review tractable and bisects clean:

1. **Add `rvoip-core` crate skeleton.** Empty types/traits, compiles, not yet wired anywhere. Workspace member added.
2. **Move neutral type definitions into `rvoip-core`.** `Conversation`, `Session`, `Connection`, `Stream`, `Message`, `Participant`, `Identity`, IDs, capability descriptor, commands, events. No behavior change — orchestration-core still owns runtime; new types are just declared.
3. **Move `BridgeManager` to `rvoip-core::bridge`.** The Phase-1 DashMap-of-bridges shape lifts as-is. orchestration-core re-exports from `rvoip-core` so existing call paths keep working.
4. **Define `ConnectionAdapter` trait in `rvoip-core::adapter` and the `Orchestrator` shell.** Still no impls.
5. **Rename `session-core` → `rvoip-sip` (path move + Cargo.toml rename).** Crate name changes; `api/*` still public; internals keep their visibility (`#[doc(hidden)] pub` for `adapters`; `pub(crate)` for the rest). Single commit, mostly mechanical. Update workspace Cargo.toml `members` and `default-members`. **Keep `crates/session-core/` as a one-file shim crate** (`Cargo.toml` + `src/lib.rs` containing `pub use rvoip_sip::*;`) for one release. Protects the out-of-workspace `crates/rvoip/` and any external consumers; deleted in PRD §13.3 step 7.
5b. **Rename wire-visible identifiers.** Flip the User-Agent string `"rvoip-session-core"` → `"rvoip-sip"` in `crates/rvoip-sip/src/adapters/registration_adapter.rs`. Flip the dialog Call-ID format `format!("{}@session-core", session_id.0)` → `format!("{}@rvoip-sip", session_id.0)` in `crates/rvoip-sip/src/adapters/dialog_adapter.rs`. Update test fixture Call-IDs containing the literal `"session-core"` in `tests/redirect_follow.rs`, `tests/unified_api_tests.rs`, `tests/generated_sip_compliance.rs`. Visible in PBX/gateway logs and SIP traces; intentional rename for crate-identity consistency.
6. **Update downstream import paths.** `crates/orchestration-core/src/**` and any tests / examples / downstream-out-of-workspace crates: `rvoip_session_core::api::*` → `rvoip_sip::api::*`. Includes ~48 doc-comment occurrences of `cargo run -p rvoip-session-core` across the 57 example files. Mechanical sed-style change.
7. **Add `rvoip-sip::adapter::SipAdapter` implementing `rvoip-core::ConnectionAdapter`.** Wraps `api::UnifiedCoordinator`. Compiles but orchestration-core doesn't use it yet.
8. **Add `rvoip-sip::server::*` modules.** `bridge.rs` (`SipBridgeStrategy` calling `api::UnifiedCoordinator::bridge()`), `contact_resolver.rs` (lift `ContactResolver` trait + `StaticContactResolver` + `RegistrarContactResolver` from `orchestration-core/src/traits.rs:81-198`; `Router` + `QueueSelector` at `:10-77` left behind), `transfer.rs` (B2BUA-side transfer orchestration helpers — lift the dispatch shape from `orchestration-core/src/orchestrator.rs:1225-1286`). Drop `ContactResolver` from orchestration-core; orchestration-core now imports it from `rvoip-sip::server`. Update orchestration-core's `VoiceAiAction` dispatch in `orchestrator.rs:1225-1260` to call `rvoip-sip::server::transfer::*` helpers instead of inline transfer dispatch (this means orchestration-core now depends on `rvoip-sip::server` in addition to `rvoip-sip::api`).
9. **Wire orchestration-core's call handler through `Orchestrator → SipAdapter`.** Replaces the direct `UnifiedCoordinator` call in `crates/orchestration-core/src/orchestrator.rs`. The end-to-end SIP flow now passes through the `ConnectionAdapter` seam — proves rvoip-core works with one adapter live.
10. **Add `crates/rvoip-sip/examples/sip_b2bua.rs`** demonstrating the SIP-only B2BUA surface (`use rvoip_sip::server::*`, no rvoip-core involvement). Adds `crates/rvoip-core/examples/sip_only_orchestrator.rs` demonstrating the cross-transport surface with one adapter registered.
11. **Mark queue-themed examples in orchestration-core as `#[deprecated(note = "moves to consumer in PRD §13 step 7")]`** at the function level. No code change beyond annotations.
13. **Publish crate-level docs in `crates/rvoip-sip/src/lib.rs`.** A `//!` module-level doc that shows the three usage patterns side-by-side: (a) softphone via `api::*` (StreamPeer / Endpoint / CallbackPeer); (b) B2BUA via `server::*` (bridge + contact_resolver + transfer + b2bua); (c) cross-transport via `rvoip-core::Orchestrator` + `SipAdapter` registration. Each pattern with a 5-10 line code snippet linking to the matching example. Counterpart `//!` doc in `crates/rvoip-core/src/lib.rs` showing the cross-transport entry point. This is what makes `rvoip-sip` "easy to use" for the three target consumers (softphone vendor, gateway author, B2BUA / call-center backend).

Each step compiles cleanly, all existing tests pass, the workspace stays buildable, perf does not regress.

---

## 7. Critical files to change

| Step | File | Change |
|---|---|---|
| 1 | `Cargo.toml` (workspace) | Add `crates/rvoip-core` to `members` + `default-members` |
| 1–4 | `crates/rvoip-core/**` (new) | New skeleton crate per §4 module layout |
| 3 | `crates/orchestration-core/src/orchestrator.rs:25-90` | Replace `BridgeManager` with `pub use rvoip_core::bridge::BridgeManager` |
| 5 | `Cargo.toml` (workspace) | Rename `crates/session-core` → `crates/rvoip-sip`; update `members` and `default-members` |
| 5 | `crates/rvoip-sip/Cargo.toml` | `name = "rvoip-sip"` (was `rvoip-session-core`); description updated |
| 5 | `crates/rvoip-sip/src/**` | Wholesale path rename from `crates/session-core/src/**`; no source changes |
| 5 | `crates/session-core/Cargo.toml` (rewritten) + `crates/session-core/src/lib.rs` (new, 1 line: `pub use rvoip_sip::*;`) | One-file shim crate kept for one release; protects out-of-workspace `crates/rvoip/` and external consumers. Deleted in PRD §13.3 step 7. |
| 5b | `crates/rvoip-sip/src/adapters/registration_adapter.rs` | User-Agent string `"rvoip-session-core"` → `"rvoip-sip"` |
| 5b | `crates/rvoip-sip/src/adapters/dialog_adapter.rs` | Call-ID format `"{}@session-core"` → `"{}@rvoip-sip"` |
| 5b | `crates/rvoip-sip/tests/{redirect_follow,unified_api_tests,generated_sip_compliance}.rs` | Test fixture Call-IDs containing literal `"session-core"` updated to match new host part |
| 6 | `crates/orchestration-core/src/**` and any callers | `rvoip_session_core` → `rvoip_sip` import paths |
| 6 | `crates/orchestration-core/Cargo.toml` | Replace `rvoip-session-core` dep with `rvoip-sip` |
| 6 | `crates/rvoip-sip/examples/**` (57 files across 7 subdirectories) | ~48 doc-comment occurrences of `cargo run -p rvoip-session-core` → `cargo run -p rvoip-sip` |
| 7 | `crates/rvoip-sip/src/adapter.rs` (new) | `SipAdapter` impl of `rvoip-core::ConnectionAdapter` |
| 8 | `crates/rvoip-sip/src/server/bridge.rs` (new) | `SipBridgeStrategy` calling `api::UnifiedCoordinator::bridge()` (no media-core wrapping; api/ already does that) |
| 8 | `crates/rvoip-sip/src/server/contact_resolver.rs` (new) | Lift `ContactResolver` trait + `StaticContactResolver` + `RegistrarContactResolver` from `crates/orchestration-core/src/traits.rs:81-198` |
| 8 | `crates/rvoip-sip/src/server/transfer.rs` (new) | B2BUA-side transfer orchestration helpers (decide which leg, build target URI, blind vs attended). Calls into `api::UnifiedCoordinator::send_refer`/`accept_refer`/`transfer_attended` (REFER mechanics stay in api). Lift dispatch shape from `crates/orchestration-core/src/orchestrator.rs:1225-1286` |
| 8 | `crates/orchestration-core/src/traits.rs` | Drop `ContactResolver`/`RegistrarContactResolver`/`StaticContactResolver` (lines 81-198); keep `Router` + `QueueSelector` (lines 10-77; deleted in step 7) |
| 8 | `crates/orchestration-core/src/orchestrator.rs` | Import contact-resolver / transfer helpers from `rvoip-sip::server`; rewire `VoiceAiAction` dispatch at `:1225-1260` to call `rvoip-sip::server::transfer::*` |
| 9 | `crates/orchestration-core/src/orchestrator.rs` | Route SIP calls through `rvoip_core::Orchestrator → rvoip_sip::adapter::SipAdapter` instead of direct `UnifiedCoordinator`. Decide whether the existing `coordinator.reject_call()` sites at `:209` (admission gate) and `:306` (route rejection) also flow through the adapter, or stay as direct calls (behavior delta worth a callout in the commit message). |
| 9 | `crates/orchestration-core/Cargo.toml` | Add `rvoip-core` workspace dependency (in addition to existing `rvoip-sip`) |
| 7 (events) | `crates/infra-common/src/events/cross_crate.rs` | Add `RvoipCrossCrateEvent::Core(RvoipCoreCrossCrateEvent)` variant + the `RvoipCoreCrossCrateEvent` enum (new event vocabulary for rvoip-core's cross-transport `Connection*` / `Bridge*` / `Conversation*` events) |
| 10 | `crates/rvoip-sip/examples/sip_b2bua.rs` (new) | SIP-only B2BUA example (no rvoip-core) |
| 10 | `crates/rvoip-core/examples/sip_only_orchestrator.rs` (new) | rvoip-core Orchestrator with one SIP adapter registered |
| 11 | `crates/orchestration-core/examples/{human_queue,ai_only_queue,mixed_ai_human_queue,ai_then_human_handoff}.rs` | Add `#[deprecated]` annotation |
| 13 | `crates/rvoip-sip/src/lib.rs` | Module-level `//!` doc showing the three usage patterns (softphone via `api::*`, B2BUA via `server::*`, cross-transport via `rvoip-core::Orchestrator + SipAdapter`), each with a 5-10 line snippet linking to its example |
| 13 | `crates/rvoip-core/src/lib.rs` | Module-level `//!` doc showing the cross-transport entry point (`Orchestrator::register(adapter)`, command surface, event vocabulary) |

Things explicitly **not** changed in this PR series (the scope contract):

- `crates/orchestration-core/src/store.rs` (workforce stores `MemoryAgentStore` / `MemoryQueueStore` / `MemoryAgentOfferStore`) — untouched per `PERFORMANCE_PLAN.md` scope
- `crates/orchestration-core/src/assignment.rs` (`AssignmentManager`) — untouched
- `crates/orchestration-core/src/voice_ai.rs` — untouched (extracted to `rvoip-harness` in PRD §13.3 step 6, not now)
- `crates/orchestration-core/src/types.rs` `Agent` / `Queue` / `AgentOffer` — untouched
- `crates/media-core/**` — untouched. `media-core::BridgeHandle` is what `api::UnifiedCoordinator::bridge()` returns today; that contract is unchanged.
- `crates/dialog-core`, `crates/sip-transport`, `crates/rtp-core`, `crates/registrar-core` — untouched. They keep their crate identity. `rvoip-sip::api::*` consumes `dialog-core` and `media-core` (as session-core does today). `rvoip-sip::server::contact_resolver` consumes `registrar-core` (as orchestration-core's `RegistrarContactResolver` does today). Nothing else in `rvoip-sip::server::*` or `rvoip-sip::adapter` touches dialog-core or media-core directly — they all go through `api::UnifiedCoordinator`.
- `crates/rvoip/` (currently out-of-workspace, depending on legacy session-core) — untouched. Brought back into workspace in PRD §13.3 step 7 as the facade.
- The `api/*` public surface inside `rvoip-sip` — surface shape unchanged. `UnifiedCoordinator`/`StreamPeer`/`CallbackPeer`/`Endpoint` keep their methods and semantics. Only the import path changes (`rvoip_session_core::api` → `rvoip_sip::api`).

---

## 8. Verification

After each commit:

1. **Build:** `cargo build --workspace` — clean.
2. **Unit tests:** `cargo test --workspace` — clean. In particular:
   - `cargo test -p rvoip-orchestration-core` (existing tests, including `developer_workflows`)
   - `cargo test -p rvoip-sip` (the renamed session-core tests — all 35 integration tests pass under the new crate name)
   - `cargo test -p rvoip-core` (new — initially type-shape and `BridgeManager` unit tests)
3. **Perf no-regression:** `cargo test -p rvoip-orchestration-core --test perf_active_calls --release` for N ∈ {100, 500, 1000} — per-call wall time stays at or below the 2026-05-10 baseline (0.065 / 0.083 / 0.112 ms per the `PERFORMANCE_PLAN.md` ship report). The DashMap + admission semaphore work survives the move because they're being lifted, not rewritten.
4. **Live SIP/RTP:** `RVOIP_LIVE_SIP_RTP_COUNTS=5,50 cargo test -p rvoip-orchestration-core --test perf_live_sip_rtp --release` — N=50 setup ≤ ~1100 ms (matches Phase 2 baseline of 1058 ms).
5. **session-core regression suites still pass under `rvoip-sip`** — the FreeSWITCH/Asterisk-tested behaviors (state-table, SDP matching, REGISTER/423 retry, REGISTER challenge retry, early media, SRTP, TLS, glare, session timer, REFER/NOTIFY, blind transfer, event filtering) all run green from the new crate path. This is the proof that the rename did not break the proven surface.
6. **Examples:**
   - `cargo run -p rvoip-orchestration-core --example human_queue` — runs (with deprecation warning)
   - `cargo run -p rvoip-orchestration-core --example ai_only_queue` — runs
   - `cargo run -p rvoip-orchestration-core --example ai_then_human_handoff` — runs
   - `cargo run -p rvoip-orchestration-core --example registered_sip_agent` — runs
   - existing rvoip-sip examples (formerly `session-core/examples`: 57 `.rs` files across 7 subdirectories — `endpoint/`, `stream_peer/`, `callback_peer/`, `unified/`, `pbx/`, `sip_client/`, `regression/`) — all run from their new crate path. Doc-comments inside them (~48 occurrences of `cargo run -p rvoip-session-core`) updated in step 6.
   - `cargo run -p rvoip-sip --example sip_b2bua` — bridges two SIP legs through `server/*` (new)
   - `cargo run -p rvoip-core --example sip_only_orchestrator` — Orchestrator with one SIP adapter (new)
7. **Cross-crate sanity:** `crates/rvoip-core/Cargo.toml` has zero adapter-crate deps (no `rvoip-sip`, `rvoip-webrtc`, etc.). Per `INTERFACE_DESIGN.md` §18 rule 2.
8. **Wire-identifier check (post step 5b):** capture an INVITE and a REGISTER from a softphone example and grep the trace — User-Agent header should read `rvoip-sip` (not `rvoip-session-core`); Call-ID host part should be `@rvoip-sip` (not `@session-core`). Confirms the rename reached SIP wire visibility.
9. **Shim sanity:** `cargo check -p rvoip-session-core` (the shim crate at `crates/session-core/`) builds and re-exports the rvoip-sip surface. A trivial downstream `use rvoip_session_core::api::UnifiedCoordinator;` still resolves.

---

## 9. Target crate hierarchy (the end-state shape — must be considered now)

This PR series creates two new crates (`rvoip-core`, `rvoip-sip`) but the wider workspace needs a deliberate hierarchy so that future moves (SIP-family consolidation, WebRTC-family addition, common-crate renames) don't fight each other. The decision to lock in now: **flat directory layout with `rvoip-`-family naming prefixes.** Sort `crates/` alphabetically and family members cluster.

### 9.1 Target structure (when the full PRD §13.3 migration is done)

```
crates/
├── rvoip/                       FACADE — re-exports everything, feature-gated
│                                (the existing crates/rvoip/ once it's brought back into
│                                workspace at PRD §13.3 step 7)
│
├── rvoip-core/                  TRANSPORT-AGNOSTIC SPINE — voip-3 types, ConnectionAdapter
│                                trait, BridgeManager, Orchestrator entry point
│
│  ── SIP family (alphabetically clusters under "rvoip-sip-*") ──
├── rvoip-sip/                   UMBRELLA — was crates/session-core (api/, server/, adapter)
├── rvoip-sip-core/              SIP message parsing + SDP — was crates/sip-core
├── rvoip-sip-dialog/            SIP dialog state machine — was crates/dialog-core
├── rvoip-sip-registrar/         SIP REGISTER processing — was crates/registrar-core
├── rvoip-sip-transport/         SIP wire transport (UDP/TCP/TLS/WS) — was crates/sip-transport
│
│  ── WebRTC family (future, PRD §13.3 step 5) ──
├── rvoip-webrtc/                UMBRELLA — DTLS-SRTP, ICE, signaling, PeerConnection
│
│  ── UCTP / substrate adapters (future, PRD §13.3 steps 3-4) ──
├── rvoip-uctp/                  UCTP envelope encode/decode
├── rvoip-quic/                  QUIC substrate adapter
├── rvoip-webtransport/          WebTransport substrate adapter
├── rvoip-websocket/             WebSocket substrate adapter
│
│  ── COMMON crates (used across families) ──
├── rvoip-audio/                 Audio processing primitives — was crates/audio-core
├── rvoip-auth/                  OAuth2 / token auth — was crates/auth-core (NOT SIP digest;
│                                SIP digest stays inside rvoip-sip's private auth/ module)
├── rvoip-codec/                 Codec implementations (G.711, future Opus) — was crates/codec-core
├── rvoip-infra/                 GlobalEventCoordinator, telemetry — was crates/infra-common
├── rvoip-media/                 Transport-agnostic media: MediaStream trait, mixing,
│                                transcoding pairs — was crates/media-core
├── rvoip-rtp/                   RTP/SRTP transport (used by rvoip-sip AND rvoip-webrtc)
│                                — was crates/rtp-core
│
│  ── Future common crates (PRD §13.3 followups) ──
├── rvoip-harness/               AI voice harness (ASR/TTS/Dialog providers) — extracted from
│                                orchestration-core::voice_ai (PRD §13.3 step 6)
├── rvoip-identity/              IdentityProvider impls (OAuth+DPoP, OIDC, FIDO, AAuth)
├── rvoip-vcon/                  vCon emission, JWS sign, JWE encrypt
├── rvoip-users/                 User registry — was crates/users-core (could merge with
│                                rvoip-identity later; defer the call)
│
└── (gone after PRD §13.3 step 7)
   orchestration-core/           DELETED — becomes the rvoip facade or its workforce content
                                 lifts to the consumer (Thelve)
```

### 9.2 Classification of every existing crate

| Today | Kind | Target name | Move scheduled for |
|---|---|---|---|
| `crates/sip-core/` (`rvoip-sip-core`) | SIP-specific | `rvoip-sip-core` (name unchanged) | Step 12 of this PR series (directory rename only) — see §9.4 |
| `crates/sip-transport/` (`rvoip-sip-transport`) | SIP-specific | `rvoip-sip-transport` (name unchanged) | Step 12 |
| `crates/dialog-core/` (`rvoip-dialog-core`) | SIP-specific | `rvoip-sip-dialog` (rename) | Step 12 |
| `crates/registrar-core/` (`rvoip-registrar-core`) | SIP-specific | `rvoip-sip-registrar` (rename) | Step 12 |
| `crates/session-core/` (`rvoip-session-core`) | SIP-specific | `rvoip-sip` (umbrella, rename) | **Step 5 of this PR series** (already in scope) |
| `crates/audio-core/` (`rvoip-audio-core`) | Common | `rvoip-audio` (rename, drop `-core`) | Deferred to a focused common-crate-rename PR |
| `crates/auth-core/` (`rvoip-auth-core`) | Common (OAuth2; not SIP digest) | `rvoip-auth` (rename, drop `-core`) | Deferred. SIP digest stays inside rvoip-sip's private `auth/` module unchanged. |
| `crates/codec-core/` (`rvoip-codec-core`) | Common | `rvoip-codec` (rename, drop `-core`) | Deferred |
| `crates/infra-common/` (`rvoip-infra-common`) | Common | `rvoip-infra` (rename) | Deferred |
| `crates/media-core/` (`rvoip-media-core`) | Common | `rvoip-media` (rename, drop `-core`) | Deferred |
| `crates/rtp-core/` (`rvoip-rtp-core`) | Common | `rvoip-rtp` (rename, drop `-core`) | Deferred |
| `crates/users-core/` (`users-core`) | Common (needs investigation) | `rvoip-users` (or merge into `rvoip-identity`) | Deferred until rvoip-identity lands |
| `crates/orchestration-core/` (`rvoip-orchestration-core`) | Workforce + facade-in-waiting | Deleted or becomes `rvoip` facade | PRD §13.3 step 7 |
| `crates/rvoip/` (out-of-workspace today) | Facade | `rvoip` (back into workspace) | PRD §13.3 step 7 |
| `crates/old_call-engine/` | Legacy | Delete | Deferred (not in any active path) |

### 9.3 Directory layout decision: flat vs nested

Two viable shapes; we pick **flat with naming prefix**:

- **Flat (chosen):** every crate is at `crates/<name>/`. Family membership is conveyed by the `rvoip-sip-*`, `rvoip-webrtc-*` prefix. Sorting `crates/` alphabetically clusters families together.
- **Nested (rejected):** `crates/rvoip-sip/` directory contains both the umbrella crate AND sub-crate subdirectories (`crates/rvoip-sip/dialog/`, `crates/rvoip-sip/registrar/`, etc.). Cargo supports this, but the umbrella crate's own `src/` lives next to subdirectories with their own `Cargo.toml`, which is fiddly.

Flat-with-prefix matches the existing convention (every crate today already has the `rvoip-` prefix in its package name) and keeps Cargo paths short. Nested is rejected on grounds of adding mechanical complexity without solving a real problem.

### 9.4 What this PR series does about hierarchy

**Recommended (and what this plan commits to):** in this PR series, do **only** the rename forced by the carve — `crates/session-core/` → `crates/rvoip-sip/` (package rename `rvoip-session-core` → `rvoip-sip`). Add a new **step 12** at the end of the migration order: rename the four other SIP-specific crates (`sip-core`, `sip-transport`, `dialog-core`, `registrar-core`) into `rvoip-sip-*` directory and package names. This is mechanical, has clear scope, and can ship as one focused commit. Defer all common-crate renames to their own PRs after this series.

Why split it this way:

- The session-core → rvoip-sip rename is **forced** by the carve (we can't add the new server/adapter surface without it). Doing it now is unavoidable.
- The four other SIP-family renames are **independent** of the carve mechanically. They're directory + package-name renames that touch every importer's `Cargo.toml` and every `use` statement. Bundling them into the carve doubles the bisect surface for no architectural gain.
- Step 12 (after the carve work proves out) is the right home: the workspace already has `rvoip-sip` as its umbrella, and the four SIP children just adopt the consistent `rvoip-sip-*` naming. Single mechanical commit, ~easy review.
- Common-crate renames (`audio-core` → `rvoip-audio`, etc.) are even more decoupled — they touch every crate in the workspace and have zero architectural value beyond cleanup. They wait until the migration is otherwise done so the diff isn't fighting other in-flight work.

**Updated migration order (extends the §6 list to 13 steps with 5b sub-step):**

12. **Rename remaining SIP-family crates.** `crates/sip-core/` → `crates/rvoip-sip-core/` (package name unchanged); `crates/sip-transport/` → `crates/rvoip-sip-transport/` (package name unchanged); `crates/dialog-core/` → `crates/rvoip-sip-dialog/` (package `rvoip-dialog-core` → `rvoip-sip-dialog`); `crates/registrar-core/` → `crates/rvoip-sip-registrar/` (package `rvoip-registrar-core` → `rvoip-sip-registrar`). Update workspace Cargo.toml `members` + `default-members`. Update every importer's `Cargo.toml` and `use` statements (sed-style mechanical change). Optionally keep one-line re-export shim crates at the old names for one release. After this commit, sorting `crates/` clusters the entire SIP family alphabetically.

**Alternative considered and rejected for this PR series:**

- *Do all renames now in one big bang.* Rejected: 5x larger diff, harder bisect, no architectural payoff that's worth it. The carve work and the rename work are independent — keep them independent.
- *Defer step 12 to a separate PR series entirely.* Defensible but leaves the workspace in a half-renamed state for longer than necessary. Step 12 is cheap once the carve is done; might as well close the loop.

### 9.5 Implications for the followup section

The "Out of scope / followups" section below is updated implicitly — the SIP-family directory renames move into this PR series (as step 12), and the common-crate renames are added explicitly to followups.

---

## 10. Out of scope / followups (not this PR series)

These are tracked for the next PR series but not blockers for landing the rvoip-sip carve:

- **PRD §13.3 step 3:** `rvoip-uctp` envelope encode/decode crate.
- **PRD §13.3 step 4:** `rvoip-quic`, `rvoip-webtransport`, `rvoip-websocket` substrate adapters.
- **PRD §13.3 step 5:** `rvoip-webrtc` interop adapter (and the start of the WebRTC family — future `rvoip-webrtc-ice`, `rvoip-webrtc-dtls-srtp`, etc. as needed).
- **PRD §13.3 step 6:** Extract `voice_ai.rs` → `rvoip-harness`.
- **PRD §13.3 step 7:** Delete workforce code (`Agent` / `Queue` / `Router` / `AgentOffer` / `AssignmentManager`); rename `orchestration-core` → `rvoip` facade (or delete and re-create from `crates/rvoip/`); relocate / delete queue-themed examples and `developer_workflows.rs`.
- **PRD §13.3 step 7:** Promote `Conversation`/`Session`/`Connection` from "type definitions co-located with `Call`/`CallLeg`" to canonical names; deprecate the old aliases.
- **`rvoip-vcon` crate** (per `INTERFACE_DESIGN.md` §11.4) — vCon emission, signing, encryption.
- **`rvoip-identity` crate** (per `INTERFACE_DESIGN.md` §8) — production `IdentityProvider` impls (OAuth 2.1+DPoP, OIDC, SIP Digest, FIDO/passkeys, AAuth). May absorb `users-core` at this point.
- **Common-crate renames** (per §9.2 classification — defer to one focused mechanical-rename PR after the carve is shipped):
  - `crates/audio-core/` (`rvoip-audio-core`) → `crates/rvoip-audio/` (`rvoip-audio`)
  - `crates/auth-core/` (`rvoip-auth-core`) → `crates/rvoip-auth/` (`rvoip-auth`)
  - `crates/codec-core/` (`rvoip-codec-core`) → `crates/rvoip-codec/` (`rvoip-codec`)
  - `crates/infra-common/` (`rvoip-infra-common`) → `crates/rvoip-infra/` (`rvoip-infra`)
  - `crates/media-core/` (`rvoip-media-core`) → `crates/rvoip-media/` (`rvoip-media`)
  - `crates/rtp-core/` (`rvoip-rtp-core`) → `crates/rvoip-rtp/` (`rvoip-rtp`)
  - `crates/users-core/` (`users-core`) → `crates/rvoip-users/` (`rvoip-users`) OR merge into `rvoip-identity`
- **Legacy cleanup:** `crates/old_call-engine/` is not in the active workspace path; delete in a focused cleanup PR.
- The `rvoip` facade crate at `crates/rvoip/` (currently out-of-workspace, depending on legacy session-core) will be brought back into workspace in step 7 once `orchestration-core` becomes (or is replaced by) the facade.
