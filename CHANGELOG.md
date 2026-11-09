# Changelog

## Unreleased

### Highlights

0.5.0 is a breaking minor release about carrier-scale throughput and overload
behaviour. A new opt-in media executor, `MediaExecutorConfig::v2()`, runs
media on dedicated, CPU-pinned lanes sized by a host plan. Receive loops and
paced sends run inside one poller per lane; inbound SRTP is authenticated,
decoded and delivered inline on the socket owner and RTP is sent
synchronously; lanes wake on a 2 ms tick and keep their deadlines in a timing
wheel; SRTP, RTCP and receive statistics stay with the socket owner; per-call
hot state is packed into a few cache lines; per-call sends and receives are
fired from a per-lane call table; and the SIP inbound pump runs fused on the
socket owner with zero-copy playout. `MediaExecutorConfig::v1()`, the per-call
task graph, remains the default: nothing changes unless an application
installs v2. SIP is hardened for overload and call lifecycle: TLS flows apply
back-pressure instead of dropping responses or closing, transaction/TU
deadlocks are fixed, new dialog-forming INVITEs can be shed with a stateless
503 before any state exists, hangup and crossed-BYE races end calls cleanly,
ownerless sessions are swept, and overload rejections carry `503` with
`Retry-After`. Signaling and media hot paths copy, lock and scan less per
call, and media graphs that bridge a SIP leg to an in-process AI run on that
leg's media lane. The executor v2 CPU-per-call figures below were measured on
Xeon 6 SMT in production; load qualification of executor v2 is part of the
0.5.0 release run.

### Breaking changes

Each item has a one-line migration.

- **`Require: replaces` is answered with 420 Bad Extension** (#379). rvoip
  does not perform RFC 3891 dialog replacement, so the default
  `Config::uas_capabilities` no longer claims the `replaces` option tag. A
  request that requires it, such as an INVITE that asks to replace one of
  this endpoint's dialogs, is rejected as RFC 3261 §8.2.2.3 requires. It
  used to be processed as an unrelated new call. Attended transfer where
  rvoip is the transferee (an inbound REFER, then an INVITE with Replaces
  sent to the target) is unaffected. Migration, for an application that
  performs the replacement itself:
  `config.uas_capabilities.supported_extensions.push("replaces".into());`.
- **`RejectReason` gains `ServiceUnavailable { retry_after_secs }`** (#355).
  `rvoip-core-traits`' `RejectReason` is not `#[non_exhaustive]`. rvoip-sip
  answers it with `503 Service Unavailable` and `Retry-After`; QUIC,
  WebTransport and WebSocket map it to 503. Migration: add an arm (or a
  wildcard) to exhaustive `match`es on `RejectReason`.
- **`rvoip_sip::state_table::EventType` gains `Dialog100Trying` and
  `CancelAfterTrying`** (#348). The enum is not `#[non_exhaustive]`.
  Migration: add the two arms to exhaustive `match`es; custom state tables
  without the new rows keep their behaviour, and `CancelAfterTrying` is
  derived by the executor and cannot be submitted as an event.
- **`rvoip_sip_dialog::transaction::transport::TransportManagerConfig` gains
  the public fields `tls_flow` (#338) and `new_call_admission` (#350).**
  Migration: add them to struct literals, or end the literal with
  `..Default::default()`.
- **`rvoip_harness::InProcessAiConfig` is `#[non_exhaustive]`** and gains
  `gated_input` (#349). Code outside the crate can no longer build it with a
  struct literal, `..Default::default()` included. Migration: start from
  `InProcessAiConfig::default()` and set fields, or chain `with_codec`,
  `with_media_queue_capacity`, `with_event_queue_capacity`,
  `with_lifecycle_ack_timeout`, `with_terminal_history_capacity` and
  `with_gated_input`.
- **`CrossBridgeHandle::new` and `CrossBridgeHandle::with_swap_channels` take
  the pumps' `JoinHandle<()>`s** instead of `AbortHandle`s, so `stop()` can
  await the pumps' exit (#340). Migration: pass the `JoinHandle`s that
  `bridge::frame_pump::spawn_pump` returns instead of calling `abort_handle()`
  on them.
- **rvoip-sip's inbound `ConnectionAdapter::end` can return an error after the
  call ended** (#355). Once its hangup join passes the configured bound it
  fails; `Ended` has already been delivered and the call is reclaimed in the
  background. On a draining adapter, `end` now fails before it retires the
  route. Migration: treat that error as "ended, cleanup continuing", and tune
  the bounds with `Config::with_session_cleanup`.
- **A call whose media a peer BYE stopped first now ends as
  `AdapterEvent::Ended { Normal | Cancelled }`** instead of `Failed { "SIP
  media driver failed" }` (#355). Migration: stop treating that failure as a
  remote hangup; for a genuine media failure the terminal is now published
  before the BYE confirmation is joined.
- **rvoip-core may publish terminal events of different connections in a
  different order than the adapter reported them** (#345). Per-connection
  order is unchanged, and `AdapterLifecycleSink::deliver_terminal` now returns
  once core has retired the connection and owns its teardown, rather than
  after media cleanup. Migration: do not rely on terminal ordering across
  connections, or on media cleanup having finished when `deliver_terminal`
  returns.

### SIP

#### Configurable UAS capabilities

- `Config::uas_capabilities` (an `rvoip_sip::api::unified::UasCapabilities`)
  sets what the endpoint implements as a UAS. Inbound requests are checked
  against it before processing (RFC 3261 §8.2): 405/501 with Allow, 416, 420
  with Unsupported, 415 with Accept, 406. The automatic OPTIONS 200 now
  advertises the same lists in Allow, Supported and Accept (RFC 3261 §11.2).
  It used to send a fixed 8-method Allow that omitted REGISTER, MESSAGE,
  NOTIFY, SUBSCRIBE and PRACK, with no Supported or Accept.
- The default no longer lists `replaces`; see Breaking changes.
- `100rel` is withdrawn when `Config::use_100rel` is `NotSupported`, so
  `Require: 100rel` gets 420 (RFC 3262 §4).
- sip-dialog: `DialogConfig::uas_capabilities` / `with_uas_capabilities`.

#### TLS flow back-pressure (#338)

- A momentarily full TLS writer queue no longer drops SIP responses. Every TLS
  send used `try_send` on the flow's bounded writer queue (128 messages by
  default) and failed with `BufferCapacityExceeded` the instant it was full; a
  single-worker 250 CPS reproduction lost 119 `200 OK` and four `503`
  responses this way. A send now waits for queue room inside the caller's own
  future, bounded by the transport's write timeout (the handshake timeout, 10
  s by default), and fails only when that expires. No retry task is spawned,
  so a timed-out or cancelled send is never written later. A response pinned
  to an exact flow stays on that flow; a pooled request flow that closed
  during the wait still falls back to a fresh dial.
- A momentarily full transport event queue no longer closes the TLS flow. The
  reader used to drop the connection as soon as the transaction layer's queue
  was full, which on a shared carrier trunk stranded every dialog routed over
  it. It now keeps the message it holds, stops reading (TCP flow control
  pushes back on the peer) and delivers it in order once there is room.
  `TlsReceiveBackpressurePolicy` bounds the wait: `Pause` (default) closes the
  flow only after `TlsFlowConfig::receive_pause_max` (600 s by default);
  `Close` gives up after `min(write timeout, 1 s)`. The wait still ends at
  once when the flow's writer stops or its idle or authentication deadline
  passes.
- TLS sockets now set `TCP_NODELAY` by default on accepted and dialed flows,
  so Nagle's algorithm cannot hold a response behind an unacknowledged
  segment. Turn it off with `TlsFlowConfig::with_tcp_nodelay(false)`.
- New per-transport `TlsFlowConfig` (`#[non_exhaustive]`, `with_*` builders)
  carries these settings and an optional `IngressAdmissionHook`, which sees
  every parsed inbound message with its peer address before it is queued and
  returns an `IngressVerdict`: `Pass`, `Absorb` (drop it) or `Respond(bytes)`
  (answer on the same flow without stalling reads), so an application can shed
  overload statelessly at read rate. `TlsFlowConfig::stats` and
  `TlsTransport::flow_stats` return a `TlsFlowStats` snapshot of pause, close
  and hook counters, shared by every transport built from clones of one
  config. Set it with `rvoip_sip::Config::with_sip_tls_flow` (field
  `sip_tls_flow`), `TransportManagerConfig::tls_flow`, or the new
  `TlsTransport::bind_with_role_and_flow_config` and
  `TlsTransport::client_only_with_flow_config`. Existing `TlsTransport`
  constructors use `TlsFlowConfig::default()`.

#### Transaction layer (#339, #350)

- The transaction layer no longer deadlocks against a dialog handler under
  load. The dialog manager drains the bounded transaction event queue and
  can await `send_response` or `terminate_transaction` while it handles one
  of that queue's events. A server transaction completed `send_response`
  only after it had published the resulting `StateChanged`, so with the
  queue full the runner waited for the handler and the handler waited for
  the runner, with no timeout. `send_response` now completes once the
  response is written, the state hook (Timer G/H/J/K) has run and the event
  holds its place in the queue; the event is still delivered losslessly and
  in order.
- `terminate_transaction` no longer waits for its own terminal events to be
  consumed. It used to publish `StateChanged(Terminated)` and
  `TransactionTerminated` before completing, so a handler that called it
  with a full queue stalled for one second, after which the manager closed
  admission for every new transaction. It now completes once the runner is
  joined and the transaction is removed from the manager and its timers; the
  manager keeps delivering the terminal events in order and keeps the
  transaction key reserved until they are queued. A second, concurrent
  termination of the same transaction now also waits for the runner join
  instead of proceeding as if the runner had already stopped.
- An explicit transaction termination no longer closes the transaction manager
  when its terminal events wait on a full transaction-user queue (#350). After
  one second the manager used to fail closed and latch into draining for good:
  every later inbound request failed without a response, every local BYE
  failed, and ended sessions could never release their slots; a cleanup burst
  at about 3,400 calls/s was enough. The publication now continues in the
  background, in order and with its exact owners, and stops on shutdown. Only
  a closed transaction-user channel still fails closed, and that is now logged
  and counted. `TransactionManager::slot_cleanup_stats` returns a
  `SlotCleanupStats` snapshot (`#[non_exhaustive]`).
- A draining transaction manager answers a new INVITE with a stateless `503
  Service Unavailable` carrying the stateless-overload `Retry-After`, instead
  of dropping it so the caller retransmits until Timer B (#350). Other
  requests are unchanged.
- Repeated transaction failure logs (initial-request send failures,
  state-entry failures, failed INVITE cancellation) are thinned to the first
  ten occurrences, then powers of two, then every 10,000th, each carrying an
  `occurrences` count (#350).

#### Stateless new-call admission (#350)

- New stateless admission for new dialog-forming INVITEs in `rvoip-sip-dialog`
  (`transaction::overload`). A per-manager `NewCallAdmission` gate
  (`AdmissionMode::Off` by default, `Observe`, `Enforce`) answers a rejected
  INVITE with an RFC 3261 §8.2.6 stateless 503 (all Vias, deterministic
  `rvoip-ovl-` To tag, optional `Retry-After`) before any transaction, dialog
  or session state exists, gives a retransmission the identical 503, and
  absorbs the ACK for it. In-dialog requests, retransmissions, ACK and CANCEL
  are never decided. Applications publish an `AdmissionLevel` (closed, or shed
  a fraction of Call-IDs) and an optional `NewCallRateLimit` at run time, and
  read `NewCallAdmissionStats`. Over UDP, TCP and WebSocket the gate runs on
  the transaction dispatch path; over TLS it runs on the connection reader
  through the TLS flow ingress hook, unless the application supplied its own
  hook. Configure it with `TransportManagerConfig::new_call_admission` or
  `rvoip_sip::Config::with_sip_new_call_admission` (field
  `sip_new_call_admission`), and reach it with
  `TransactionManager::new_call_admission` or
  `UnifiedCoordinator::new_call_admission`. All new types are
  `#[non_exhaustive]`; there are no process-global statics.
- New `TransactionManager::set_new_dialog_capacity_guard`
  (`NewDialogCapacityGuard`): while the transaction user reports its session
  slots full, a new INVITE gets a stateless 503 with `Retry-After` before any
  transaction exists. `rvoip_sip::Config::server_call_admission_early_reject`
  (default `false`) installs it against `server_call_admission_limit`, and
  `UnifiedCoordinator::sip_slot_cleanup_stats` exposes the counters.

#### Inbound call lifecycle and hangup (#348, #355, #364)

- A rejected inbound call no longer blocks the adapter event translator
  (#348). Rejecting an incoming call before it is published (connection
  capacity exhausted, a missing or rejected signaling context, an
  authenticated principal that is no longer active, a backpressured handoff)
  sends its final response and can fall back to a hangup and a forced cleanup;
  the translator ran that inline, so one slow rejection delayed every later
  event of every call. Rejections now run as retained tasks that shutdown
  joins, at most 256 at a time; past that the translator rejects inline, so it
  is backpressured rather than spawning unbounded work. Each rejection owns
  its exact call lifetime, so it never touches a newer call that reused the
  session ID.
- `ConnectionAdapter::accept` reserves the `Connected` event slot before it
  answers an inbound call (#348). With the adapter event queue full it now
  rejects the still-unanswered call with `503 Signaling Event Backpressure`
  and returns `AdmissionRejected`, instead of sending 200 OK and then leaving
  the answered call without its establishment handoff or media setup.
- New `Config::sip_adapter_call_capacity: Option<usize>` and
  `Config::with_sip_adapter_call_capacity()` (#348). The default `None` keeps
  the 256-event queue and 4096 pending inbound observations; when set, the
  event queue holds two events per call and the pending-observation store one
  per call, never fewer than the defaults. Servers that answer more than about
  128 calls at once should set it to their expected concurrent call count;
  valid values are 1 to `Config::MAX_SIP_ADAPTER_CALL_CAPACITY` (131072).
- New `Config::session_cleanup` (`SessionCleanupConfig`,
  `Config::with_session_cleanup`) bounds the session cleanup the `SipAdapter`
  owns (#355). All three bounds are on by default:
  - `inline_hangup_deadline`: 5 s, before a BYE transaction exists;
  - `inbound_end_backstop`: 10 s;
  - `pending_hangup_watchdog`: on.
- New `cleanup_diag::session_cleanup_counters()` returns
  `SessionCleanupCounters`, always-on counters for the bounded cleanup paths:
  cancelled reject terminals, inline-deadline and backstop expiries, inbound
  end reclaims and their outcomes, forced session-slot releases and failures,
  and pending-hangup watchdogs armed (#355).
- Every inbound 503 from `ConnectionAdapter::reject` that carries no
  `Retry-After` of its own now gets
  `Config::server_overload_retry_after_secs`, including `RejectReason::Custom
  { code: 503, .. }` (#355).
- Hangup fixes (#355): a local hangup that loses the session lane to a peer
  BYE no longer waits the non-INVITE timeout for a BYE it never sent;
  cancelling a caller of inbound `end` or `reject` no longer loses the call's
  terminal handoff to the orchestrator; an inbound end whose hangup never
  completes no longer holds the orchestrator's end and connection forever; an
  ended inbound call whose hangup failed no longer keeps its session-store
  slot (bounded reclaim and exact-release retries end in a counted forced slot
  release); and a UAS that hangs up after its 200 OK and never gets the ACK
  now ends the call through the accepted-call watchdog instead of leaking the
  session and its media port.
- Counting live setup/teardown deadlines is now O(1); it used to sum over
  every live call under the scheduler lock on each lifetime close (#355).
- A local hangup whose BYE is crossed by the peer's own BYE no longer fails
  when the peer's BYE releases the call before the local BYE is answered
  (#364). The confirmation wait reported "SIP BYE confirmation ended during
  exact local cleanup", so the inbound `end` returned an adapter error and
  started a needless reclaim for a call that had ended normally. A
  confirmation that exact cleanup interrupts now counts as success only when
  the lifetime already carries a normal (`Ended`) terminal fact; any other
  terminal, or none, still fails.

#### Orphan sweep and RTP port admission (#360)

- New `SessionCleanupConfig::orphan_session_ttl` (default 60 s, 5 s to 1 h,
  `None` turns it off) and `with_orphan_session_ttl`. A `SipAdapter` reclaims
  call sessions that have had no owner (no adapter route, pending inbound
  context, in-flight rejection or reclaim) and no new RTP for the TTL,
  together with their media ports; established calls get a BYE first. Call
  sessions abandoned without an owner no longer keep their session slot and
  media port forever. `SipAdapter::orphan_sessions_by_state()` reports the
  last sweep's ownerless sessions by state.
- An inbound INVITE is answered `503 Service Unavailable` with
  `Config::server_overload_retry_after_secs` before any session state exists
  when the RTP range has no free slot for the call's reservation: one port, or
  an RTP/RTCP pair for a non-mux offer with `Config::rtcp_non_mux`. The
  coordinator warns at startup when `server_call_admission_limit` exceeds the
  calls its RTP range holds.
- An inbound INVITE whose setup fails after the dialog exists (session
  creation, the dialog commit, media allocation or another setup action) now
  gets one `503` with `Retry-After` after its upper resources are released. It
  used to get no final response, leaving the caller to time out and the server
  transaction and dialog allocated.
- `SessionCleanupCounters` gains `orphan_session_reclaims` and its outcomes
  (`already_released`, `graceful`, `forced`, `timed_out`), plus
  `media_port_admission_rejects`, `setup_failure_rejects` and
  `setup_failure_unanswered`. rtp-core gains `SessionPortReservation` and
  `PortAllocator::{ports_per_session, configured_session_slots,
  available_port_count, available_session_slots}` (pair slots count free
  aligned even/odd pairs); media-core gains
  `MediaSessionController::{available_session_port_slots,
  configured_session_port_slots}`.

#### State table (#348)

- A UAC hangup or cancel after a `100 Trying`, with no other provisional
  response, now sends CANCEL at once, as RFC 3261 §9.1 allows. Before, the
  call waited in `CancelPending` for a 180 or 183 that a peer may never send,
  so the INVITE ran on until the peer answered or Timer B fired. A 100 to the
  initial INVITE now records progress (`Dialog100Trying`) without publishing
  ringing, and a 100 that reaches `CancelPending` sends the pending CANCEL.
  `CallProgress` events for 100 are unchanged.
- A peer BYE that crosses our own in-flight BYE (`Terminating` + `DialogBYE`)
  now commits `Terminated` and publishes `CallTerminated`. There was no row
  for it, so the peer BYE was answered but the session stayed in `Terminating`
  until our BYE completed or timed out. Like the `Terminating` +
  `DialogTerminated` row, the new row runs no cleanup actions: the retained
  exact terminal release quiesces the local BYE before it cleans the dialog
  and media once.

#### SIP media streams (#340, #342, #343)

- Closing a `SipMediaStream` awaits the media driver's `JoinHandle` instead of
  spinning on `is_finished` for up to two seconds (#340). A cancelled close
  leaves the driver owned by the stream, so dropping the stream still aborts
  it.
- The inbound and outbound media pumps hold one codec-change waiter per codec
  epoch, and the inbound playout timer is re-armed only when its deadline
  moves, instead of re-registering both for every packet (#343).
- A media pump that stops because the inbound audio source closed (the
  ordinary end of every call's media after a remote hangup) logs
  `SipMediaStream pump stopped: audio source closed` at `debug` instead of a
  per-call `warn` (#342). Other pump failures still warn.

### Signaling performance

Wire output, published events and STIR/SHAKEN behaviour are unchanged unless a
bullet says otherwise.

- `rvoip-sip-dialog` no longer deep-copies SIP requests and responses where it
  only reads them (#346). Removed: a copy of every inbound request, ACK and
  response made only to compute its transaction key (the new
  `transaction_key_from_request` and `transaction_key_from_response` take the
  message by reference and return the same key as
  `transaction_key_from_message`); a second copy of the request in every
  client and server transaction constructor; two copies of each inbound INVITE
  published as `IncomingCall` (the whole event is now copied only while a
  STIR/SHAKEN verifier is installed); and a copy of the original request for
  every response sent through the dialog layer and every `200 OK` a server
  transaction sends. The new `TransactionManager::server_transaction_request`
  returns the transaction's shared `Arc<Request>` while it is active.
- `rvoip-sip` no longer parses each inbound INVITE a second time (#352).
  Inbound INVITE observers and the pending call bundle
  (`IncomingCall::raw_request()`) get the server transaction's own shared
  request; the bytes are parsed again only when the transaction has already
  retired.
- Less copying per published call event (#352): dialog events reach the
  session shards as the shared envelope instead of a deep copy, each
  application event moves into the control owner's channel instead of being
  cloned (the public observation is still a separate sanitized copy), and the
  lifecycle index checks for transfer outcomes without cloning and
  Debug-formatting every event.
- `UnifiedCoordinator::events_with_control` no longer subscribes to the public
  observation bus, whose copies the control owner dropped anyway; its stream
  now ends when the control channel closes (#352). `SipAdapter` Debug-formats
  unmapped API events into `AdapterEvent::Native` only while the
  orchestrator's debug log (target `rvoip_core::orchestrator`), their only
  consumer, is enabled; direct `subscribe_events` /
  `try_subscribe_atomic_events` consumers still receive them.
- New opt-in `Config` fields for inbound servers; the defaults keep current
  behaviour (#352):
  - `server_call_admission_lock_free`
    (`with_server_call_admission_lock_free`): decide server call admission
    with one atomic reservation instead of a mutex. It never admits past
    `server_call_admission_limit`, but may reject (503) one INVITE of a
    concurrent race exactly at the limit;
  - `inbound_session_history` (`with_inbound_session_history`, default
    `true`): `false` creates inbound sessions without the diagnostic
    transition history, as outbound sessions already are;
  - `session_inline_incoming_call_dispatch`
    (`with_inline_incoming_call_dispatch`): process each initial INVITE on the
    dialog worker that delivered it instead of on a session event shard.
- Per-call registry lookups no longer scan every live call (#342). The UAS
  reliable-provisional (RFC 3262) registry indexes its retransmit tasks and
  closed-transaction fences by dialog, so PRACK, the exact final-response
  close and dialog cleanup each read one entry instead of iterating every
  shard; rvoip-sip's exact-response registry indexes pending INFO / REGISTER
  obligations by session lifetime, so exact call teardown reads one index
  entry instead of walking every pending obligation. Together these scans cost
  5.8% of worker CPU at 4,500 calls per second in a CPS-scaling profile,
  growing with the number of live calls.
- Call setup no longer polls (#354). An inbound `ConnectionAdapter::accept`
  waits for the ACK on the call's committed state instead of polling the
  session map every 10 ms, and a call whose session is removed during the wait
  fails with `ConnectionNotFound` at once instead of at the setup timeout. A
  SIP media stream waits for its media owner and its SDP negotiation the same
  way.
- The per-frame exact media lookup no longer takes the global session registry
  mutex, and resolving a trace's owner by SIP Call-ID no longer scans every
  registered call (#354). An inbound INVITE scans pending inbound observations
  for expired entries only when one can have expired. Negotiation parses each
  SDP body once and checks for an existing media session without sampling RTP
  statistics; `MediaSessionController` gains `has_media_session` and
  `get_session_config`, which read the published session without locking the
  RTP session.

### Media executor

Everything in this section is opt-in. The default media runtime is still one
shared lane, and `MediaExecutorConfig::v1()` (the per-call task graph) is
still the default executor; `MediaExecutorConfig::v2()` turns on every switch
below. CPU-per-call figures are each switch's measured effect in the
production v2 profile, measured on Xeon 6 SMT.

#### Media lanes and host plan (#343, #347)

- **The media executor can be split into independent lanes** (#343).
  `rvoip_rtp_core::task_runtime::MediaRuntimeConfig` (`#[non_exhaustive]`)
  selects `MediaLanes::Shared { worker_threads, lanes }`: up to 8
  multi-threaded runtimes, each with its own scheduler, timer driver and I/O
  reactor, splitting one worker-thread budget. Install it once with
  `install_media_runtime_config`, or set `rvoip_sip::Config::media_runtime` /
  `Config::with_media_runtime`, which `UnifiedCoordinator::new` installs
  before any media starts; a conflicting second layout is a `ConfigError`.
  `media_runtime_snapshot` reports each lane's workers and placements.
- **Each UDP media transport lives on one media lane** (#343). Its RTP and
  separate RTCP sockets are re-registered on that lane's reactor (same
  descriptor, address and options; no rebind) and its receive tasks run there,
  so media readiness no longer wakes the signaling runtime that created the
  transport, and the transport keeps receiving if that runtime stops.
  Transports are placed round robin.
- **Lanes can be dedicated threads pinned to CPUs** (#347).
  `MediaLanes::Dedicated { cpus }` (also `MediaLanes::dedicated(n)` and
  `MediaLanes::pinned(cpus)`) runs one current-thread Tokio runtime per lane
  on its own thread, pinned to `cpus[i]` when set (Linux); a CPU the kernel
  refuses, or a platform without thread affinity, leaves that lane unpinned
  with a warning. `MediaLaneSnapshot::pinned_cpu` reports the CPU each lane
  got; `start_media_runtime`, `current_media_lane` and a public
  `pin_current_thread` are new.
- **`rvoip_rtp_core::host_plan` sizes the media lanes and the signaling
  runtime from the host** (#347). `ExecutorPlan::probe(&HostPlanConfig)` reads
  the usable CPUs (affinity and cgroup v2 cpuset), SMT sibling topology,
  `cpu.max` quota, memory limits, `pids.max`, `RLIMIT_NOFILE` and NUMA nodes
  from `/proc` and `/sys`, and plans one media lane per usable core, pinned to
  the core's first SMT sibling, with signaling threads floating over the
  second siblings. Without SMT the cores are split by a signaling share; under
  a CPU quota the plan uses `⌊quota⌋` threads and pins nothing.
  `ExecutorPlan::media_runtime_config()` and `signaling_runtime_builder()`
  return the matching lane layout and Tokio builder. `HostPlanConfig`
  (`#[non_exhaustive]`, validated) holds the signaling shares (`sigma`,
  default 0.40, and `sigma_smt`), the housekeeping-core threshold, the SMT
  policy, an explicit signaling thread count and an optional
  `SignalingBalance` of per-call CPU costs;
  `HostPlanConfig::sibling_signaling()` gives one signaling thread per media
  core (23 media lanes and 23 signaling threads instead of 10 on a 24-core SMT
  host). Nothing reads the environment, and nothing changes unless the
  application applies the plan.

#### Executor v2 switches

- **`rvoip_rtp_core::executor::MediaExecutorConfig`** (#351): a typed,
  process-wide configuration (`#[non_exhaustive]`; `v1()`, `v2()`, and a
  `with_*` builder and `*_active` accessor per switch), installed once with
  `install_media_executor_config` or through
  `rvoip_sip::Config::media_executor` / `Config::with_media_executor`, which
  `UnifiedCoordinator::new` installs; an equal configuration is shared, a
  conflicting one is a `ConfigError`. `MediaExecutorConfig::validate` rejects
  `tx_batch` without `tx_handle_cache` under v2
  (`MediaExecutorConfigError::Requires`, #361) and lane tick quanta other than
  1, 2, 4 or 5 ms (`MediaExecutorConfigError::InvalidLaneTickQuantum`, #356).
  `executor::inline_executor_counters()` returns process totals for the
  application to export.
- **Inline receive, lane pollers and synchronous send** (#351). An
  authenticated inbound RTP packet is decrypted in place in the receive buffer
  and accounted run to completion on the socket owner
  (`RtpSession::install_inline_media_sink`, the `InlineMediaSink` trait), with
  one owner timer per socket and a receive drain budget; without a sink the
  session publishes `PacketReceived` as in v1. `lane_poller` runs receive
  loops inside one poller task per lane worker, `lane_timer` puts lane-hosted
  futures on the poller's lane clock (`rvoip_rtp_core::media_clock`), and
  `lane_tx` hosts paced send tasks on the poller
  (`executor::spawn_on_media_lane`). `RtpSendHandle::try_send_now` sends a
  frame synchronously with SRTP protected into a reused buffer, identical on
  the wire to `send_packet_with_pt`; with `tx_batch`, co-due frames leave in
  one `sendmmsg` on Linux. SSRC-keyed maps on the packet path use a keyed
  SplitMix64 hasher (`fast_ssrc_hash`).
- **Owner SRTP, RTCP and receive statistics** (#357). With `owner_srtp` a
  receive loop checks its transport's inbound SRTP/SRTCP context out and
  authenticates without the context lock (not on a transport with a separate
  RTCP socket); with `owner_rtcp`, RTCP multiplexed on the RTP socket is
  handled on the socket owner with the session's own RTCP handling, and
  periodic reports leave with a synchronous send byte-identical to
  `send_rtcp_bytes`; with `owner_receive_stats` the socket owner publishes
  per-packet receive statistics through a single-writer seqlock instead of the
  statistics mutex.
- **Inline decode and a PCM buffer pool** (#353). With `inline_decode`, each
  G.711 dialog's reorder window, source timeline, concealment, decode, pacing
  and delivery run on the socket owner instead of the dialog's receive task,
  with identical frames, timestamps and concealment. Other codecs, and dialogs
  a bridge attaches to, stay on (or are demoted to) the v1 handler, which
  keeps serving DTMF; such dialogs no longer publish `PacketReceived` unless
  demoted, and the in-tree bridge demotes both legs itself. New
  `MediaSessionController::install_inline_audio_consumer`
  (`InlineAudioConsumer`) and
  `MediaSessionController::request_inline_demotion`. With `pcm_pool`, decoded
  and concealed PCM uses a bounded per-thread pool
  (`rvoip_media_core::pcm_pool`).
- **Fused SIP media pump, cached send handles and AI-call send** (#361).
  `fused_inbound_pump` runs a G.711 SIP stream's inbound pump on the RTP
  socket owner as media-core's inline audio consumer; `direct_inbound_sink`
  (with `count_only_sink`) feeds an offered `InboundFrameSink` directly;
  `ai_tx` sends encoded G.711 synchronously through a cached,
  generation-fenced path on the socket's lane
  (`MediaSessionController::prepare_g711_sender`, `PreparedG711Sender`);
  `ai_tx`/`session_owner_placement` place the stream's pumps on that lane and
  `ai_tx`/`ai_rx` publish it via `MediaStream::media_lane`;
  `direct_encoded_writer` makes `SipMediaStream::try_take_encoded_g711_writer`
  return a writer. `rvoip-core-traits`' `MediaStream` gains the defaulted
  hooks `offer_inbound_frame_sink`, `try_take_encoded_g711_writer` and
  `media_lane`; streams that do not implement them are unchanged. No
  `rvoip_sip::Config` or `Event` change.
- **Lane tick** (#356). `MediaExecutorConfig::lane_tick: Option<LaneTick>`
  (`LaneTick { quantum, grid_timers }`): a lane poller on a dedicated lane
  parks to the next boundary of a 1, 2, 4 or 5 ms grid after each pass and
  serves every socket and lane-clock deadline that became ready meanwhile in
  one pass; with `grid_timers` lane-clock deadlines are filed on the same
  grid. Deadlines are never served early and at most one quantum late,
  saturated passes skip the sleep, and cross-thread work ends the sleep at
  once or within 500 µs; decoded pacing paces from the served deadline, so the
  quantum never accumulates. `v2()` includes a 2 ms tick with grid timers
  (−22% CPU per call). Diagnostics:
  `rvoip_rtp_core::lane_tick::lane_tick_counters()`.
- **Lane timing wheel and one clock read per pass** (#358). With
  `lane_timer_wheel` a lane clock keeps its deadlines in a 256-bucket
  single-level wheel (far deadlines in an overflow heap): O(1) file, cancel
  and re-arm with no allocation, and exactly the heap's firing order and
  times. With `lane_pass_clock` a poller pass reads the clock once and the
  futures it polls read that value through `media_clock::now` / `now_std`;
  deadlines are never served early. Both in `v2()` (−4.6% CPU per call).
  Diagnostics: `executor::lane_wheel_counters()`.
- **Per-call hot state** (#362). With `hot_state` the socket owner packs what
  it touches per packet: an SRTP context packs each RTP direction into three
  cache lines, with an HMAC-SHA1 state keyed once per context; the receive
  loop reads one doorbell line per datagram and re-reads control-plane state
  only after a writer rang it; the synchronous send path caches the remote
  address and latch state until the doorbell rings; lane clocks prefetch the
  state of futures a round is about to wake. Wire output and what consumers
  observe are unchanged. In `v2()` (−2.5% CPU per call). Diagnostics:
  `rvoip_rtp_core::hot_state::hot_state_counters()`.
- **PCM pool sizing and lane-batched counters** (#359). With
  `counter_pool_cleanup` the per-thread PCM pool keeps up to
  `PCM_POOL_CLEANUP_CAPACITY` (1024) buffers of at most
  `PCM_POOL_CLEANUP_MAX_SAMPLES` (1024) samples, so a lane pass no longer
  allocates and frees buffers beyond 32, and per-event process counters added
  during a lane pass (`executor::add_lane_deferred`) are published every 64
  passes, on idle passes and at stop instead of one shared atomic add per
  event. Totals are unchanged; `pcm_pool_counters().misses` and the owner RTCP
  counters may trail by up to a batch, and a thread's pool may hold about 2
  MiB of small buffers.
- **Callback send from a per-lane call table** (#371). New
  `rvoip_rtp_core::lane_table` and `callback_send`: an application implements
  the synchronous `LaneTxSource` trait (`on_due(now) -> LaneNext::{At, Done,
  Escalate}`) for a paced per-call sender, and a supervisor hosted on the
  call's lane installs it with `lane_table::install_tx`; the poller then calls
  `on_due` straight from the lane's timing wheel, with no waker, hosted-future
  poll or `select!` per frame. `LaneTxHandle::retire` and
  `LaneTxHandle::finished` keep the supervisor in control; entries fire never
  early, on the tick grid, within per-pass and per-entry budgets; a panicking
  handler poisons only its entry; slots are generation-fenced. Where there is
  no table, `install_tx` gives the source back to be driven as a paced future.
  In `v2()` (−8% CPU per call with a recording sender as the source).
  Diagnostics: `lane_table::callback_lane_counters()`.
- **Receive loops served from the lane table** (#372). With
  `callback_rx_kernel` a v2 receive loop on a dedicated lane moves into the
  lane's table once it owns its call state and is called from the table's
  ready list when its socket is readable and from the wheel when its owner
  deadline is due, running the same per-packet step (`RxCore::on_datagram`) as
  the async loop. With `lane_rx_readiness` (Linux) the lane keeps one
  level-triggered epoll for those sockets and reads each reported socket once
  per report (empty `recvfrom` calls went from 1.55 per received datagram to
  0). `callback_rx_kernel` needs `callback_send` and `lane_rx_readiness` needs
  `callback_rx_kernel`. Both in `v2()` (−2.8% CPU per call against v2 without
  them). Diagnostics: the `rx_*` fields of `CallbackLaneCounters`.
- **Zero-copy playout in the fused inbound pump** (#373). With
  `playout_zero_copy` (needs `fused_inbound_pump`) a fused pump's playout
  buffer keeps pending frames in a few unordered slots instead of a `BTreeMap`
  and lends each due frame to the pump, then keeps it as its concealment
  template, instead of copying it. Frames, order, timestamps, payloads,
  concealment, late drops and playout statistics are identical. media-core
  adds `PlayoutBuffer::with_slot_storage` and `PlayoutBuffer::pop_due_with`;
  `PlayoutBuffer::new` and `pop_due` behave as before. In `v2()`.

### RTP/RTCP and media quality

#### Receive and send robustness (#337)

- A concealment frame can no longer ask for gigabytes of memory. A gap
  confirmed before the receive path had learned the stream's timestamp step
  was sized from the successor's timestamp without a half-range check, so a
  same-SSRC clock reset (for example seq 100 at ts 16000, then seq 102 at ts
  0) asked for about 2^32 samples, an 8.6 GiB zeroed allocation that aborts
  the process. A backward step now resumes at the received packet and relearns
  the cadence, and every concealment frame is bounded to one second of audio;
  a larger request is refused and the media behind it still plays. The first
  gap of a stream is also sized correctly.
- One lost inbound packet no longer delays or silences the rest of a call. An
  N-packet loss is concealed after one reorder hold instead of N holds (five
  lost packets used to hold the stream for 225 ms). The decoded playout queue
  drains a batch the reorder window released together instead of keeping it as
  standing latency. `PlayoutBuffer` follows a stream that fell behind its
  clock: after half a second of late frames arriving one frame apart while
  every slot is concealed, it delays its playout clock (at most
  `max_depth_frames` per step) instead of dropping every later frame and
  fading to silence; bursts and reorder holds do not move the clock. New
  `PlayoutBuffer::clock_delays()` counts the delays.
- The media-core RTP receive loop arms its timer at the next reorder or
  playout deadline instead of waking every 5 ms, so an idle or silent session
  no longer wakes 200 times per second.
- rtp-core treats an RTP datagram the local network stack refuses as a packet
  lost on the wire rather than a failed send: `EPERM`/`EACCES` (a netfilter
  DROP rule in OUTPUT, a full conntrack table), `ENOBUFS`, `ENOMEM`, and ICMP
  errors reported on a later send. The SIP outbound media pump used to end the
  call's outbound audio at the first refusal; now the far end sees a
  one-packet gap. Errors that mean the socket is unusable still fail the send.
  New `UdpRtpTransport::refused_rtp_datagrams()` counts them.
- Before symmetric RTP latches, outbound RTP and RTCP sends store the SDP
  destination only when it changed, instead of allocating an `Arc` and taking
  `arc_swap`'s writer path for every packet.

#### Hot path (#354, #357, #373)

These changes keep wire output, statistics, RTCP content and every error
outcome unchanged, and add no configuration.

- G.711 encodes and decodes through lookup tables. `rvoip-codec-core` gains
  `MULAW_ENCODE_TABLE` (16 KiB) and `ALAW_ENCODE_TABLE` (4 KiB) in
  `utils::tables`, built at compile time from the reference encoder, and every
  `G711Codec` entry point uses them and the existing decode tables;
  `encode_mulaw_table` and `encode_alaw_table` now really use a table.
  Measured on the Xeon 6 build host, a 20 ms frame encodes about 11 times
  faster (PCMU 567 ns to 50 ns) and decodes about 2 to 3 times faster. The
  free `ulaw_compress`/`alaw_compress` reference functions are unchanged
  (#354).
- PCMU and PCMA frames no longer take a codec mutex: a dialog's G.711 codec
  generation encodes and decodes through a shared stateless codec (#354).
- Encoded audio sends no longer lock the RTP session, and RTP send totals no
  longer take the session statistics mutex: the packet writer publishes its
  counters through a lock-free single-writer record that RTCP and
  `RtpSession::get_stats()` read (#354).
- SRTP keys its HMAC once per key derivation and clones the keyed HMAC-SHA1
  state per packet instead of running the key schedule and copying the packet;
  `SrtpAuthenticator` implements `Clone` and a `Debug` that omits key material
  (#354). Every SRTP and SRTCP HMAC-SHA1 tag, and the once-per-context keying
  (`hmac_sha1_core`), is computed by an `#[inline(never)]` function, and a
  test checks that every SHA-1 computation in the SRTP, hot-state and owner
  receive code runs in one of them, so the legacy-SSE SHA-NI code cannot be
  inlined into AVX packet paths under fat LTO or PGO; `calculate_auth_tag` no
  longer copies the packet to append the ROC (#357, #373).
- `PacketLossTracker` keeps its 2,048-packet duplicate window in a
  direct-address ring instead of a `HashSet` pruned on every advance (#354).
- The maps an RTP session owns use 16 shards instead of DashMap's default of
  four per CPU (1,024 on a 192-thread host), which reduces per-call memory and
  RTCP report work (#354).
- The UDP RTP and RTCP receive loops reuse one receive buffer instead of
  allocating and zeroing one per datagram; `PlayoutBuffer` reuses its
  concealment template's sample buffer instead of cloning every emitted frame;
  and stopping a media session removes only that session's aliases instead of
  scanning every active call's mapping (#354).

### Core and media graph

- Connection admission no longer sweeps the lifecycle table when bounded
  lifecycle retention is enabled (#345). Inbound admission swept every
  retained row every 64 admissions, and outbound admission on every claim,
  taking each live connection's lifecycle lock under the registry lock. Rows
  are now reclaimed by exact identity when their cleanup completes or their
  adapter cleanup quarantine resolves, so admission cost no longer grows with
  the number of live calls. `end_session` likewise removes only the ending
  session's own connection-to-session bindings.
- Authoritative operational publication no longer yields while it holds the
  publication-order lock and the stream has capacity (#345). `reserve().await`
  can return `Pending` on Tokio's cooperative budget even with free capacity,
  which parked every other connection's lifecycle publication behind one busy
  task; core now waits only when the stream is full.
- A slow connection teardown no longer stalls an adapter's event consumer
  (#345). For an `Ended`/`Failed` event core awaited bridge and media-graph
  teardown while holding the publication-order lock, delaying every other
  connection's events on that transport. Terminal events now retire the
  connection synchronously, in adapter order, release the lock, and hand
  teardown and the terminal publication to the bounded lifecycle supervisor
  (inline when it is saturated or draining). Rejections caused by a saturated
  inbound admission gate likewise run their adapter cleanup off the consumer.
  Cancelling a caller of `AdapterLifecycleSink::deliver_terminal` or of a core
  connection-failure publication no longer degrades the operational event
  stream.
- Media teardown no longer spins a runtime worker while it waits for media
  tasks on another runtime (#340). `MediaGraphHandle::shutdown_and_wait` and
  `wait_closed` await the graph actor's `JoinHandle`, the graph's terminal
  cleanup awaits the aborted sink delivery tasks, and
  `CrossBridgeHandle::stop` awaits both frame pumps, instead of polling
  `is_finished` in `yield_now` loops (up to 1–5 s each). Recording drains, on
  `ManagedMediaRoute::drain` and at source EOF, wake when the route's delivery
  task ends instead of polling every millisecond.
- Media graphs run on the media lane their streams publish
  (`MediaStream::media_lane`, #363). A graph whose source hosts reader graphs
  runs on the source's lane; a graph whose source has no lane (an in-process
  AI, for example) runs on the target's lane when the target hosts writer
  graphs; otherwise it runs on the caller's runtime as before. This covers
  directional bridge graphs and the per-connection graph used by listeners,
  broadcast and recording. Streams publish no lane by default (rvoip-sip
  publishes one under `MediaExecutorConfig::v2()`), so nothing moves unless a
  stream asks. New `start_media_graph_on(source, codec, policy, runtime)`
  starts a graph whose actor and route delivery tasks run on a given runtime.
- New `rvoip_core::Config::lane_inline_graph_delivery` (default `false`,
  `with_lane_inline_graph_delivery`, #363). For a bridge where exactly one
  side publishes a media lane, such as a SIP leg bridged to an in-process AI,
  graphs on that lane hand each frame to its target from the graph actor while
  the route's queue is idle, instead of waking the route's delivery task;
  targets receive the same frames in the same order, under the same
  peer-switch, cutover and gate fences. Per-graph totals are published at
  graph stop as `rvoip_media_graph_inline_deliveries_total` and
  `rvoip_media_graph_inline_fallbacks_total`.
- `rvoip-core-traits`: `MediaStream::try_gated_frames_out()` (default `None`)
  lets a stream offer a `GatedFrameInput`, its consumer channel plus the gate
  in front of it (`GatedFrameAdmission::{Open, Discard, Closed}`); a media
  graph routing into such a stream applies the gate itself and writes each
  frame straight into the consumer's channel, under the same peer-switch and
  cutover fences as any other route (#349). `PeerRouteTicket::forward_with`
  (`PeerForwarded`, #349) and `PeerRouteTicket::try_reserve`
  (`PeerTryReserve`, `PeerForwardPermit`, #363) are the matching peer-switch
  primitives for a producer that commits through its own gate and for a
  non-waiting forward.
- Media graph transcodes that change only the sample rate (for example G.711
  at 8 kHz to and from the in-process AI's 16 kHz PCM) resample directly
  instead of going through `FormatConverter`, which copied each frame twice;
  the output is bit-identical (#349).
- The media graph resolves its per-frame counters
  (`rvoip_media_graph_source_frames_total`, `rvoip_media_graph_frames_total`
  and the labelled `rvoip_media_graph_transcodes_total`) once per graph and
  codec group instead of looking each one up for every frame (#349). Names,
  labels and counts are unchanged; a metrics recorder installed after a graph
  started is not seen by that graph's counters.

### AI harness

- New `InProcessAiConfig::gated_input` (default `false`, #349). When set, each
  AI stream offers its caller-audio input as a gated input, so the bridge's
  media graph applies the hold/stop gate and writes caller audio into the AI
  session's input channel directly instead of through the route's input pump
  task for every frame. The session receives the same frames in the same
  order, and hold, resume and end behave the same. Combined with
  `rvoip_core::Config::lane_inline_graph_delivery`, caller audio reaches the
  AI session from the SIP leg's lane without any task hop (#363).

### Release process, CI and dependencies

- Dependency security bumps (#341): `quinn-proto` 0.11.19, `rustls` 0.23.46,
  `rustls-webpki` 0.103.16, `tokio` 1.54.1, and `webpki-roots` /
  `webpki-root-certs` 1.0.11, in the workspace and examples lockfiles.
  RUSTSEC-2026-0387 (`crypto-bigint` 0.5, transitive via the RustCrypto
  elliptic-curve 0.13 stack) is accepted in `deny.toml` until the
  elliptic-curve 0.14 upgrade, since its fix ships only in 0.7.6.
- RUSTSEC-2026-0459 (SQLite before 3.54 can return uninitialized page-cache
  memory for a crafted database and `-wal` pair) is accepted in `deny.toml`
  and the beta gate's `cargo audit`. It reaches rvoip through
  `libsqlite3-sys` 0.30, bundled by `sqlx-sqlite` in `rvoip-users-core`. The
  fixed `libsqlite3-sys` 0.39 is outside every sqlx release's range (0.9.0
  caps it below 0.38 and needs Rust 1.94), and exposure needs an
  attacker-written database file plus `-wal`; users-core opens only its own
  configured database. Revisit when sqlx allows the fixed line.
- `rvoip-rtp-core` now requires tokio 1.41 or later
  (`RuntimeMetrics::global_queue_depth`, #356).
- The orphan-sweep TTL test and the shared setup-teardown scheduler test no
  longer fail on starved CI runners: they wait for the reclaim or scheduler
  pass they assert on instead of checking it as soon as an earlier signal
  fires (#364).
- Several PRs in this release changed the `rvoip-sip` public API, so the
  baseline (`public-api/rvoip-sip.txt`) must be regenerated in the 0.5.0
  release commit.

## 0.4.0 — 2026-10-11

### Highlights

0.4.0 is a breaking minor release. RTCP is now fully wired: rvoip offers
`a=rtcp-mux` by default, sends RFC 3550-correct reports with a private CNAME,
filters RTCP from strangers, and can run RTCP on a separate port for peers that
decline mux. Per-call media quality, including the peer's RTCP view of the
stream we send, now reaches applications. `rvoip_sip::Config` gains deployment
profiles for public servers, UDP carrier trunks, NAT'd endpoints and mutual-TLS
peering, plus SIP OPTIONS keep-alive. RFC 4028 session timers and in-dialog
request routing are now conformant, and calls survive rejected refreshes.
`rvoip-core` can bound lifecycle retention for long-running workers, owns and
drains its periodic and playback workers, reports resource snapshots, and
encodes TTS for the destination codec. UCTP gains authenticated application
profiles over WebSocket and QUIC and an experimental JavaScript client. G.711
now has one implementation, which fixes corrupted audio from the utility
helpers and the tone generator. Upgrading from 0.3.12: read the
[0.4 migration guide](docs/MIGRATING_0.4.md); live interop with Asterisk 20 and
FreeSWITCH 1.10 is summarised in the
[release notes](crates/sip/rvoip-sip/docs/RELEASE_NOTES_NEXT.md).

### Breaking changes

Each item has a one-line migration; [docs/MIGRATING_0.4.md](docs/MIGRATING_0.4.md)
has before/after code for each.

- **`ConnectionId` and `ConversationId` are no longer tuple structs** (#259).
  They carry a private one-use lifecycle fence, so `ConnectionId(s)` and
  `id.0` no longer compile; newly minted IDs look like
  `conn_<incarnation>_<sequence>` instead of `conn_<uuid>`. Migration: use
  `ConnectionId::from_string(s)` / `ConversationId::from_string(s)` and
  `id.as_str()` or `id.to_string()`; never parse the ID text. `new`,
  `Default`, `Display`, `Eq`/`Hash`/`Ord` and the serde string form are
  unchanged.
- **Public configuration structs are `#[non_exhaustive]`**:
  `rvoip_core::Config`, `rvoip_core::TenantQuotas`, `rvoip_sip::Config`
  (and its `PeerConfig` alias), `rvoip_sip_dialog::api::DialogConfig`,
  `rvoip_websocket::UctpWsConfig`, `rvoip_quic::UctpQuicConfig`, and
  `rvoip_uctp::state::UctpCoordinatorCaps`. Code outside the defining crate
  can no longer build them with a struct literal, including the
  `..Default::default()` form, so later releases can add fields without a
  semver break. Migration: start from a constructor, then assign fields or
  call builders:

  ```rust
  // Before
  let config = Config { max_concurrent_setups: 64, ..Config::default() };

  // After
  let mut config = Config::default();
  config.max_concurrent_setups = 64;
  ```

  Use `Config::default()` (core), `Config::local` / `Config::on` or a
  profile constructor (SIP), `DialogConfig::new` or `Default`,
  `UctpWsConfig::new`, `UctpQuicConfig::new`, and
  `UctpCoordinatorCaps::default()`. `TenantQuotas` gains
  `with_max_concurrent_sessions`, `with_max_concurrent_recordings`, and
  `with_max_concurrent_ai_sessions`, so
  `TenantQuotas::default().with_max_concurrent_sessions(1)` replaces the
  literal. Reading fields and assigning to them is unchanged.
- **New configuration fields** (covered by the item above for constructor
  users): `rvoip_core::Config::capture_session_vcon` (default `true`);
  `rvoip_sip::Config::rtcp_mux_required`, `offer_rtcp_mux`, `rtcp_non_mux`,
  `rtcp_reduced_minimum_interval`, `rtcp_xr_voip_metrics`,
  `active_call_rtcp_counts_as_media`, `media_quality_interval`,
  `options_keepalive_targets`, `options_keepalive_interval_secs` and
  `sip_allow_tls_contact_on_sips`; `UctpWsConfig::application_handler`; and
  `UctpCoordinatorCaps::application_handler_timeout`. Migration: none when
  you start from a constructor; every default preserves 0.3.12 behaviour
  except the RTCP defaults below.
- **`TtsPlayback::audio_format()` is a new required method** (#291). rvoip
  now encodes and paces TTS output for the destination stream instead of
  forwarding payloads unchanged. Migration: return
  `TtsAudioFormat::PcmS16Le { sample_rate_hz }` for PCM output, or
  `TtsAudioFormat::Encoded { codec }` when the provider already emits one
  20 ms packet per frame in the destination codec.
- **`TtsRequest` gains `destination_codec: Option<CodecInfo>`** (#291).
  `play_audio` fills it with the stream's negotiated codec and leaves
  `sample_rate_hz` unset. Migration: add `destination_codec: None` to
  struct literals (or use `..Default::default()`), and read it in providers
  that can synthesize directly in the destination codec.
  `TtsProvider::synthesize_for_codec` and `media_graph::encode_pcm_prompt`,
  which existed only on `main` before this release, were removed again in
  favour of this field and the shared playback encoder.
- **`rvoip_sip::Event::MediaQualityChanged` gains `quality: MediaQualityStats`**
  (#306); its existing integer fields are kept. Migration: add `..` to
  exhaustive struct patterns, or read the new field.
- **`rvoip_sip::Event` gains `PeerReachabilityChanged { target, reachable,
  status_code }`** (#305). Migration: add an arm (or a wildcard) to
  exhaustive `match`es on `Event`.
- **Low-level INVITE authentication retry signatures.** rvoip-sip-dialog's
  `DialogManager::send_invite_with_auth_options` now takes
  `(dialog_id, InviteAuthRetryOptions)` instead of eight positional
  arguments, so the retry keeps the original registered-flow routes; and
  `rvoip_sip::internals::DialogAdapter::resend_invite_with_auth` gains
  `learned_min_se: Option<u32>`. `UnifiedDialogApi` and the rvoip-sip
  facade are unchanged. Migration: pass the retained
  `InviteAuthRetryOptions` (fields `sdp`, `authorization_headers`,
  `extra_headers`, `from_display`, `contact_uri`, `outbound_proxy_uri`,
  `supported_100rel`), and `None` for `learned_min_se` unless a 422 set it.
- **Quality structs gain fields** (#306, #304). `rvoip_core::QualitySnapshot`
  gains `rtt_ms`, `remote_packet_loss_pct` and `remote_jitter_ms`;
  infra-common's `MediaQualityMetrics` gains packet counters, `rtt_ms` and
  `remote_*`; media-core's `QualityMetrics` gains packet counters and
  `remote_*`; rtp-core's `RtpSessionStats` gains `rtcp_packets_received`,
  `rtcp_packets_rejected`, `peer_report` and `peer_bye`. Migration: add
  `..Default::default()` to struct literals (media-core's `QualityMetrics`
  has no `Default`; build it with `QualityMetrics::from_rtp_stats`).
- **media-core `MediaSessionInfo` gains `rtcp_port: Option<u16>`** (#307).
  Migration: add `rtcp_port: None` to struct literals, or start from
  `MediaSessionInfo::default()`.
- **RTCP defaults changed** (#303, #304). Every SDP offer now carries
  `a=rtcp-mux`; the periodic report interval follows RFC 3550 (five-second
  minimum, randomised) instead of one second, and `RTCP_MIN_INTERVAL` is now
  five seconds; RFC 3611 XR is off unless enabled; the SDES CNAME is a random
  per-session value instead of `$USER@hostname`; a call whose answer declines
  mux sends no RTCP at all (including no BYE) unless `Config::rtcp_non_mux`
  is on; inbound RTCP is accepted only from the call's peer. Migration:
  `Config::offer_rtcp_mux = false` restores the 0.3.x offer for a peer that
  mishandles the attribute; `Config::rtcp_xr_voip_metrics` or
  `RtpSession::set_rtcp_xr_enabled(true)` re-enables XR;
  `Config::rtcp_reduced_minimum_interval` opts into the RFC 3550 §6.2
  reduced minimum; read the CNAME from `RtpSession::cname()`.
- **RFC 4028 90-second floor** (#310). `Config::validate`, and so peer
  construction, rejects `session_timer_secs` or `session_timer_min_se`
  below 90 seconds and `session_timer_secs` below `session_timer_min_se`.
  Migration: use intervals of at least 90 s; tests that need second-scale
  timers enable the `test-hooks` feature and set
  `Config::session_timer_allow_short_intervals_for_testing`.
- **Profiles now enable session timers** (#305). `Config::carrier_sbc` and
  `Config::proxy_rtpengine` set `session_timer_secs = Some(1800)` and
  `session_timer_min_se = 90`. Migration: set `session_timer_secs = None`
  after the constructor to keep 0.3.12 behaviour.
- **`Config::freeswitch_internal` is deprecated** (`since = "0.4.0"`, #305).
  Migration: `Config::lan_pbx(name, bind, advertised)`; it only set
  `strict_codec_matching`, which every constructor already enables.
- **Dropping `UctpWsAdapter` now stops its listener** (#293). The accept loop
  used to keep running detached; now dropping the final adapter cancels
  admission and inbound peers, and `UctpWsServer` is no longer a
  constructible unit struct. Migration: keep the adapter alive for as long
  as it should serve, and stop it explicitly with `begin_drain()` and
  `shutdown(budget).await`.
- **Tenant quota updates are reconciled against configured capacity**
  (#275). Shrinking or removing a limit while its permits are held now
  returns an error instead of over-admitting, and limits beyond Tokio's
  semaphore capacity are rejected. Migration: drain affected work before
  shrinking or removing a busy tenant's limit.
- **G.711 utility output changed** (#273). `codec_core::utils` table, batch
  and SIMD helpers and media-core's tone generator now produce standard
  G.711 bytes and samples; they used to disagree with the canonical codec
  for every input. Migration: none for standard peers; anything that
  stored or compared the old utility output must regenerate it.

### SIP

#### RTP/RTCP multiplexing offer (#303, #260)

- rvoip-sip now offers RTP/RTCP multiplexing by default. Every SDP offer it
  generates (initial INVITE, re-INVITE and UPDATE, hold and resume, the
  late-SDP offer in a 200 OK, and the session-less hold/active fallback)
  carries `a=rtcp-mux` (RFC 5761), so periodic RTCP reports flow on the
  single media socket whenever the peer answers with `a=rtcp-mux`. Before
  this, offers omitted it and RTCP never flowed on calls rvoip originated.
  Answers are unchanged: they echo `a=rtcp-mux` only when the offer had it.
- When the answer declines mux, the call carries no RTCP at all, per RFC 5761
  §5.1.1: no periodic reports, and no RTCP BYE at teardown, either to the RTP
  port or to RTP port + 1. rtp-core's `RtpSession::close` now sends its BYE
  only when mux was negotiated. Without `Config::rtcp_non_mux`, offers carry
  no `a=rtcp:` fallback port.
- Opt out with `Config::offer_rtcp_mux = false` for a peer that mishandles
  the attribute; offers then match 0.3.x. `Config::rtcp_mux_required`
  (added in this release, default `false`) is the strict mode: offers carry
  `a=rtcp-mux` and `a=rtcp-mux-only`, and offers or answers without
  multiplexing fail before the staged negotiation commits. It overrides the
  opt-out and always offers mux.

#### Deployment profiles and OPTIONS keep-alive (#305)

- `rvoip_sip::Config` has a profile constructor for each common way to
  deploy a SIP server or endpoint. A profile only fills in documented
  defaults: everything it sets stays a public field you can change
  afterwards, and each profile's rustdoc lists the fields it sets.
- New `Config::carrier_trunk_udp(name, bind, public, trunk_proxy_uri)`: a
  plain UDP trunk authenticated by source IP. Public signaling and media
  address, outbound proxy to the trunk SBC, playout buffer, 1800 s session
  timers; no TLS, no SRTP, no REGISTER.
- New `Config::public_server(name, bind, public)`: a server on a public or
  1:1-NAT address. Advertised SIP and media addresses, ICE Lite, playout
  buffer, 1800 s session timers. Chain `tls_reachable_contact(...)` for a
  TLS listener.
- New `Config::behind_nat(name, bind, stun_server, sip_instance)`: an
  endpoint behind NAT. TLS RFC 5626 registered flow with CRLF keep-alive,
  STUN, ICE Full, `rtcp_mux_required = true`, playout buffer.
- New `Config::tls_direct_routing(...)`: an SBC peering over mutual TLS,
  modelled on Microsoft Teams Direct Routing's SBC requirements (not
  certified or tested against Teams). TLS listener with an FQDN Contact,
  required client certificates, SDES-SRTP required, ICE Lite, OPTIONS
  keep-alive to the peer, 1800 s session timers, no REGISTER.
- `Config::carrier_sbc` now turns on RFC 4028 session timers
  (`session_timer_secs = Some(1800)`, `session_timer_min_se = 90`), so a
  call whose far end vanished without a BYE ends within 30 minutes.
- `Config::proxy_rtpengine` is no longer a placeholder: it adds the playout
  buffer and 1800 s session timers. Its docs now say plainly that rvoip-sip
  does not speak RTPengine's `ng` control protocol; the proxy anchors media.
- `Config::freeswitch_internal` is deprecated in favour of
  `Config::lan_pbx`. It only set `strict_codec_matching`, which every
  constructor already enables. `Config::local_lab` stays, documented as an
  alias of `Config::local`.
- New `Config::options_keepalive_targets` and
  `Config::options_keepalive_interval_secs` (default 60): the coordinator
  pings each target with an out-of-dialog `OPTIONS` once per interval,
  carrying `Config::contact_uri` as its Contact, and publishes the new
  `Event::PeerReachabilityChanged { target, reachable, status_code }` on the
  first outcome and on every change; 408, 503 or no response counts as
  unreachable. Off unless targets are listed.
- `Config::tls_reachable_contact` now advertises the TLS listener on
  `Config::sip_advertised_addr`'s IP when that is set and
  `Config::tls_advertised_addr` is not, instead of leaving a wildcard bind
  unadvertised or advertising a private bind address.
- `Config::validate` rejects `session_timer_secs` below
  `session_timer_min_se` and `options_keepalive_targets` entries that are not
  SIP URIs.

#### Session timers (RFC 4028) and in-dialog routing (#309, #310)

- As the UAS, a 2xx no longer carries `Require: timer` when the INVITE did
  not advertise `timer` in `Supported` or `Require` (§9). rvoip still runs
  the timer for such callers and names itself the refresher
  (`refresher=uas`), including when a proxy inserted `Session-Expires`. A
  caller that supports timers but proposes an interval below the local
  Min-SE gets 422 with `Min-SE`. The answered interval is never below the
  request's `Min-SE`; a large peer Min-SE now raises the interval instead of
  rejecting the call with 422.
- Session-timer headers are added only to 2xx answers to INVITE and UPDATE.
  A 2xx to BYE, INFO or another method no longer carries `Session-Expires`
  or `Require: timer`.
- The non-refresher now sends its `BYE` (`Reason: SIP;cause=408`) at
  `interval - min(32, interval / 3)`, as RFC 4028 §10 recommends, instead of
  at the full interval. The refresher still refreshes at half the interval.
- Refresh requests now always carry `refresher=uac`. The parameter names a
  role in the refresh transaction, so a refresher that answered the call
  used to send `refresher=uas` and hand the job to its peer (§5, §7.4).
  Refreshes also carry the largest `Min-SE` received in a 422 on the dialog.
- When the peer rejects the refresh UPDATE (for example 405 from a peer that
  does not support UPDATE), the re-INVITE fallback now reaches the wire. It
  used to carry a second `Session-Expires` and `Min-SE` and have no offer, so
  it never got built and the call was torn down with a 408 BYE at the first
  refresh. The refresh re-INVITE now re-offers the current local SDP
  unchanged (§7.4).
- Other re-INVITEs (hold, resume, renegotiation) now carry the negotiated
  interval and keep the current refresher. They used to propose the
  configured interval with `refresher=uac`, which could hand the refresh to
  the side that was not running a refresh timer.
- A rejected session refresh no longer ends the call (RFC 4028 §10). Only a
  refresh that times out, whose transport fails, or that draws 408 or 481
  ends the session with a `Reason: SIP;cause=408` BYE; such an UPDATE no
  longer tries a re-INVITE first. Any other non-2xx used to tear the call
  down too. Now 491 Request Pending is retried with the same method after the
  RFC 3261 §14.1 backoff (2.1–4 s for the side that placed the call, 0–2 s
  for the other). 422 is retried at once with the response's `Min-SE` as both
  the floor and the minimum interval (§7.4). A rejected UPDATE still falls
  back to re-INVITE. Any other rejection of the re-INVITE (488, 403, 5xx, …)
  keeps the call and logs a warning. One more refresh is attempted halfway to
  the BYE deadline (at most once per interval, within the four-retry cap);
  if that is rejected too, the session expires at the end of the current
  interval unless the peer refreshes it or another re-INVITE or UPDATE
  succeeds first.
- The 2xx to a session refresh now renegotiates the timer (RFC 4028 §7.2,
  §7.4). Its `Session-Expires` interval and refresher replace the old ones,
  so a peer can lengthen the interval or take over refreshing. A 2xx without
  `Session-Expires` to a refresh that proposed one turns the timer off. Both
  used to be ignored.
- An incoming refresh is answered with its own interval and refresher
  (§9), within the local Min-SE, and the new values take effect. The 2xx used
  to echo the dialog's previous values. A timer-capable peer whose refresh
  asks for less than the local Min-SE now gets 422 with `Min-SE`, as on the
  initial INVITE.
- Refreshes use UPDATE only when the peer has not ruled it out: a peer whose
  `Allow` header (from its INVITE, re-INVITE, UPDATE or 2xx) omits UPDATE is
  refreshed with re-INVITE from the start. A held call can now be refreshed
  with re-INVITE; that refresh used to be dropped and the call expired.
- RFC 4028 §5 floor: `Config::validate` (and so peer construction) rejects
  `session_timer_min_se` below 90 seconds, `session_timer_secs` below 90
  seconds, and a `session_timer_secs` below `session_timer_min_se`. A caller
  without timer support whose request carries a proxy-inserted
  `Session-Expires` below the local Min-SE is answered with the Min-SE
  instead of the smaller value; a timer-capable caller still gets 422.
  Tests that need second-scale intervals set
  `Config::session_timer_allow_short_intervals_for_testing`, which exists
  only with the `test-hooks` feature; every rvoip-sip CI lane now builds with
  `test-hooks` so those tests and process fixtures run.
- In-dialog UPDATE and re-INVITE now use the remote target (the peer's
  `Contact`) as the Request-URI, as RFC 3261 §12.2.1.1 requires. They used
  the peer's From/To URI, so a refresh sent by rvoip as the called party
  could go to the wrong host.
- In-dialog REFER, MESSAGE, INFO, NOTIFY, OPTIONS and every other in-dialog
  request now use the remote target (the peer's `Contact`) as the
  Request-URI, as RFC 3261 §12.2.1.1 requires; the To header keeps the
  peer's address. Blind and attended transfers used to send the REFER to the
  host in the peer's From/To URI. PRACK now targets the early dialog's
  remote target, which is taken from the reliable 18x `Contact`.

#### Authentication retries

- An INVITE that is authenticated after a 422 now keeps the interval the 422
  asked for (RFC 4028 §7.4). The 401/407 retry used to fall back to the
  configured `Session-Expires`, so a PBX with a higher Min-SE (FreeSWITCH
  with Min-SE 120 behind proxy auth) answered it with a second 422. That
  cost a round trip and spent the two-retry 422 budget, and one more
  challenge, such as a stale nonce, failed the call with 422. A later 422
  with a smaller `Min-SE` no longer lowers the floor already learned.
- A 401/407 retry of an INVITE to a registered contact (RFC 5626, as used by
  the `carrier_sbc` and `behind_nat` profiles) now goes out on the same
  registered flow as the original INVITE. dialog-core's
  `send_invite_with_auth_options` used to drop the flow routes and resolve
  the contact address instead, which sits behind the client's NAT. The
  core method now takes the retained `InviteAuthRetryOptions` and passes
  them through unchanged. In-dialog and REGISTER retries already reused
  their stored request options.
- Digest credentials retained across INVITE retries are re-signed rather
  than resent (RFC 7616 §3.4). The 422 retry, and a 401 retry that keeps an
  earlier proxy credential, now carry the next `nc` for that nonce and a new
  `cnonce`. They used to repeat `nc=00000001`, which registrars and proxies
  that track nonce counts reject as a replay. A new nonce, including one
  from a `stale=true` challenge, still starts at 1. REGISTER refreshes
  already counted correctly and now have a test.

#### Transport, TLS and dialog robustness

- Re-INVITE answers could be lost to a race on fast networks. The state
  lane that sends a re-INVITE stores the pending offer when the lane is
  released, after the transport write returns, and a peer on a LAN or
  loopback could answer inside that window. A 200 OK was then taken for a
  retransmission (ACKed and dropped), and a 488 was rolled back unreported,
  so `renegotiate_media` waited until its deadline. Final responses to a
  client re-INVITE, and matched UPDATE/re-INVITE completions, now wait for
  the sending lane before correlating with the pending offer.
- Shutdown no longer fails when a BYE was answered just before it.
  dialog-core answers a BYE with 200 OK and then delivers `ByeReceived` to
  the session layer from a cleanup job; shutdown closed the session layer's
  event intake first, so that delivery was quarantined and dialog stop
  failed permanently with `dialog-stop`. `run_shutdown_sequence` now calls
  the new `drain_bye_cleanup_before_shutdown` (bounded at 5 s): it stops
  accepting BYEs, waits for in-progress BYE handlers and their cleanup, then
  closes the session layer.
- An outbound INVITE whose first TLS write failed could hang
  `OutboundCallBuilder::send()` forever, for example when a TLS 1.3 server
  rejects the client certificate with `certificate_required` just after the
  handshake. Three cleanup races are fixed. A closed or exhausted failover
  plan with no live plan now counts as the INVITE's terminal failure
  (RFC 3261 §8.1.3.1). An INVITE the transaction layer dropped before its
  first write began, or whose route is pinned to a connection that has
  closed, is treated as unreachable, so no CANCEL is attempted or awaited
  for it. Under CPU contention such failures now settle in about 300 ms.
  As a backstop, `send()` waits at most 5 s for rollback while release
  continues as a retained lifecycle task. A call whose INVITE did reach the
  wire before the connection closed can still take until Timer B (32 s) to
  report failure; see Known issues.
- Outbound SIP resolution no longer holds a dialog shard write lock while
  awaiting DNS (#264). A slow resolver used to block dialog lookups on that
  shard during BYE, initial INVITE, PRACK, and initial or refreshed
  SUBSCRIBE; routing data is now captured under the lock and the lock is
  released before resolution. No public API change.
- An initial INVITE rejected while its early dialog is being created now gets
  exactly one final response (#283): 400 for protocol validation failures,
  500 for other early-dialog failures, sent before any dialog or application
  setup event exists. Fixed, payload-free diagnostic reasons name the known
  setup failures; peer or provider text never enters them.
- Opt-in TLS Contact compatibility (#287): `Config::sip_allow_tls_contact_on_sips`
  (default `false`, or `RVOIP_SIP_TLS_CONTACT_COMPATIBILITY=true|false`)
  accepts a SIPS INVITE whose Contact is an explicit
  `sip:...;transport=tls` URI when it arrived on an observed TLS server
  transaction. Only the internal remote target is normalised to SIPS; a
  claimed TLS Via over another transport is not enough, and the advertised
  Contact stays SIPS.
- For dialogs admitted that way with no route set and a Contact IP matching
  the observed peer, in-dialog requests such as BYE reuse the accepted TLS
  connection instead of dialling the advertised Contact (#288). The flow is
  used only while the request and remote target still equal the pinned
  Contact and no Route header exists; a dead flow fails rather than
  reconnecting, and flow identity is process-local and never restored from
  persisted dialogs.
- Structured response diagnostics keep the analytics-block redress profile
  (#290): for `Reason: SIP;cause=603;v=analytics1` only, a bounded location,
  HTTP(S) redress URL (credentials, query and fragment stripped), telephone
  and email are projected; other parameters are still discarded. `url` is
  now a normal dependency of rvoip-sip.
- The inbound `MediaStream` pump reframes variable-duration Opus (2.5–120 ms
  packets) into the encoder's 20 ms frames before re-encoding (#286), so
  valid non-20 ms packets no longer fail encoding and drop audio from a
  bridge. Partial PCM is reset on discontinuities, invalid shapes and codec
  changes; input over 120 ms is rejected.

#### SIP documentation

- `rvoip_sip::Config::playout` now documents the inbound jitter buffer in
  full: what it does, the `PlayoutConfig` knobs with defaults and units, its
  latency cost (~40 ms at the default depth), when to enable it (routes over
  the public internet, carrier trunks) and when to leave it off (LAN, lab),
  with compiled examples. `PlayoutConfig` and its fields in
  `rvoip-media-core` are documented to match. No defaults changed.
- `Config::rtcp_mux_required` is now the reference for RTCP behaviour: it
  describes both paths, multiplexed by default and a separate port with
  `rtcp_non_mux`, and what a peer that declines mux loses without it (no
  RTCP quality statistics; SBCs that use RTCP for dead-media detection may
  tear the call down).
- The rvoip-sip README gains a "Media options" section with a decision table
  naming the profile and settings to start from for each deployment shape,
  and an RTCP section that points non-mux PBXes and carriers at
  `rtcp_non_mux`. `docs/MEDIA_QUALITY.md` documents the media quality API
  and `docs/TUNING.md` gains "RTCP Reporting".
- `docs/SIP_DIAGNOSTIC_OBSERVER_PROPOSAL.md` proposes application-scoped SIP
  diagnostic observation (#281). It is a design proposal, not an
  implemented API.

### RTP/RTCP and media quality

#### RFC 3550 reporting and inbound filtering (#304, #260)

- The periodic RTCP task now starts whenever a report generator exists, even
  before SDP supplies a peer, skips transmission until a peer is known, and
  resolves the current destination on each tick (#260). Calls whose remote
  address arrives in late SDP, or whose peer changes, keep reporting, and
  reports stop when the session closes.
- Sender Reports carry the RTP timestamp of the stream's media clock at the
  report's NTP time — the last sent media timestamp extrapolated at the clock
  rate (RFC 3550 §6.4.1) — instead of wall-clock milliseconds (periodic) or a
  frozen scheduler value (`send_sender_report`). Telephone-event packets do
  not move the anchor.
- A session that has not sent RTP in the last two report intervals sends a
  Receiver Report with its report blocks instead of an SR with zero counts
  (RFC 3550 §6.4).
- The periodic report interval follows RFC 3550 §6.2/§6.3 and Appendix A.7:
  5% of the session bandwidth (derived from static payload types, else
  80 kbit/s; `RtpSession::set_bandwidth` overrides), members and senders from
  observed SSRCs, a five-second minimum halved before the first report,
  randomisation over [0.5, 1.5] and the e − 3/2 compensation. Reports
  previously went out every second. `RTCP_MIN_INTERVAL` is now five seconds.
  New `Config::rtcp_reduced_minimum_interval` (default `false`) enables the
  §6.2 reduced minimum (360 / session kbit/s).
- Every report and the close-time BYE carry an SDES CNAME. The CNAME is a
  random 96-bit base64 value per session (RFC 7022) rather than
  `$USER@hostname`; `RtpSession::cname()` returns it. The `hostname`
  dependency is removed.
- RFC 3611 VoIP-metrics XR is no longer appended to every report. It is off
  by default; `RtpSession::set_rtcp_xr_enabled` switches it per session, and
  new `Config::rtcp_xr_voip_metrics` (default `false`) enables it for calls
  whose peer SDP carries `a=rtcp-xr` naming `voip-metrics`. Media-core
  carries both policies as `RTCP_XR_PARAMETER` and
  `RTCP_REDUCED_MINIMUM_PARAMETER`.
- Inbound RTCP is accepted only from the call's expected peer: its signalled
  or latched address, or a remote SSRC already sending RTP. A neighbouring
  call's non-mux peer sending RTCP to its RTP port + 1 — this session's RTP
  port under consecutive allocation — no longer feeds this call's reports,
  RTT, or BYE handling. `RtpSessionStats` gains `rtcp_packets_received` and
  `rtcp_packets_rejected`.
- New `Config::active_call_rtcp_counts_as_media` (default `false`) lets RTCP
  from the call's peer count as activity for the active-call no-media and
  media-idle watchdogs, so held or silent calls that still report are kept.

#### Separate RTCP port for peers that decline mux (#307)

- New `Config::rtcp_non_mux` (default `false`, builder `with_rtcp_non_mux`)
  gives each call an even RTP port and RTP port + 1 for a separate RTCP
  socket (RFC 3550 §11), reserved together so concurrent calls never
  collide. Calls rvoip offers reserve the pair up front; inbound calls only
  when the offer lacks `a=rtcp-mux`. Offers carry `a=rtcp:<port>` (RFC 3605)
  beside `a=rtcp-mux` (never beside `a=rtcp-mux-only`), and answers to
  non-mux offers carry it too. When the peer declines mux, periodic SR/RR and
  the close-time BYE go from the RTCP port to the peer's `a=rtcp:` address
  or its RTP port + 1, never to its RTP port; inbound RTCP on the port gets
  the same peer filter, SRTCP covers it under SDES-SRTP, and a NAT-mapped
  peer RTCP source is learned only from the latched RTP stream's SSRC on the
  same IP. When the peer accepts mux, the RTCP port is released when the
  negotiation commits. A call that keeps its RTCP port uses two ports, which
  halves capacity per media port range. Not used with ICE, DTLS-SRTP keying,
  strict `rtcp_mux_required`, or signalling-only media. Enables PBXes such as
  Asterisk with default pjsip settings, and carriers without mux, to get RTCP.
- rtp-core: `PortAllocator::allocate_rtp_rtcp_pair` and
  `release_session_port`; `RtpSession::new_event_driven_with_rtcp_socket`,
  `set_remote_rtcp_addr`, `local_rtcp_addr` and `release_rtcp_socket`;
  `UdpRtpTransport::set_rtcp_mux`, `release_rtcp_socket` and
  `local_rtcp_socket_addr`. media-core: `RTCP_SEPARATE_PORT_PARAMETER`,
  `REMOTE_RTCP_ADDR_PARAMETER`, `MediaSessionInfo::rtcp_port` (new public
  field) and `MediaSessionController::release_rtcp_port`.

#### Per-call media quality reaches the application (#306)

- `rvoip-sip` adds `UnifiedCoordinator::media_quality(&SessionId)` and
  `SessionHandle::media_quality()`, returning `MediaQualityStats`
  (`#[non_exhaustive]`): packets sent/received/lost, local loss percent,
  jitter (ms) and MOS estimate for the received stream, plus `rtt_ms`,
  `remote_packet_loss_percent`, `remote_packets_lost` and `remote_jitter_ms`
  from the peer's RTCP reports about the stream we send. Peer-reported
  fields are `None` until RTCP arrives and stay `None` for calls without
  RTCP (no negotiated `a=rtcp-mux` and no `rtcp_non_mux` port).
- New `Config::media_quality_interval` (`with_media_quality_interval`),
  default `None`: when set, every call with media publishes
  `Event::MediaQualityChanged` on that cadence. Previously that event had no
  production source. The event gains a `quality: MediaQualityStats` field
  (breaking for exhaustive patterns); its existing integer fields are kept.
  A zero interval fails `Config::validate`.
- `rvoip-core`: `QualitySnapshot` gains `rtt_ms`, `remote_packet_loss_pct`
  and `remote_jitter_ms` (`Option`s; breaking for struct literals — add
  `..Default::default()`). With the SIP sampler on, SIP media streams report
  `has_quality_measurement() == true`, `Event::MediaQuality` fires for SIP
  connections, and `spawn_media_quality_sampler` averages the optional fields
  only over streams that carry them. `RvoipAppBuilder::media_quality_interval`
  now also enables the SIP sampler at the same cadence.
- `rtp-core`: `RtpSessionStats::peer_report` retains the latest RTCP report
  block a peer sent about our SSRC (`PeerReceptionReport`: reporter SSRC,
  fraction and sign-extended cumulative loss, extended highest sequence,
  jitter in timestamp units and ms, LSR/DLSR RTT, receive time), and
  `RtpSessionStats::peer_bye` records an inbound RTCP BYE. Previously these
  were only logged.
- `media-core`: `QualityMetrics` gains packet counters and `remote_*`
  fields (`QualityMetrics::from_rtp_stats`), the cross-crate
  `MediaQualityMetrics` gains counters, `rtt_ms` and `remote_*`, and
  `MediaSessionController::get_media_quality` /
  `publish_media_quality_updates` sample calls. `MediaEventHub` now maps
  `StatisticsUpdated` and `QualityDegraded` to
  `MediaQualityUpdate` / `MediaQualityDegraded` with the real session id,
  and the legacy `MediaEventAdapter` no longer publishes a fabricated MOS
  for an `"unknown_session"`.

### Media and codecs

- One G.711 implementation for every build and caller (#273, refs #257).
  The public `codec_core::utils` scalar, table, batch and SIMD helpers used
  a duplicate algorithm that disagreed with the canonical codec for all
  65,536 PCM inputs and all 256 encoded bytes (PCM silence became `0x4d`
  instead of `0xff`). They now delegate to the ITU-T reference module, which
  is compiled unconditionally as `codecs::g711_reference` so
  `--no-default-features` builds and the fuzz crate compile, and the static
  decode tables are generated from it. Public signatures and table symbols
  are unchanged; the A-law scalar decoder is now `const`. All encodes and
  decodes for both laws match an independent port of ITU-T G.191 `g711.c`.
- media-core's tone generator used its own simplified mu-law encoder that
  mapped silence to full-scale negative, so every negotiated codec carried
  distorted tone audio. It now uses the reference encoder and has an SNR
  regression test. The uncompiled media-core `codec/g711.rs` tables were
  removed.
- Decoded stereo playout advances the RTP timestamp by samples per channel
  (#276). A 20 ms 48 kHz stereo frame used to advance 1,920 ticks instead
  of 960 and skip the following packet.
- The single-speaker RTP receive path normalises a replacement SSRC's
  independent RTP clock onto the call timeline and gives it a fresh decoder
  (#285). A replacement source needs two advancing sequential packets;
  late or duplicate packets are rejected before decode, and the new decoder
  is committed only after a successful decode, so a peer that swaps SSRC no
  longer causes a clock jump or Opus state carried over from the old
  source.

### Core and orchestrator

- Bounded lifecycle retention (#259). `Orchestrator::configure_bounded_connection_lifecycles(maximum)`,
  called before adapters are registered, bounds the retained connection,
  session and conversation rows instead of cumulative calls; published
  0.3.12 kept every retired connection ID for the life of the process and
  failed closed at 262,144 admissions. IDs are minted with a
  process-incarnation namespace, a checked monotonic sequence and a shared
  one-use fence, and in bounded mode only freshly minted IDs can create a
  connection or conversation; string and serde copies remain valid for
  lookups. Rows are reclaimed after cleanup completes. Compatibility mode,
  the default, keeps the fail-closed tombstones.
- `Orchestrator::release_closed_conversation` releases a closed conversation,
  its ended sessions and tenant membership once the application has
  archived what it needs. In bounded mode applications must call it after
  every conversation teardown; closed history counts against the budget
  until released, and neither `close_conversation` nor the idle closer
  releases it. `bounded_connection_lifecycles_enabled` reports the mode.
- `Orchestrator::connection_id_budget_usage()` returns the retained
  connection-ID count and limit for monitoring and worker rotation (#265).
  It is a read-only snapshot; it releases nothing.
- New `Config::capture_session_vcon` (default `true`) lets deployments
  without a vCon exporter turn off the default in-memory session vCon
  capture. `ConversationOpened` and `SessionStarted` are emitted after the
  bounded registry lock is released.
- Tenant quota updates are serialised with recording and AI permit
  reservation and reconciled against the previously configured total, and
  both limits are validated before either changes (#275). Repeating a limit
  is idempotent while permits are held, increases add capacity, decreases or
  removal require the affected permits to drain, and limits beyond Tokio's
  semaphore capacity are rejected without panicking.
- Periodic SDK workers (media quality sampler, idle closer, capacity
  scheduler) run under a bounded supervisor (#277): one worker per role
  (first cadence wins), zero intervals rejected, only a weak owner held
  between ticks, and a terminal `drain_periodic_tasks()` integrated into
  lifecycle shutdown. New fallible `try_spawn_media_quality_sampler`,
  `try_spawn_idle_closer` and `try_spawn_capacity_scheduler`; the existing
  `spawn_*` methods log failures. A worker that exited, for example by
  panicking, is replaced on the next start request.
- TTS playback can be cancelled while it waits for transport queue capacity,
  not only while reading the source (#282). Playback workers are owned by a
  bounded supervisor (at most max(64, 4 × setup capacity)), fenced against
  connection teardown, cancelled on terminal teardown, and drained with
  `drain_playback_tasks()`; `playback_task_count()` reports them.
  Completed/Cancelled/Failed outcomes are preserved; Completed means the
  transport queue accepted the audio, not that the peer played it.
- `Orchestrator::play_pcm(connection_id, PcmPlaybackSource)` plays
  caller-fed mono PCM through the negotiated audio stream (PCMU, PCMA and
  feature-enabled Opus), resampling with media-core and pacing 20 ms frames
  on a fixed schedule; `PcmPlaybackSource::channel(rate, capacity)` returns
  a bounded `PcmPlaybackSender`.
- TTS is codec-aware (#291). `TtsRequest::destination_codec` carries the
  stream's negotiated codec, and `TtsPlayback::audio_format` declares PCM
  (`TtsAudioFormat::PcmS16Le`) or already-encoded output
  (`TtsAudioFormat::Encoded`). PCM is re-framed to 20 ms, zero-padded and
  encoded through the same encoder as `play_pcm`; encoded output must match
  the destination by name, clock rate and channel count and is re-stamped
  with the destination stream id, payload type and RTP timestamps.
  Mismatches fail `play_audio` and cancel the provider. Playback paces on a
  fixed 20 ms schedule, so per-frame work no longer accumulates as drift and
  a stalled source re-anchors instead of bursting its backlog.
- `Orchestrator::resource_snapshot` returns a bounded, identifier-free
  census (#289): live versus retained-terminal sessions and conversations,
  retained and retired connection IDs, cleanup quarantines, periodic and
  playback workers, and per-adapter counts from the new default
  `ConnectionAdapter::resource_snapshot` hook, polled concurrently under one
  deadline. Adapters report `Reported`, `Unsupported`, `TimedOut` or
  `Failed`, and unmeasured fields stay `None` rather than zero. The SIP and
  WebRTC adapters report their route, media/port and task registries.
- rvoip-vapi socket write failures carry structured, payload-free
  diagnostics (#263): connection ID, media or control classification,
  sanitised socket error class, preceding timeout count and, for sustained
  stalls, elapsed time, frame sizes, message and byte counts and writer
  queue depths. Public failure reasons, deadlines and call lifecycle are
  unchanged.

### UCTP, WebSocket, QUIC and JavaScript

- Experimental authenticated UCTP application profiles (#262). A host
  installs one `ApplicationHandler` before ingress; an envelope's
  `payload.profile` selects it, and the auth challenge advertises the
  installed profile. Version, signature, authentication, authorisation and
  required-scope checks stay in the coordinator; the handler receives the
  whole envelope, the authenticated principal, a bounded output channel and
  a peer-close cancellation token. Duplicate envelope IDs go to `replay`
  (default: reject) instead of running `handle` again. Without a handler,
  envelopes carrying a string `payload.profile` keep the legacy dispatch.
  Each handler call is bounded by
  `UctpCoordinatorCaps::application_handler_timeout` (default
  `APPLICATION_HANDLER_TIMEOUT`, 5 s); a handler that never completes yields
  `error 504 transient/application-handler-timeout`. The host still owns
  schema validation, recipient authorisation and durable idempotency.
- WebSocket wires the handler in through `UctpWsConfig::with_application_handler`
  and exposes owner-scoped inbound Conversation/Session/medium hints for
  host admission (not an authorisation grant). QUIC gains
  `UctpQuicConfig::with_application_handler` with the same semantics (#292).
- `rvoip-websocket` has a runnable `application_profile` example host and
  Rust client with a Python loopback runner and a dedicated CI job (#272).
- `UctpWsAdapter` owns its inbound listener (#293): `begin_drain()` stops
  admission and releases the port while established peers continue,
  `is_draining()` reports it, and `shutdown(budget).await` cancels
  admission, incomplete TLS/WebSocket upgrades and active peers and returns
  `true` only once cleanup completes (a timed-out call returns `false`;
  cleanup continues and a later call can wait again). Dropping the final
  adapter requests cancellation. Outbound `originate` clients are outside
  this lifecycle.
- `sdk/uctp-js` is an experimental, private ESM source package with
  TypeScript declarations for browser and Node 22+ UCTP WebSocket clients
  (#271): UCTP v1 bearer negotiation, optional pre-credential
  application-profile discovery, correlated requests and events, bounded
  pending and frame limits, explicit reconnect and diagnostic redaction.
  It never repeats an effect automatically, has no runtime dependencies,
  is not published to npm, and has its own pinned Node/TypeScript CI job.

### Release process, CI and dependencies

- Every release must now have a `CHANGELOG.md` entry (#296). **Prepare
  release PR** refuses to run while `## Unreleased` is empty and moves its
  entries under the new `## X.Y.Z — YYYY-MM-DD` heading. Verification and
  publication reject a release without a non-empty section for its version.
- The release metadata check now scans every live Markdown, TOML and Rust
  file, not only Cargo manifests, for rvoip dependency snippets whose
  version does not match the workspace, including partial requirements
  such as `"0.3"`, renamed `package = "rvoip-…"` entries and snippets inside
  doc comments. Archived plans and versioned migration guides are frozen as
  history. The `rvoip-sip-dialog` and `rvoip-sip-registrar` READMEs, whose
  snippets had stayed at 0.3.10, are now updated by Prepare. Prepare also
  re-runs the check after rewriting versions. The rvoip-vcon README leaves
  the list: it has no dependency snippet, and its vCon wire-format version
  (`vcon: "0.4.0"`) would otherwise be rewritten by a later release.
- The rvoip-sip public API baseline (`public-api/rvoip-sip.txt`) is
  regenerated for this release and `check_public_api.sh` compares against
  `v0.3.12`, the latest published tag, instead of `v0.3.7`.
- `hickory-resolver` 0.26.1 → 0.26.2 in the workspace and examples
  lockfiles (#294, #295). GitHub Actions bumped (#178): `actions/checkout`
  7.0.1, `upload-artifact` 7.0.1, `download-artifact` 8.0.1, `setup-node`
  7.0.0, `taiki-e/install-action` 2.87.22, and updated
  `cargo-deny-action` and `attest-build-provenance` pins. The examples
  lockfile records rvoip-sip's `url` dependency (#301) and is refreshed for
  the integrated dependency graph.

### Known issues

- A TLS or TCP connection that closes after an INVITE was written does not
  fail the pending INVITE transaction at once; the call fails when Timer B
  fires (32 s by default). Tracked in #311.
- `Config::rtcp_non_mux` is ignored with DTLS-SRTP keying, which would need
  a second DTLS handshake on the RTCP port (RFC 5764 §4.1); such calls get
  no RTCP when the peer declines mux. A re-INVITE that drops mux mid-call
  also leaves that call without RTCP.
- RTCP does not implement RFC 3550 §6.3.3 timer reconsideration, a BYE does
  not decrement the member count, and dynamic-payload codecs use an
  80 kbit/s bandwidth default for the report interval.
- The deprecation and removal schedule for the duplicate G.711 utility
  symbols, and live receiver validation, remain open in #257.
- The grouped dependency update in #297 was deferred to a later release.

## 0.3.12

### 0.3.12 release recovery

- The protected 0.3.11 publication stopped after 25 of 46 crates reached
  crates.io. No `v0.3.11` tag or GitHub release was created.
- `rvoip-core` now keeps its forward `rvoip-harness` test dependency path-only,
  so Cargo can package core before harness exists on crates.io. Release
  planning detects cycles involving versioned test dependencies before the
  first upload.

### Release workers move to AWS

- The `remote-release`, `remote-preflight`, and `remote-diagnostic` profiles
  now run their ephemeral workers on EC2 (`m5.large` / `m5.xlarge` /
  `m5.4xlarge`, gp3 root volumes) in a dedicated release VPC instead of
  Google Compute Engine. Resource classes are renamed `gcp-*` to `ec2-*`,
  the planner emits `aws_matrix` / `aws_shard_count`, and evidence, logs,
  and the performance prebuild cache live in S3 (`s3://`) rather than GCS.
- `scripts/release/aws_fanout.py` replaces `gcp_fanout.py`. It keeps the
  `prepare`, `verify`, and `early-failure-decision` contracts (schemas
  `rvoip-ec2-release-fanout-v1` / `rvoip-ec2-release-shard-v1`, treating
  `stopping`, `stopped`, `shutting-down`, and `terminated` as finished) and
  adds `user-data`, which renders the gzip-compressed EC2 user-data that
  writes `/etc/rvoip-release.env`, installs the reviewed startup and
  shutdown scripts, and registers the `rvoip-release-shutdown.service`
  checkpoint.
- The controller authenticates with GitHub OIDC role assumption; no cloud
  key is stored in GitHub. The GCP qualification pilot workflow and its
  startup script are removed.
- The release environment identifier becomes
  `rvoip-release-v6-rust-1.91-nextest-0.9.140-prebuilt-perf-v2-lld-ec2-m5-perf16-soaklong16-interop16`,
  so every environment-sensitive gate runs fresh on the first AWS
  qualification.
- The long-soak worker class, which runs the canonical 2,000-CPS gate, is
  sized at 16 vCPUs (`m5.4xlarge`) and must stay at 16 or more. On 8 vCPUs
  the sweep sits at its CPU knee: about 3 ms of CPU per call needs roughly
  5.65 cores at 2,000 CPS, and p99 setup latency swung 17 to 39 ms run to
  run on identical code. Transparent huge pages were ruled out with zero
  compaction stalls.
- Each long gate now owns a long-soak worker: three instead of two. The two
  hour-long soaks previously ran back to back on one worker for about
  2 h 20 min, which left the three-hour controller job about ten minutes of
  slack. Qualification now finishes about an hour sooner, at a peak of 132
  vCPUs.
- The Jambonz OSS interoperability pins move to the 0.9.11 release line:
  `sbc-inbound` `d0d8ba93b2f3f9d09e3877be6a434744305cac4d` and `sbc-outbound`
  `7099671e69342dac60e2ab3001c56b18820ee302`, with source tarball digests
  re-verified. The release check requires the pinned components to be the
  current upstream heads, and upstream had moved on from 0.9.9.
- The short-performance worker class moves to 16 vCPUs (`m5.4xlarge`). Its
  call-setup sweeps drive 2,000 CPS, the same load that put the long soak on
  its CPU knee: on 8 vCPUs the PBX media-server profile's 2,000-CPS point
  passed at 15.8 ms p99 in one run and fell to a 47.5% answer rate in the
  next, on identical code. Its gates use absolute thresholds, so no baseline
  was re-recorded. Peak fleet demand is 192 vCPUs.
- The shutdown checkpoint unit is ordered after `network-online.target`, as
  its script requires. Without the ordering, the network could stop while a
  cut-off worker was still uploading, and interop and the long soaks left no
  `PARTIAL` result when the early-failure cutoff stopped them.
- The Jambonz lab's MySQL fixture makes its seed files world-readable. EC2
  workers check out the candidate under `umask 077`, `COPY` kept the rvoip
  seed at 0600, and the MySQL entrypoint, which reads seeds as the `mysql`
  user, exited before the lab could start. When `compose up` fails, the lab
  now prints every container's logs so the receipt names the cause.
- The interoperability worker moves to 16 vCPUs (`m5.4xlarge`) and, with the
  proxy-interop workers, launches alongside the performance prebuild instead
  of after it. Interop reads no performance bundle; it is the fleet's longest
  serial chain, and three of its gates compile on the worker. On 4 vCPUs the
  SIPp listener's cold full-LTO release build overran its timeout and the
  libSRTP build took 28 minutes. Cold release builds on EC2 now get a
  45-minute timeout, and four interop estimates carry measured durations. Historical GCP qualification evidence under
  `crates/sip/rvoip-sip/docs/` is unchanged. See
  `docs/AWS_RELEASE_WORKERS.md`.

### Cloudflare Tunnel (Parley demo)

- `deploy/cloudflare/config.yml` and `scripts/run-cloudflare-tunnel.sh` front a
  localhost Parley process through `parley.rudeless.ai` (HTTP `/v1`, widget,
  desk, webhooks) and `parley-uctp.rudeless.ai` (UCTP WebSocket). SIP/UDP is
  not in the ingress. Requires a named tunnel or `CLOUDFLARE_TUNNEL_TOKEN`.

### UCTP conversation dispatch

- The UCTP coordinator dispatches `conversation.create`, `conversation.list`,
  and `conversation.close` to the substrate adapter (oneshot + Orchestrator)
  instead of dropping them. WebSocket, QUIC, and WebTransport adapters fulfill
  those envelopes when configured with an Orchestrator. `conversation.close`
  is a new C→S type; `conversation.opened` / `conversation.closed` remain the
  server replies. `session.invite` with an Open `cid` can attach to that
  Conversation via `start_session`.

### Vapi AI Participant (breaking)

- `VapiAdapter::attach_agent` no longer copies the caller's `participant_id`
  onto the Vapi Connection. It joins a distinct `Ai`/`Agent` Participant and
  originates under that id. `attach_agent_for_participant` accepts an existing
  AI Participant (rejected if that id is already `Human`).
- `VapiAgentCall::ai_participant_id()` returns the AI Participant.

### Orchestrator Participant role verbs

- `Orchestrator::set_participant_role`, `take_over`, and `hand_off` change
  voip-3 roles without moving Connections. `take_over` / `hand_off` leave at
  most one `Agent` in the Session; extra agents become `Observer`.
- `Event::ParticipantRoleChanged` (and `rvoip_core.participant_role_changed`
  on the cross-crate bus) fires when the role actually changes.

### SIP ingress budget and admission observer

- `SipListenerAuthPolicy::with_source_rate_limit` drops requests from any
  source address over its token budget before any other admission check,
  trusted trunks included, and answers nothing: a flood is not told it is
  heard. `SipRequestAuthorization::Dropped` carries that outcome through the
  transaction layer without a response.
- `SipListenerAuthPolicy::with_ingress_observer` reports every admission
  decision (`SipIngressEvent`: source, method, admitted/rejected/dropped) to
  a caller-supplied `SipIngressObserver`, so an edge can count what it
  refuses instead of reading it back out of logs.
- `rvoip::app::SipConfig::source_rate_limit` and `ingress_observer` expose
  both through the facade; they take effect with the trusted-trunk policy.

### SIP NAT traversal on the live RTP socket

- `TransportStunClient` in `rvoip-rtp-core` sends Binding requests through a
  running `RtpTransport` and reads the response from the transport event
  stream. `rvoip-sip` now discovers the public mapping after allocating each
  call's RTP transport and waits for that exact mapping before rendering offer
  or answer SDP, so the advertised port is the port media actually uses. A
  static `media_public_addr` remains authoritative; resolution, timeout, and
  response failures still fall back to the local media address. With STUN
  configured, initial SDP generation can wait up to 1.5 seconds.

### SIP DTLS-SRTP interoperability

- `UDP/TLS/RTP/SAVPF` is accepted as a DTLS-SRTP audio profile alongside
  `SAVP`, so media servers that answer with the WebRTC feedback profile no
  longer fail before the DTLS handshake.
- `Config` gains `DtlsSetupRole` (`actpass` by default, `active` or `passive`
  for endpoint and NAT interop), and `srtp_offered_suites` now drives the DTLS
  `use_srtp` profile list in the configured order. AES-256 SDES suites are
  rejected when DTLS-SRTP is selected because the DTLS stack supports only the
  AES-128 SHA1-80 and SHA1-32 profiles.

### WebTransport origin allowlist

- `UctpWtConfig` accepts an opt-in exact `Origin` allowlist. Missing,
  duplicate, and unlisted browser origins are rejected before the WebTransport
  CONNECT is accepted; non-browser behaviour is unchanged when no policy is
  configured.

### WebSocket media bridge browser candidates

- The WebSocket media bridge answerer binds `0.0.0.0` and passes mDNS `.local`
  candidates through instead of using the loopback WebRTC profile, so Chrome's
  anonymised IPv4 host candidates pair and ICE completes.

### First-frame media latency restored on the receive side

- The bounded-audio-delivery work added a paced decoded-playout queue with
  two delays in front of the first decoded frame of every stream: a 45 ms
  startup reorder hold applied unconditionally, and delivery to the
  application only on a 5 ms playout tick. Together they added up to
  50 ms before an application saw the first frame, where it previously saw
  it within the packet's own arrival. An application that answers a call
  the moment the first early-media frame is delivered, as vapi-central's
  node does, lost that race to its own 200 OK every time.
- A decoded frame that is already due now leaves at packet arrival instead
  of waiting for the tick; later frames still pace on the tick.
- The startup reorder hold now applies only once a sequence gap is
  observed among the pending packets. An unbroken first run carries no
  evidence of reordering and is released at once; from the first observed
  gap onward the 45 ms hold protects ordering exactly as before. The one
  behaviour given up: if the first two packets of a stream arrive
  backwards, the earlier one is now treated as late rather than reordered.
  Tests: `startup_releases_a_contiguous_first_packet_without_holding_it`,
  `startup_holds_only_once_a_gap_is_observed`.

### AMR-WB coverage ported from Thelve

- Thelve's vendored fixes for AMR-WB, an `AmrWb` arm on the SIP media
  stream and bridge admission by negotiated payload type rather than the
  codec-name table, were already present here through the SIP-core codec
  wiring. Their acceptance tests are now carried too: the cross-connection
  bridge admits an AMR-WB leg that reports its negotiated payload type and
  still refuses one that does not, and the SIP stream's descriptor and
  codec follow the SDP clock and fmtp in both RFC 4867 framings while a
  non-16 kHz-mono shape is refused before a codec is built.
- vapi-ref-harness carried a state-table override adding the UAS
  `Answering + DialogCANCEL -> Cancelled` transition for a matched CANCEL
  that wins final-response authorship while accept is entering `Answering`.
  The shipped table already contains that transition byte for byte; a
  state-table test now pins it alongside the `Ringing` and `EarlyMedia`
  cases so it cannot regress silently.

### Receive-side playout overflow no longer silences a dialog

- The paced decoded-playout queue added with bounded audio delivery holds
  at most 64 frames. When a burst overflowed it, the RTP event handler for
  that dialog terminated, so no audio was ever decoded again for the rest
  of the call. A bridge releasing media it buffered until commit produces
  exactly such a burst, which is why a caller receiving forwarded early
  media saw the final answer before any decoded audio. Overflow now drops
  the newest frame and keeps the receive loop alive, the same way a full
  application callback channel is handled; the first drop is logged and
  the rest are counted. Regression test:
  `decoded_playout_overflow_drops_frames_but_keeps_the_receive_loop_alive`.

### Two-phase bridge peer handoff

- `Orchestrator::prepare_transport_fenced_peer_handoff` and
  `prepare_peer_handoff` stage a replacement bridge and return a
  `PreparedPeerHandoff` that owns the staged routes and destination
  reservation. `commit_peer_handoff_with_timeout_and_receipt`,
  `commit_peer_handoff_with_timeout`, `commit_peer_handoff_with_receipt`,
  and `commit_peer_handoff` perform the bounded quiescence and ownership
  commit and return a `PeerHandoffReceipt` (previous and replacement
  bridge ids, retained, source, target, `committed_at`). Dropping or
  awaiting `abandon()` on a prepared handoff rolls back without touching
  the original bridge. The strict variant refuses a bridge without a
  transport delivery fence with `NotImplemented`. Applications that must
  do durable work and re-check authority between preparation and commit
  can now do so; `replace_bridge_destination` and its transport-fenced
  wrapper are unchanged and are implemented as prepare followed by
  commit, so there is one commit path.
- `Event::PeerHandoffCommitted` is now emitted. It was defined in 0.3.10
  but never published. It fires exactly once from the shared commit
  path, after the ownership switch and before the compatibility
  `ConnectionsUnbridged` and `ConnectionsBridged` events, and never on
  a failed or abandoned handoff. `VapiAgentCall::wait_shared` now
  resolves to `HandedOff` when the AI leg is retired by a handoff.
- Ported the downstream acceptance tests for the handoff, the Vapi
  existing-call attachment (credential isolation on a shared
  credential-free adapter, no provider-call creation, no fallback to
  creation on socket failure), and the QUIC transport fence (stale
  generation rejected at dequeue; three-client A–B to A–C cutover where B
  receives no post-handoff frame).

### Bridge cutover fence and glare classification

- The media-graph forwarding gate's cutover mutex was only ever taken by
  `set_enabled`; the sink worker checked the flag and then called `send()`,
  which reserves and commits in one step, so a frame parked waiting for
  target capacity could be published after quiesce. The unticketed legacy
  sink now reserves first, races that reservation against the quiesce
  signal, and re-reads the flag under the cutover mutex before committing.
  A worker either publishes strictly before the cutover or abandons its
  frame. Routes that buffer while disabled keep their send; ticketed peer
  routes already fenced on generation and are unchanged.
- A bridge-destination replacement that loses a race to a concurrent
  contender now reports `BridgeNotFound` at every detection point (retired
  peer ticket, quiescence observing a changed peer, or a commit whose
  generation moved), matching the registry lookup and the reservation
  step. Callers get one classifiable retry signal instead of three. A
  quiescence timeout and a data route that ended mid-replacement remain
  `InvalidState`; a bridge that was never transport fenced remains
  `NotImplemented`.
- `SessionState::inbound_audio_codecs` moved from the hot struct to the
  copy-on-write cold block. It is read once per offer/answer, and keeping
  it hot had regressed the SIP hot-layout tripwire.

### Added

- Added Jambonz OSS 0.9.9 as a mandatory external SIP interoperability peer,
  using the shared Endpoint, StreamPeer, and CallbackPeer PBX runner against
  the real SBC/B2BUA, registrar, Drachtio, Redis, MySQL, and RTPengine topology.
- Added `CallbackPeerBuilder::on_refer_accepted` for successful local
  acceptance of an inbound REFER, distinct from the existing callback for a
  remote peer accepting an application-originated transfer.

### Fixed

- Serialized RFC 3515 implicit-subscription NOTIFY requests for an exact REFER
  lifecycle so `100 Trying`, progress, and terminal status cannot race or
  overtake one another.
- Preserved an optional RFC 3892 `Referred-By` header unchanged into the
  referenced INVITE while retaining `Refer-To` as the only required transfer
  target header.
- Hardened sensitive diagnostics, certificate and path handling, CI cache use,
  release feature-bundle checks, and CodeQL publication policy.
- Pinned the canonical performance client's app-session dispatcher shape so
  exact-candidate results cannot change with the release worker's detected CPU
  count.
- Reconciled current performance results from authoritative per-worker output
  instead of nested provenance copies, staged the required high-density burst
  evidence into the generated evaluation, and made the one-hour 500-call split
  soak an explicit machine-validated release result. A missing authoritative
  result tree now fails through the structured gate path rather than an
  uncaught filesystem exception.
- Preserved intentional prior-release and qualification references when
  preparing candidate-aware documentation, while still advancing live crate
  dependency examples and the current workspace runtime marker.

### Interoperability and release evidence

- The Jambonz UDP/plain-RTP profile covers authenticated registration,
  PCMU/PCMA bidirectional calls, provisional/final signaling, hold/resume,
  RFC 4733 DTMF, CANCEL/487, rejection, blind transfer with ordered NOTIFY,
  replacement INVITE, either-side BYE, and cleanup across all three public SIP
  APIs.
- Jambonz-specific exclusions are G.729, AMR, TLS/SRTP, the
  RVoIP-as-B2BUA scenario, WebRTC, PSTN, application verbs, recording, HA, and
  load. These exclusions do not reduce the separately qualified RVoIP codec or
  transport features.
- Publication now requires synchronized release notes, changelog,
  interoperability, compatibility, and RFC documentation with the protected
  exact-candidate qualification receipt.
- Publication requires a fresh exact-candidate performance evaluation with
  three clean canonical 2,000-CPS passes, the complete performance/resiliency
  matrix, high-density media and one-hour soak evidence, regression comparison,
  and a machine-readable artifact index; July measurements are baseline history
  rather than 0.3.10 qualification evidence.

## 0.3.9 — 2026-09-05

This coordinated 45-crate release makes the carrier media and remote-endpoint
paths reachable through the public facade, adds deployment-oriented feature
bundles, and hardens security, codec negotiation, browser interoperability,
and protected release evidence. It incorporates the previously unpublished
`0.3.8-thelve.1` candidate described below into the stable release.

### Production remote SIP endpoint profile

- The built-in registrar can now require authenticated RFC 5626 outbound
  registrations on exact TLS/WSS flows. It processes `ob`, `+sip.instance`,
  and `reg-id`, rejects incomplete remote endpoints with `439`, and retains
  opaque process-local flow capabilities rather than dialing private Contact
  addresses.
- Registered-AOR origination carries an ordered set of verified exact routes
  through the facade, SIP adapter, dialog manager, authentication retries, and
  transport failover. A failed primary stream can advance to a secondary flow
  without losing flow identity.
- Exact connection close, expiry, unregister, replacement, and restart make a
  route unavailable. Replacement uses prepare/response/commit ordering, so a
  zero-wire response or a staged-flow close cannot discard or falsely promote
  the previous live route.
- `SipConfig::remote_endpoint_profile()` fails startup unless TLS, mandatory
  SRTP, registrar identity, and a reachable media address are configured. The
  process-local AOR-affinity and real-NAT qualification boundaries are
  documented in `docs/sip/REMOTE_ENDPOINT_PROFILE.md`.

### DTLS-SRTP on the SIP media path

- `rvoip-sip` can negotiate `UDP/TLS/RTP/SAVP` with SHA-256 certificate
  fingerprints and RFC 8842 setup roles when its `dtls-srtp` feature is
  enabled. `Config::srtp_keying = SrtpKeyingMode::DtlsSrtp` selects the new
  path; SDES remains the compatible default.
- DTLS 1.2 shares the call's RTP socket through RFC 7983 demultiplexing. The
  media transport is latched secure-only before the asynchronous handshake,
  the certificate fingerprint from DTLS must match authenticated SDP before
  contexts are installed, and stale call generations cannot receive keys.
- Release gates now prove a real two-endpoint SIP call, independent
  `webrtc-srtp` RTP/SRTCP interoperability, shared-socket handshake behavior,
  strict DTLS-enabled Clippy, and facade feature forwarding. The carrier and
  full facade bundles include DTLS-SRTP; the minimal SIP endpoint does not.
- This is a supported dedicated handshake path. The older compatibility
  constructors under `rvoip-rtp-core::api::{client,server,common}` remain
  fail-closed and are not silently redirected to it.
- RTP session, client, and server teardown now send BYE behind a Receiver
  Report as RFC 3550 compound RTCP. RVoIP no longer emits unnegotiated
  reduced-size BYE packets that its peer correctly rejects during teardown.

### Deployment-oriented facade feature bundles

- The `rvoip` facade now offers six additive `bundle-*` starting points for a
  SIP endpoint, carrier SIP, browser gateway, AI conversation gateway, the
  complete pure-Rust facade, and the complete facade with native codecs.
  Existing leaf features and the default `sip` selection remain unchanged.
- The bundle catalog is declared with the facade manifest, rendered into
  `docs/FEATURE_BUNDLES.md`, checked for documentation drift, tested once per
  bundle with default features disabled, and inspected at the resolved
  dependency-graph level.
- Codec features now propagate through both the SIP adapter and the shared
  media graph. G.711 is baseline; G.729 and both AMR variants are in the
  carrier and pure-Rust full bundles; native Opus is explicit in the browser,
  AI, and native-full bundles. A pure-Rust build can no longer acquire Opus
  accidentally through `rvoip-core` feature unification.
- Direct `rvoip-core` consumers that previously relied on its default features
  for Opus transcoding must enable the crate's `opus` feature (or
  `all-codecs`) explicitly. The `full` feature continues to select every
  mainline codec, including Opus.

### Codec feature-matrix hardening

- AMR-NB and AMR-WB capability registration now compiles only when the
  corresponding codec is enabled, keeping codec-free builds warning-clean
  without weakening AMR support.
- Capability inventory tests now cover AMR-only builds explicitly, and the
  release gate exercises codec-free, AMR, and `all-codecs` configurations so
  AMR, Opus, PCMU, and PCMA remain first-class negotiated codecs.

### Committed per-session SIP codec renegotiation

- `SipAdapter::renegotiate_media` now renders a one-shot codec offer for the
  exact call generation, waits for the peer's final re-INVITE result, and
  returns the codec and payload type actually committed from the SDP answer.
  It no longer changes the media graph optimistically after request dispatch.
- Rejected, timed-out, replaced, or unobservable re-INVITEs fail closed and
  retain the stable negotiated media generation. A rejected transaction can
  be retried without changing adapter-global codec policy or another call's
  offer.
- A live `SipMediaStream` keeps its identity and channels while a committed
  codec generation rebuilds both media pumps. Managed cross-transport graphs
  now receive complete codec descriptors, preserving dynamic payload types and
  fmtp during UCTP/SIP and WebRTC/SIP hot swaps.

### Awaitable media readiness

- `ConnectionAdapter::wait_for_stream` now provides a transport-neutral,
  cancellation- and deadline-aware alternative to application polling loops.
  Existing adapters inherit a bounded compatibility implementation and may
  override it with a native registration watch without changing callers.
- `Orchestrator::wait_for_stream` captures and revalidates the exact connection
  lifecycle generation. Missing connections, terminal teardown, replacement,
  adapter loss, cancellation, deadline expiry, and adapter query failure are
  distinct outcomes.
- `StreamSelector` filters by media kind, optional codec and direction, plus
  explicit registered, source-ready, or bidirectional readiness. SIP, WebRTC,
  QUIC, and WebTransport production-path tests now consume this public surface.

### Lossless RTP observation on UCTP transports

- `observe_rtp_datagram` and `ObservedRtpDatagram` expose validated RTP
  sequence, timestamp, SSRC, marker, CSRCs, parsed extensions, padding size,
  and padding-free codec payload without taking ownership of adapter internals.
- `UctpQuicAdapter::new_with_rtp_ingress_observer` and
  `UctpWtAdapter::new_with_rtp_ingress_observer` publish those packets with
  their authenticated core route before conversion to `MediaFrame`. Delivery
  is bounded and best-effort, so an unavailable observer never stalls audio.
- The existing constructors and payload-only media consumers are unchanged.
  Normal outbound `MediaFrame` forwarding deliberately starts a new RTP hop:
  marker is false, CSRC/extensions are empty, and padding is omitted. Exact
  reserialization is available only through `pack_observed_rtp_datagram`.

### ICE (RFC 8445) on the SIP path

- New crate **`rvoip-ice-core`**: a sans-io ICE agent and RFC 8489 STUN
  codec. The agent is handed packets and the clock and polled for
  transmissions, events, and deadlines — no sockets, no runtime — so role
  conflicts, nomination races, loss, restarts, and wrong-password handling
  are scripted deterministic tests (19 of them, over a virtual wire with a
  port-restricted NAT). The codec is verified against the RFC 5769 vectors,
  which are self-validating: FINGERPRINT covers every byte before it and the
  HMAC everything before that.
- `SipConfig::ice(SipIcePolicy::{Disabled, Lite, Full})` on the app builder,
  `Config::ice` on the coordinator. **Disabled is the default and is
  today's behavior byte for byte** — SDP without ICE attributes negotiates
  exactly as before, and a peer that declines ICE retires the runtime and
  proceeds on the SDP path.
- Offers and answers carry `a=ice-ufrag`/`a=ice-pwd`/`a=candidate` (and
  `a=ice-lite` for lite) when enabled; the peer's material is extracted
  from parsed SDP; RFC 8839 `ice-mismatch` (a middlebox rewrote c=/m=
  after the peer built its SDP) is detected and stands ICE down for that
  call rather than fighting the box that owns the path.
- One pump task per media session shuttles STUN between the agent and the
  RTP socket: the transport now forwards demuxed STUN datagrams as
  `RtpEvent::StunPacket` (both plain and SRTP receive paths — ICE sits
  below SRTP, so checks on a secured socket are forwarded, not rejected),
  and `RtpTransport::send_stun_bytes` is the one legitimate plaintext send
  on a secured transport, gated on the payload actually classifying as
  STUN. Nomination retargets media through the same `establish_media_flow`
  the SDP path uses.
- A full peer beside a lite one is controlling regardless of who offered
  (RFC 8445 §6.1.1); lite refuses to build without a reachable address.
- Scope honesty: single component (requires rtcp-mux semantics; component
  ids stay in the model so two-component is an extension), no TURN yet, no
  trickle over SIP, and the post-nomination re-INVITE (RFC 8839 §4.4) is a
  recorded follow-up — the media path itself is already correct from the
  retarget.

### Media quality reaches the application

- **`OperationalEventKind::Quality`** puts per-connection quality on the
  authoritative stream. An application that took the operational receiver has
  stopped reading the observational broadcast, so quality it never sees is
  quality it cannot act on — and a call degrading is worth reacting to while
  it is still up. Scaled to integers (hundredths) because the enum is `Eq`
  and floats are not; a negative or non-finite reading clamps to zero rather
  than wrapping into an enormous unsigned value.
- **MOS survives the SIP boundary.** `Event::MediaQualityChanged` gained a
  `mos` field. media-core computed the estimate all along; the adapter was
  discarding it with a comment saying it would keep doing so "until the
  ApiEvent grows a `mos` field".
- **`MediaStream::has_quality_measurement`** (default `true`) lets a
  transport say it has no measurement. `QualitySnapshot::default()` is all
  zeros, which reads as *flawless* rather than *unknown*, and the type had no
  way to distinguish them. `spawn_media_quality_sampler` now skips
  unmeasured connections instead of averaging a perfect score into the
  report.
- **`SipMediaStream` retains its last quality report**, so `quality_snapshot`
  returns the RTCP-derived measurement the adapter routed to it rather than a
  default. Before this it always returned zeros, which made the sampler
  unusable on the SIP path: polling it published flawless quality for every
  call forever.
- `RvoipAppBuilder::media_quality_interval` starts the sampler. Off by
  default.

### STUN-discovered advertised address

- `SipConfig::discover_advertised_addr(stun_server)` learns the reachable
  address at startup instead of requiring it configured, for a listener
  behind NAT whose public address is not known ahead of time.
- **Not ICE, deliberately.** RFC 8445 negotiates candidate pairs with
  connectivity checks, and a carrier SIP trunk does not offer it — the far
  end expects one reachable media address. What ICE's server-reflexive step
  buys on this path is knowing that address, which is a STUN binding request.
  Browser legs, where ICE genuinely applies, are served by the WebRTC
  transport.
- Fails closed: a STUN server that cannot answer fails the build rather than
  starting a listener that advertises a guess. A call that connects and
  carries no audio is harder to diagnose than a service that refused to
  start. A static `advertised_addr` wins over discovery.

### Carrier-grade media on the SIP path

- **`PlayoutBuffer`** (`media-core::processing::audio::playout`) smooths and
  conceals a decoded audio stream: frames are reordered onto the media clock,
  a short backlog absorbs burst arrival, and a frame that never arrives is
  synthesized rather than left as a gap. Concealment is repeat-with-fade —
  the cheap technique, named as such — which removes the click that dominates
  the artifact budget; a long burst fades to silence rather than repeating
  the same 20 ms indefinitely. RTP timestamp wrap is handled, without which a
  single wrap discards every later frame.
- Playout is driven by a local media-clock deadline, not by packet arrival.
  Excess depth opens a bounded drain valve to reconverge latency, while a
  long-baseline RTP/arrival comparison tracks remote oscillator skew. The
  release matrix holds depth bounded for one simulated hour at both +50 ppm
  and -50 ppm and proves G.711 PLC fires on the missing frame's deadline.
- `Config::playout` on the SIP config, `SipConfig::playout` on the app
  builder. Generic/local configs retain the compatibility default; the
  carrier/SBC profile and app listeners admitting a trusted trunk enable the
  carrier-safe default automatically. Controlled LAN labs may opt out.
- Note for anyone surveying this area: `media-core`'s
  `rtp_processing::jitter::JitterBuffer` is a stub — `get_packet` is
  `pop_first` and `flush_old_packets` clears the whole buffer. It has no
  callers. The real packet-level buffer is `rtp-core`'s
  `AdaptiveJitterBuffer`.

### SRTP reachable from the app builder

- `SipConfig::media_security(SipMediaSecurity::{Disabled, Preferred,
  Required})`. `rvoip-sip::Config` already carried `offer_srtp` and
  `srtp_required`, but no builder surfaced them, so an application had no way
  to ask for encrypted media. `Required` refuses plaintext fallback;
  `Preferred` carries the call in the clear when the peer declines, which is
  the case an operator most needs to know about.

### Trusted private identity and carrier signaling

- A trusted trunk's `P-Asserted-Identity` now reaches the inbound context as
  the distinct, redacted `InboundAssertedIdentity` field, with
  `SipTrustedTrunk` provenance. It is intentionally not generic `X-*`
  metadata, so an application cannot accidentally treat an untrusted value as
  carrier-authenticated caller identity.
- Surfaced **only** when the peer was admitted by trusted-trunk policy. RFC
  3325 makes PAI meaningful only inside a trust domain; from an unverified
  peer it is a forgeable header that looks authoritative, which is worse than
  its absence.
- Trusted trunks can opt in to a bounded private-header allowlist. The first
  supported carrier field is `P-Charging-Vector`; the default remains empty,
  unlisted fields are stripped, and PAI/PPI cannot enter through the raw
  header path.
- `OutboundCallBuilder::with_ppi` adds typed `P-Preferred-Identity` alongside
  typed PAI. Both identities are preflight-validated, redacted from
  diagnostics, emitted on the first INVITE, and retained byte-for-byte across
  401/407 authentication retries.

### N-way conferencing

- `Orchestrator::conference_create/join/leave/end/members` plus a
  `conference` module holding the mixer. Bridging is pairwise and does not
  generalize: a conference has to *sum* audio, and every participant needs a
  different sum with their own voice removed, or they hear themselves
  returned a packet late.
- One task per conference mixes on a 20 ms tick. The sum is computed once in
  `i32` and each member receives it minus their own contribution, so the
  work is linear in members rather than quadratic, and the result saturates
  rather than wraps — a wrapped sum is an audible click.
- Members keep their own negotiated codec in both directions and are
  resampled into and out of the conference rate, so a G.711 carrier leg and
  an Opus browser leg mix together without either renegotiating. A member's
  tap is owned by the member, so leaving tears the route down.
- The internal mix bus is canonical mono. Stereo members are downmixed before
  resampling and expanded to their negotiated interleaved layout before
  encoding; RTP timestamps advance by sample frames rather than scalar
  samples. A real stereo-Opus regression protects the ordinary browser leg.
- A member whose transport has closed is removed from the mix rather than
  retried; one member's undecodable packet is skipped rather than silencing
  the conference.
- `conference_set_contribution` silences a member's voice while leaving them
  hearing the mix — a supervisor monitoring a call. Silencing at the mixer
  rather than at the member's transport keeps the rest of the conference
  unable to tell anyone is listening, which is what monitoring means.

### AMR in the codec factory

- `CodecFactory::create_negotiated_codec(payload_type, encoding_name,
  sample_rate, channels, fmtp)` constructs a codec from its negotiated SDP
  identity. `create_codec` keys off the payload type alone, which is enough
  only for statically assigned codecs; AMR is dynamically assigned and its
  mode set arrives in `fmtp`, neither of which a payload type can express.
  Non-AMR names delegate to `create_codec`, so existing callers are
  unaffected.

### Per-recording sink factories

- `RecordingSinkFactory` opens one `RecordingSink` per recording, and
  `Orchestrator::register_recording_sink_factory` registers one under the
  same namespace as a plain sink, taking precedence over it.
- Why: a registered `RecordingSink` is a single shared instance. Two
  concurrent recordings on one name wrote into the same sink, and the first
  `stop_recording` closed it — so their audio mixed and the artifact was
  attributed to whichever stopped first. That is invisible with one call in
  flight, which is the shape the deterministic harness exercises, and wrong
  for any deployment recording more than one call at a time.
- `start_recording` resolves a factory first and falls back to a registered
  sink, so existing single-sink registrations behave exactly as before. An
  unregistered name still fails closed before any tap or quota work.

### Authoritative application ingress (RVOIP-22)

- `RvoipAppBuilder::authoritative_ingress(AuthoritativeIngressConfig)`
  installs the inbound admission gate and the single-consumer operational
  event stream **before** any adapter is registered — the ordering core
  requires and the convenience `build` could not express, because it
  constructed its Orchestrator, registered adapters, and only then returned.
- `RvoipApp::take_authoritative_ingress` hands both receivers to the owning
  application exactly once; `ingress_health` reports mode, core's stream
  health, and whether the runtime still admits new work; `drain(budget)` is
  a bounded terminal join point that reports honestly whether it finished.
- In authoritative mode the app no longer admits inbound connections on the
  application's behalf: every inbound connection is presented as an
  `InboundAdmission` ticket and the normalized event follows acceptance.
- A lagged observational receiver is recorded as degraded ingress instead of
  a warning that keeps serving — `admits_new_work` goes false so a readiness
  probe can fail. Losing the operational receiver degrades the runtime and
  stops admission, which core already enforced and the app now surfaces.

### Vapi barge-in reaches the media graph

- User-speech-start now flushes adapter-local audio and the downstream
  orchestrator media-graph sink queues in the same barge-in operation. The
  discarded graph frames contribute to `VapiMediaHealth::barge_in_dropped`
  and `rvoip_vapi_barge_in_frames_dropped_total`.
- The mock-transport acceptance test backpressures a real bridged caller sink,
  proves audio is parked in the graph, then verifies the speech event drives
  queue depth to zero and accounts for every graph frame dropped.

### Checked RTP/media boundary

- `rvoip_core::rtp_boundary` converts validated RTP packets to payload-only
  `MediaFrame`s with explicit negotiated codec/PT mappings and bounded payload
  allocation. A packet-preserving handle retains marker, CSRCs, extensions,
  padding, SSRC, and sequence identity when no transformation occurred.
- `RtpPacketizer` owns deterministic SSRC, wrapping sequence, and wrapping
  timestamp state. Mismatched frame kind or payload type fails before state
  advances; `Bytes` payloads remain immutable and shared for fanout.
- The provider-neutral `checked_rtp_boundary` example and concrete SIP,
  WebRTC, and UCTP gateway examples all use the same shared API. A dedicated
  fuzz target covers malformed packet-to-frame conversion under fixed input
  and payload bounds.

### Signature freshness

- `Sig9421Verifier` bounds envelope timestamps from above as well as below.
  A far-future `ts` produced a negative age, passed the `age > ttl` test,
  and stayed valid for as long as the sender chose. `DEFAULT_SIG_CLOCK_SKEW`
  (30 s) is the tolerated drift; `with_ttl_and_skew` makes it explicit.


## 0.3.8 — 2026-08-14

This coordinated 44-crate patch release brings AMR-NB and AMR-WB into the
codec set end to end — negotiation, transport, transcoding, and release
evidence — adds record-routed proxy interop to the qualification matrix, and
repairs SIPS dialog and opus-bridge edge cases found on the way.

### AMR codecs

- Add AMR-NB and AMR-WB with IF1 and IF2 interface formats, VAD1/VAD2 and DTX
  reaching the wire, receive-side interleaving reassembly, max-red redundancy
  scheduling with dedup, and CMR damping — bit-exact against the fetched
  TS 26.073/26.101/26.201 material, with no 3GPP sources in the repository.
- Negotiate and obey the SDP mode-set, prove every mode in a live call, and
  attest each rate in the release evidence; long-run soaks cover both
  variants.
- Admit dynamic codecs into the media graph by their negotiated payload type
  (`CodecInfo` now carries it), re-frame packet times AMR cannot accept
  (10 ms joins and 30 ms splits), and label emitted frames so the UCTP pumps
  stop stamping Opus's number on everything else.
- Prove AMR crosses SRTP in process and a QUIC datagram in both directions.

### SIP, proxies, and interop

- Generate a secure fallback Contact for every RFC 3261 §12.1.1 trigger, so
  secure dialogs answer with `sips:` at the TLS-advertised address while
  explicit Contact and plain-SIP behavior are preserved (issue #176). This
  also repairs rvoip-to-rvoip SIPS setup: `Dialog::from_2xx_response` refuses
  a secure dialog whose Contact is not `sips:`, which the old plain fallback
  tripped.
- Learn the UAC route set from the dialog-forming 2xx's Record-Route
  (reversed per §12.1.2), so in-dialog requests stop bypassing
  record-routing proxies; the UAS side reads it from the request.
- Add Kamailio and OpenSIPS registrar-proxy labs with TLS and SRTP through
  rtpengine, opt-in AMR-NB transcoding (the AMR-WB transcode failure is
  attributed to rtpengine), and a per-rate sweep bound to the gate catalog.
- Expose the profiled egress registration's coordinator for
  observation-only event subscriptions; the composite adapter remains the
  sole signaling and lifecycle owner.
- `Config` gains `with_amr_dtx`, `with_amr_auto_cmr`, and
  `with_amr_mode_set` builders (private fields — `Config`'s constructible
  shape stays frozen). DTX and auto-CMR are local media policy; only the
  RFC 4867 `mode-set` is negotiated.
- `CodecInfo` carries the payload type a transport negotiated
  (`payload_type: Option<u8>`). Code constructing `CodecInfo` literals adds
  one field on upgrade; `None` preserves the name-table behavior.

### Media graph and bridges

- Keep opus↔opus bridges passthrough when the two legs numbered opus
  differently: the payload type is a per-leg SDP artifact, so the bypass
  compares name, rate, and channels, and passthrough restamps the sink's
  payload type on egress.
- Make a barge-in flush empty the re-framing accumulator as well as the sink
  queues, so no pre-interruption audio or dead-timeline timestamp survives
  into the first post-flush frame.
- Reach the opus and all-codecs feature sets from the rvoip facade.

### Qualification

`0.3.8` requires a fresh `remote-release` qualification bound to the updated
gate catalog, which adds the AMR per-rate sweep, the proxy-PBX media family,
and the AMR fuzz targets. Because the aggregate is bound to the catalog hash,
no `0.3.7` evidence qualifies this release.

## 0.3.7 — 2026-08-06

This coordinated 44-crate patch release hardens voice-AI and WebRTC media under
backpressure, exposes inbound SIP auth/context on the app facade, and repairs
SIP/WebRTC edge cases that dropped audio, DTMF, or late tracks.

### Vapi and media reliability

- Bound inbound/outbound audio queues so bursts and uplink stalls no longer kill
  the session; keep the RTP clock advancing across underruns and re-converge
  jitter depth on renegotiation.
- Adaptive jitter target, working inbound catch-up, and a symmetric outbound
  drain valve; flush stale playout audio on barge-in.
- Move WebSocket writes off the media loop, isolate control from media
  backpressure, and attribute media logs and health telemetry per call
  (`VapiMediaHealth`, current depth vs high-water, catch-up blocked ticks).

### WebRTC, Connect, and SIP

- Preserve media and unbind under driver backpressure; tolerate WebRTC startup
  backpressure without evicting Connect media routes or sinks.
- Allow primary audio and DTMF when a peer never negotiates MID; attach late
  remote audio tracks; bound per-peer UDP allocation; preserve remote codec
  preference order.
- Route wildcard contacts via the observed source address.
- Surface listener auth and inbound context policy on `SipConfig`
  (`tenant`, `trusted_trunk`, `capture_headers`) so facade apps can do
  DID-based routing and trunk admission.

### Release and workspace

- Publish the exact qualified candidate, keep qualification checkout on `main`,
  and allow signed ancestor release publication when attestation requires it.
- Inherit remaining third-party and `rtc` dependency pins from the workspace so
  version bumps stay single-source.

### Qualification

`0.3.7` requires a fresh, source-bound strict full-beta PASS. Historical
`0.3.2` exception, `0.3.4` carry-forward, and prior `0.3.6` qualification
evidence do not qualify it.

## 0.3.6 — 2026-08-02

This coordinated 44-crate patch release moves full release qualification onto
ephemeral GCP workers, repairs remote gate false failures, and lands SIP/core
correctness fixes needed for reliable attestation.

### Release qualification

- Run complete release qualification on ephemeral GCP workers with parallel
  performance, soak, proxy-interop, and diagnostic profiles.
- Cache exact performance build bundles, stream large artifacts from disk, and
  reuse selective evidence only when digests match.
- Reject failed candidates before deferred gates finish; accelerate long soaks;
  harden burst RSS and FreeSWITCH/PBX readiness checks.
- Automate active release metadata updates in `README.md`,
  `BETA_RELEASE_CHECKLIST.md`, and `RELEASE_NOTES_NEXT.md`.

### SIP, core, and security dependencies

- Publish the established event only after ACK; consume non-2xx ACK at the write
  boundary; tolerate legal final-response retransmission in soak evidence.
- Preserve cross-crate event semantics and make filtered message pagination
  deterministic.
- Send browser DTMF on the negotiated audio source.
- Upgrade jsonwebtoken, OpenTelemetry, and SIP terminal UI dependencies; remove
  the legacy AWS rustls adapter.

### Qualification

`0.3.6` requires a fresh, source-bound strict full-beta PASS. Historical
`0.3.2` exception and `0.3.4` carry-forward evidence do not qualify it.

## 0.3.5 — 2026-07-30

This coordinated 44-crate patch release hardens security and media state,
completes transactional SIP renegotiation, exposes symmetric-RTP NAT policy on
the high-level APIs, and makes Tokio the sole WebRTC runtime.

### Security and media

- Fail closed for placeholder AES-GCM profiles and unsupported DTLS
  construction; incomplete profiles cannot be advertised or negotiated.
- Correct RTP padding and RFC 8285 extensions, RTCP LSR/compound parsing, and
  loss/jitter accounting across rollover, gaps, duplicates, and reordering.
- Separate inbound/outbound and per-SSRC SRTP/SRTCP state, authenticate before
  committing replay state, and cover the result with RFC vectors and pinned
  libSRTP interoperability.
- Generate directional SDES answer keys and accept safely unpadded AES-256 key
  material in compatible mode with secret-safe diagnostics (issue #46).
- Preserve the 0.3.x `Config`, `Event`, `SessionError`, state-table, and
  negotiated-media shapes while exposing new auth, SDES, and renegotiation
  details through bounded additive diagnostic/runtime APIs.

### SIP, codecs, and WebRTC

- Make hold/resume, re-INVITE, UPDATE, delayed offers, authentication retries,
  retransmissions, rollback, and media application transactional and
  exact-generation owned.
- Expose `SipNatConfig` and `SymmetricRtpPolicy` through `EndpointBuilder` and
  `StreamPeerBuilder` (issue #50).
- Use the real Opus backend, make codec names ASCII case-insensitive, and stop
  advertising unavailable Opus or G.722 implementations.
- Remove Smol/async-std runtime support from WebRTC and correct the confirmed
  Chromium audio/SSRC/simulcast SDP regression.
- Preserve bounded, sharded, keyed/no-scan SIP lifecycle paths and make the
  registrar and coordinated workspace pass strict release linting.

### Qualification

`0.3.5` requires a fresh, source-bound strict full-beta PASS. Historical
`0.3.2` exception and `0.3.4` carry-forward evidence do not qualify it.

## 0.3.4 — 2026-07-29

This coordinated 44-crate patch release adds exact inbound-admission terminal
notification, completes RFC 6026 INVITE Accepted lifecycles, and introduces a
bounded RFC 3261 transaction-stateful proxy profile updated by RFC 4320 and
RFC 6026.

### Added

- Exact-generation inbound admission termination notification for cancelled,
  remotely ended, and failed source legs without polling.
- Compact, sharded Timer M/L retention driven by the existing manager-owned
  deadline queues rather than per-transaction runners and sleeper tasks.
- Matched/unmatched proxy CANCEL handling, fork response contexts, multiple
  and late 2xx forwarding, ACK ownership, Timer C, response aggregation, and
  strict/loose routing coverage within the documented bounded profile.
- Fail-closed Kamailio/OpenSIPS interoperability and four-peer beta-report
  attestation tooling for future strict full-beta runs.
- `rvoip-release-carry-forward-attestation-v1` release verification.

### Fixed

- Atomic, generation-protected transaction timer firing removes a saturated
  command-channel race that could strand expired transactions.
- Accepted transactions shed active runners, transports, command queues, and
  active-only locks while preserving RFC retention and exact cleanup.
- INVITE retransmission, late response, Via/route, RFC 3263 failover, and
  stateful-proxy response behavior have focused regression coverage.

### Qualification

The owner approved `0.3.4` with a transparent carry-forward disposition. The
full `0.3.4` beta, four-peer interoperability matrix, and long soaks were not
rerun. Current evidence is the complete workspace release verification and one
clean revision-bound canonical 2,000-CPS/65,000-call real-media PASS. The
immutable `0.3.2` owner-approved exception remains historical background with
strict status `NON-RC` and is not relabeled as a current beta PASS.

## 0.3.3 — 2026-07-29

This unified patch release corrects the vCon wire model, Session-finalization
path, signatures, content hashes, stores, and documentation against
`draft-ietf-vcon-vcon-core` commit
`2342aba64bdb71d9e80ab6e274a3921e2b1c769e`.

### Fixed

- End-of-Session emission now converts snapshots into the canonical
  `rvoip-vcon` model, validates them, serializes with serde, and suppresses
  persistence/`VconReady` on conversion, validation, or serialization failure.
  Inline dialog, analysis, and attachment bodies are preserved as Base64Url.
- The container now uses vCon `0.4.0`, durations in seconds, required analysis
  vendor/encoding data, attachment placement and purpose fields, URL/hash
  dependencies, complete dialog/party fields, and mutually exclusive
  redacted/amended lineage.
- vCon signatures now use JWS General JSON Serialization with appendable
  signatures and certificate references. Compact JWT serialization is no
  longer emitted as a signed vCon.
- New store handles use the specified
  `sha512-<unpadded-base64url-digest>` content hash in memory and PostgreSQL.
  The typed `VconStore` contract exposes the stored content hash, and identical
  canonical documents hash identically across memory and PostgreSQL. Existing
  persisted legacy hashes are not rewritten.
- Federation documentation no longer assigns semantics to reserved core
  `group`; sibling-vCon grouping is deferred to a future named extension
  declared in `extensions[]`.
- Documentation now states the shipped security boundary: core emission is
  unsigned, JWS signing is explicit, JWE is absent, and lineage types do not
  perform redaction.

### Added

- Canonical draft example/schema conformance coverage and a dedicated vCon CI
  job, including live PostgreSQL store tests and affected integration
  boundaries.
- Hash- and commit-bound `--targeted-delta-attestation` release verification.
  It retains unified manifest, workspace compile, and package checks while
  honestly recording that broad beta/workspace test/doc suites were not
  rerun. The approved targeted matrix is rerun and live PostgreSQL evidence is
  machine-verified.

### Breaking vCon changes

- `sign_jws` now accepts a certificate reference and returns `SignedVcon`;
  `append_signature` adds another signer, and verification accepts the General
  JSON form. HMAC algorithms are rejected for this certificate-bound API.
- Dialog `duration_ms` becomes `duration` in seconds. The model adds the
  standard party, dialog, analysis, attachment, extension, and critical
  fields. The undeclared Party `role` field is removed; the core `type` field
  classifies parties (for example, `person`, `bot`, or `organization`), while
  role semantics require a declared extension.
- `redacted: Vec<RedactionRecord>` becomes one optional `Redacted` object and
  gains the mutually exclusive optional `Amended` object.
- Core vCon analysis vendors and attachment placement become required;
  attachment `note` becomes `purpose`; party `did_or_stir` splits into `did`
  and `stir`; and snapshot encoding is fallible. The core byte-store `put`
  contract now also receives `ConversationId` and exposes
  `list_for_conversation` so sibling vCons are linked in index metadata rather
  than through the reserved `group` parameter.

## 0.3.2 — 2026-07-29

This unified release advances the reusable Bridgefu 1.0 foundation across all
44 publishable workspace crates.

### Added

- Hash-bound release-exception reporting for the owner-approved 0.3.2
  candidate, with the strict 106/108 gate result and NON-RC qualification
  preserved rather than rewritten.
- Complete authenticated-principal propagation and ownership checks across
  SIP, WebRTC, UCTP, routes, and operational events.
- Transport-neutral `DataMessage`, arbitrary WebRTC DataChannels, SIP MESSAGE,
  typed initial SIP headers, DTMF, and correlated transfer outcomes.
- Single-consumer `MediaGraph` with directional routes, codec-group
  transcoding, bounded fanout, snapshots, drops/evictions, and metrics.
- Dormant prepare/bind/activate lifecycles for SIP, WebRTC, and Amazon Connect,
  including owned cancellation, terminal events, and bounded drain.
- SIP outbound activation receipts now linearize after the exact session is
  active. Established teardown waits for the peer's successful final BYE
  response while still reclaiming local state on timeout or rejection.
- UCTP 0.2 complete-RTP routing, authenticated raw QUIC/WebTransport sessions,
  virtual publishers, direct-listener limits, and exact cleanup.
- `rvoip-moq` draft-19/MSF-01/LOC-03 publisher, subscriber, origin, relay,
  authorization, compatibility, reconnect, health, and drain abstractions.
- Configurable symmetric RTP, advertised SIP/RTP addresses, RFC 3581 `rport`,
  WebRTC ICE server/NAT policy, and per-exchange WHIP/WHEP versus WS gathering.
- Developer-preview `rvoip-vapi` bidirectional WebSocket agent adapter, exposed
  by the facade's opt-in `vapi` feature and included in `full`.
- High-level `rvoip::app` voice-only admission for SIP or WebRTC customers,
  including transport-neutral accepted-call events, startup-safe event
  retention, explicit SIP/RTP advertisement, and example 14's shared Vapi
  agent server.

### Fixed

- SCIM provisioning now generates policy-safe bootstrap passwords without
  random failures from repeated/sequential characters or username overlap.

### Breaking protocol changes

- UCTP media datagrams now carry a complete RTP packet after the UCTP header.
- Wire-incompatible MOQT draft changes are semver-breaking at the
  `rvoip-moq` compatibility boundary.
- `rvoip_core_traits::connection::Transport` adds the `Vapi` variant; downstream
  exhaustive matches over this public enum must add a corresponding arm.
- `rvoip::app::AppEvent` adds the `InboundCallAccepted` variant; downstream
  exhaustive matches over this public enum must add a corresponding arm.

The private WebRTC/RTC TURN candidate and the dynamic moq-rs publisher-lease
candidate remain outside the consumed dependency graph until project-owner
review. No upstream submission is authorized by this changelog.
