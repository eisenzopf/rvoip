//! Single-target stateful proxy primitives (RFC 3261 §16).
//!
//! [`StatefulProxy`] subscribes to a [`TransactionManager`] event stream
//! and pairs every inbound server transaction (the UAC-facing leg) with
//! one downstream client transaction (the UAS-facing leg). Requests are
//! forwarded after §16.6 mutations (Max-Forwards decrement, Via push
//! with fresh `z9hG4bK…` branch). Responses are forwarded back after
//! §16.7 mutations (top-Via pop). Timer C (§16.8) fires on stalled
//! INVITE legs and surfaces a 408 upstream.
//!
//! The proxy is dialog-agnostic — it never touches `DialogManager`.
//! Mixed-mode deployments (proxy for some traffic, UA for the rest) are
//! out of scope for Phase 6; a `StatefulProxy` and a `DialogManager`
//! both subscribing to the same `TransactionManager` would race on every
//! inbound and is therefore unsupported.

use std::cmp::Ordering as CmpOrdering;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use dashmap::{mapref::entry::Entry, DashMap};
use rvoip_sip_core::types::max_forwards::MaxForwards;
use rvoip_sip_core::types::status::StatusCode;
use rvoip_sip_core::types::uri::{Host, Scheme, Uri};
use rvoip_sip_core::types::via::Via;
use rvoip_sip_core::types::TypedHeader;
use rvoip_sip_core::{Method, Request, Response};
use rvoip_sip_dialog::transaction::{
    CancelInviteTransactionDispatch, StatefulProxyIngressEvent, TransactionEvent, TransactionKey,
    TransactionManager,
};
use rvoip_sip_dialog::FinalResponseCompletionDisposition;
use rvoip_sip_transport::resolver::{ResolvedTarget, Resolver};
use rvoip_sip_transport::transport::{TransportAuthority, TransportRoute, TransportType};
use sha2::{Digest, Sha256};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::{debug, info, trace, warn};

use crate::error::{ProxyBuildError, ProxyError};
use crate::routing::{
    self, DefaultProxyResolver, ProxyRoutingPolicy, ProxyTarget, RequestRejection,
};

/// Default Timer C duration — RFC 3261 §16.8 requires "greater than
/// 3 minutes". Applications may override it for deterministic tests.
pub const DEFAULT_TIMER_C: Duration = Duration::from_secs(181);
const RESPONSE_CONTEXT_RETENTION: Duration = Duration::from_secs(64);
const DEFAULT_RESPONSE_CONTEXT_CAPACITY: usize = 168_000;
const DEFAULT_DOWNSTREAM_TRANSACTION_CAPACITY: usize = 336_000;
const DEFAULT_BRANCHES_PER_RESPONSE_CONTEXT: usize = 64;
const DEFAULT_STATELESS_RESPONSE_ROUTE_CAPACITY: usize = 4_096;
const FORK_EXPIRY_BATCH: usize = 256;
const STATELESS_RESPONSE_EXPIRY_BATCH: usize = 256;
const STATELESS_RESPONSE_HEAP_COMPACTION_FACTOR: usize = 2;
const STATELESS_RESPONSE_HEAP_COMPACTION_SLACK: usize = 32;
const TIMER_C_EXPIRY_BATCH: usize = 256;
const TIMER_C_HEAP_COMPACTION_FACTOR: usize = 2;
const TIMER_C_HEAP_COMPACTION_SLACK: usize = 32;

/// Application-supplied routing function.
///
/// Called for every inbound request that needs forwarding. Returns a non-empty
/// `Some(RouteDecision)` to forward the request, `None` when the addressed
/// resource is unknown (404), or an empty decision when a known resource has
/// no currently available targets (480).
///
/// The closure runs on the proxy event-loop task, so it must not
/// block — defer slow lookups to a separate task and return the
/// resolved address asynchronously via a channel + cache if needed.
pub type RouteFn = Arc<dyn Fn(&Request) -> Option<RouteDecision> + Send + Sync + 'static>;

/// Application-supplied URI routing function.
///
/// This additive route surface keeps the exact three-field [`RouteDecision`]
/// shape published in 0.3.1 while allowing RFC 3263/SIPS-aware targets.
pub type UriRouteFn = Arc<dyn Fn(&Request) -> Option<UriRouteDecision> + Send + Sync + 'static>;

/// Observable events emitted by [`StatefulProxy`] for application
/// consumption. Subscribe via [`StatefulProxy::subscribe_events`] or
/// the corresponding `ProxyCoordinator` accessor.
///
/// The stream is **observability-only**: the proxy still acts on these
/// events (e.g. forwards a 3xx upstream) regardless of whether anyone
/// is listening. Future iterations may add an interception trait that
/// lets an application redirect the proxy's response — that is a
/// deferred follow-up.
#[derive(Debug, Clone)]
pub enum ProxyEvent {
    /// A downstream leg returned a 3xx response. `contacts` carries
    /// every URI from the response's `Contact:` header(s) in the
    /// order they appeared — the application can re-fork against
    /// these targets by issuing a fresh request out of band.
    ///
    /// The proxy continues forwarding the 3xx upstream after emission;
    /// the UAC is the canonical redirect handler.
    RedirectReceived {
        /// The upstream server transaction that triggered the leg
        /// which received the redirect.
        upstream_tx: TransactionKey,
        /// The 3xx status code that arrived (302, 301, 305, …).
        status: StatusCode,
        /// Targets the UAS would like the call routed to. Empty when
        /// the redirect carried no parseable Contact.
        contacts: Vec<Uri>,
    },
}

/// How to fork an inbound request across multiple downstream targets.
///
/// RFC 3261 §16.7 defines forking semantics; this enum picks the
/// concurrency policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkMode {
    /// Send the request to every target at once. The first 2xx wins;
    /// all other still-pending legs are CANCELed.
    Parallel,
    /// Try targets in order. On a failure final (3xx-6xx) advance to
    /// the next target. On 2xx, forward upstream and stop. On
    /// exhaustion, forward the best-collected failure upstream per
    /// §16.7 step 6.
    Sequential,
}

impl Default for ForkMode {
    fn default() -> Self {
        ForkMode::Parallel
    }
}

/// Where to forward a request and how to fan it out.
///
/// - `RouteDecision::to(addr)` — single target (no forking).
/// - `RouteDecision::parallel(vec![...])` — fork to all targets at once.
/// - `RouteDecision::sequential(vec![...])` — try targets in order.
/// - `RouteDecision::parallel_with_failover(vec![vec![..], ..])` —
///   per-leg RFC 3263 §4.3 candidate failover layered onto a parallel
///   fork (the outer vec is the fork list; each inner vec is the
///   candidate list the proxy walks on transport failure, 503, or an
///   authoritative no-response timeout).
/// - `RouteDecision::sequential_with_failover(vec![vec![..], ..])` —
///   same shape, sequential mode.
#[derive(Debug, Clone)]
pub struct RouteDecision {
    pub mode: ForkMode,
    pub targets: Vec<SocketAddr>,
    /// Optional per-leg RFC 3263 §4.3 candidate lists. When non-empty,
    /// each `Vec<SocketAddr>` is one leg's candidate list — the proxy
    /// tries entries in order on transport-level failure, downstream
    /// 503, or an authoritative no-response timeout. A received 408 is
    /// a real final response and does not advance the list. When empty,
    /// `targets` is used as a 1-element-per-leg candidate list (the
    /// pre-failover behaviour). Outer length defines the fork count; when
    /// both `targets` and `leg_candidates` are set, the latter wins.
    pub leg_candidates: Vec<Vec<SocketAddr>>,
}

impl RouteDecision {
    /// Single-target convenience — equivalent to a 1-element fork in
    /// `Sequential` mode, which is identical in behaviour to a
    /// 1-element parallel fork. Kept for Phase 6 backwards
    /// compatibility.
    pub fn to(destination: SocketAddr) -> Self {
        Self {
            mode: ForkMode::Sequential,
            targets: vec![destination],
            leg_candidates: Vec::new(),
        }
    }

    pub fn parallel(targets: Vec<SocketAddr>) -> Self {
        Self {
            mode: ForkMode::Parallel,
            targets,
            leg_candidates: Vec::new(),
        }
    }

    pub fn sequential(targets: Vec<SocketAddr>) -> Self {
        Self {
            mode: ForkMode::Sequential,
            targets,
            leg_candidates: Vec::new(),
        }
    }

    /// Parallel fork with per-leg candidate failover. Each inner
    /// `Vec<SocketAddr>` is one leg — the proxy walks the entries
    /// in RFC 3263 §4.3 candidate-failure order.
    pub fn parallel_with_failover(legs: Vec<Vec<SocketAddr>>) -> Self {
        let targets = legs.iter().filter_map(|leg| leg.first().copied()).collect();
        Self {
            mode: ForkMode::Parallel,
            targets,
            leg_candidates: legs,
        }
    }

    /// Sequential fork with per-leg candidate failover. Same shape as
    /// [`Self::parallel_with_failover`].
    pub fn sequential_with_failover(legs: Vec<Vec<SocketAddr>>) -> Self {
        let targets = legs.iter().filter_map(|leg| leg.first().copied()).collect();
        Self {
            mode: ForkMode::Sequential,
            targets,
            leg_candidates: legs,
        }
    }

    fn is_empty(&self) -> bool {
        self.targets.is_empty() && self.leg_candidates.is_empty()
    }
}

/// RFC 3263/SIPS-aware URI routing decision.
///
/// This is deliberately separate from [`RouteDecision`] so existing 0.3.1
/// struct literals remain source-compatible.
#[derive(Debug, Clone)]
pub struct UriRouteDecision {
    pub mode: ForkMode,
    pub targets: Vec<ProxyTarget>,
}

impl UriRouteDecision {
    /// Route one URI target as a single logical leg.
    pub fn to(target: ProxyTarget) -> Self {
        Self {
            mode: ForkMode::Sequential,
            targets: vec![target],
        }
    }

    /// Fork to every URI target at once.
    pub fn parallel(targets: Vec<ProxyTarget>) -> Self {
        Self {
            mode: ForkMode::Parallel,
            targets,
        }
    }

    /// Try URI targets in order.
    pub fn sequential(targets: Vec<ProxyTarget>) -> Self {
        Self {
            mode: ForkMode::Sequential,
            targets,
        }
    }

    fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }
}

enum SelectedRouteDecision {
    Socket(RouteDecision),
    Uri(UriRouteDecision),
}

impl SelectedRouteDecision {
    fn mode(&self) -> ForkMode {
        match self {
            Self::Socket(decision) => decision.mode,
            Self::Uri(decision) => decision.mode,
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            Self::Socket(decision) => decision.is_empty(),
            Self::Uri(decision) => decision.is_empty(),
        }
    }

    fn leg_count(&self) -> usize {
        match self {
            Self::Socket(decision) if !decision.leg_candidates.is_empty() => {
                decision.leg_candidates.len()
            }
            Self::Socket(decision) => decision.targets.len(),
            Self::Uri(decision) => decision.targets.len(),
        }
    }
}

enum RouteSelector {
    Socket(RouteFn),
    Uri(UriRouteFn),
    UriWithSocketFallback { uri: UriRouteFn, socket: RouteFn },
}

/// Configuration knobs for the stateful proxy. Apply via
/// [`StatefulProxy::with_config`] or by constructing via
/// `StatefulProxy::builder` (TODO Phase 7).
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// Timer C duration. Defaults to [`DEFAULT_TIMER_C`].
    pub timer_c: Duration,
    /// Whether to enforce Max-Forwards. When `true` (default), an
    /// inbound request with `Max-Forwards: 0` is rejected with 483.
    pub enforce_max_forwards: bool,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            timer_c: DEFAULT_TIMER_C,
            enforce_max_forwards: true,
        }
    }
}

/// Additive runtime and URI-routing controls that were not part of the 0.3.1
/// [`ProxyConfig`] source contract.
#[derive(Clone)]
pub struct ProxyRuntimeOptions {
    routing: ProxyRoutingPolicy,
    advertised_via: HashMap<TransportType, SocketAddr>,
    resolver: Arc<dyn Resolver>,
    allow_short_timer_c_for_tests: bool,
    legacy_branch_loop_detection: bool,
    response_context_capacity: usize,
    downstream_transaction_capacity: usize,
    branches_per_response_context: usize,
    stateless_response_route_capacity: usize,
}

impl std::fmt::Debug for ProxyRuntimeOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProxyRuntimeOptions")
            .field("routing", &self.routing)
            .field("advertised_via", &self.advertised_via)
            .field(
                "allow_short_timer_c_for_tests",
                &self.allow_short_timer_c_for_tests,
            )
            .field(
                "legacy_branch_loop_detection",
                &self.legacy_branch_loop_detection,
            )
            .field("response_context_capacity", &self.response_context_capacity)
            .field(
                "downstream_transaction_capacity",
                &self.downstream_transaction_capacity,
            )
            .field(
                "branches_per_response_context",
                &self.branches_per_response_context,
            )
            .field(
                "stateless_response_route_capacity",
                &self.stateless_response_route_capacity,
            )
            .finish_non_exhaustive()
    }
}

impl Default for ProxyRuntimeOptions {
    fn default() -> Self {
        Self {
            routing: ProxyRoutingPolicy::default(),
            advertised_via: HashMap::new(),
            resolver: Arc::new(DefaultProxyResolver::default()),
            allow_short_timer_c_for_tests: false,
            legacy_branch_loop_detection: false,
            response_context_capacity: DEFAULT_RESPONSE_CONTEXT_CAPACITY,
            downstream_transaction_capacity: DEFAULT_DOWNSTREAM_TRANSACTION_CAPACITY,
            branches_per_response_context: DEFAULT_BRANCHES_PER_RESPONSE_CONTEXT,
            stateless_response_route_capacity: DEFAULT_STATELESS_RESPONSE_ROUTE_CAPACITY,
        }
    }
}

impl ProxyRuntimeOptions {
    /// Install a validated request-routing policy.
    pub fn with_routing_policy(mut self, routing: ProxyRoutingPolicy) -> Self {
        self.routing = routing;
        self
    }

    /// Install the resolver used for URI route decisions.
    pub fn with_resolver(mut self, resolver: Arc<dyn Resolver>) -> Self {
        self.resolver = resolver;
        self
    }

    /// Advertise `sent_by` in Via for one exact outbound transport.
    pub fn with_advertised_via(mut self, transport: TransportType, sent_by: SocketAddr) -> Self {
        self.advertised_via.insert(transport, sent_by);
        self
    }

    /// Bound the number of retained upstream response contexts.
    ///
    /// Capacity is admission-only: when full, a new request receives a local
    /// `503 Service Unavailable`; an existing response context is never
    /// evicted to make room.
    pub fn with_response_context_capacity(mut self, capacity: usize) -> Self {
        self.response_context_capacity = capacity;
        self
    }

    /// Bound the total downstream transaction slots reserved by retained
    /// response contexts.
    ///
    /// A context reserves its complete initial fork set before it is admitted,
    /// so a sequential fork cannot overcommit capacity later. Redirect-created
    /// forks reserve additional slots before consuming the redirect.
    pub fn with_downstream_transaction_capacity(mut self, capacity: usize) -> Self {
        self.downstream_transaction_capacity = capacity;
        self
    }

    /// Bound the total number of downstream branches one response context may
    /// create, including redirect-created branches.
    pub fn with_branches_per_response_context(mut self, capacity: usize) -> Self {
        self.branches_per_response_context = capacity;
        self
    }

    /// Bound retained stateless response correlations (for example, an
    /// unmatched CANCEL forwarded according to RFC 3261 section 16.10).
    ///
    /// The proxy rejects a new correlation at capacity rather than evicting a
    /// live route that may still own an authenticated response.
    pub fn with_stateless_response_route_capacity(mut self, capacity: usize) -> Self {
        self.stateless_response_route_capacity = capacity;
        self
    }

    /// Enable the historical branch-membership loop detector in tests. It is
    /// disabled by default because it cannot distinguish a loop from a legal
    /// spiral and its branch set alone is not an RFC 3261 loop signature.
    #[doc(hidden)]
    pub fn with_legacy_loop_detection_for_tests(mut self) -> Self {
        self.legacy_branch_loop_detection = true;
        self
    }

    /// Allow a short Timer C only in deterministic conformance tests.
    #[doc(hidden)]
    pub fn with_short_timer_c_for_tests(mut self) -> Self {
        self.allow_short_timer_c_for_tests = true;
        self
    }

    fn validate(&self, config: &ProxyConfig) -> Result<(), ProxyBuildError> {
        if config.timer_c <= Duration::from_secs(180) && !self.allow_short_timer_c_for_tests {
            return Err(ProxyBuildError::InvalidConfiguration(
                "Timer C must be strictly greater than 180 seconds".into(),
            ));
        }
        if self.response_context_capacity == 0
            || self.downstream_transaction_capacity == 0
            || self.branches_per_response_context == 0
            || self.stateless_response_route_capacity == 0
        {
            return Err(ProxyBuildError::InvalidConfiguration(
                "proxy state capacities must all be greater than zero".into(),
            ));
        }
        self.routing
            .validate()
            .map_err(|error| ProxyBuildError::InvalidConfiguration(error.to_string()))?;
        if self
            .advertised_via
            .values()
            .any(|address| address.port() == 0 || address.ip().is_unspecified())
        {
            return Err(ProxyBuildError::InvalidConfiguration(
                "advertised Via addresses require a concrete IP and nonzero port".into(),
            ));
        }
        Ok(())
    }
}

/// Per-fork state for a single inbound request.
///
/// A `ForkContext` aggregates 1..N downstream legs against a single
/// upstream server transaction. The single-target (Phase 6) case is
/// just an N=1 fork.
struct ForkContext {
    upstream_server_tx: TransactionKey,
    is_invite: bool,
    mode: ForkMode,
    /// Original inbound request — used to (a) re-forward to the next
    /// sequential target on failure and (b) build upstream responses
    /// (Timer C 408, 483, 404) with the correct From/To/Call-ID/CSeq
    /// /Via stack per RFC 3261 §8.2.6.2.
    original_request: Request,
    /// Target-specific requests and exact RFC 3263 candidate routes. A logical
    /// target keeps its own request because strict/loose Route processing and
    /// Record-Route insertion can differ across application-selected targets.
    leg_plans: Vec<LegPlan>,
    /// Number of legs already started — used by sequential mode to
    /// advance to the next leg index on failure (replaces the prior
    /// address-set scan which broke once a leg could have multiple
    /// candidates).
    legs_started: AtomicUsize,
    /// Logical branches whose complete RFC 3263 candidate set failed before a
    /// downstream transaction reached its first transport write. Each failure
    /// participates in response aggregation as a branch-local 503.
    startup_failures: AtomicUsize,
    /// Per-leg state. In Parallel mode, all legs are populated up-front.
    /// In Sequential mode, legs are populated one at a time as earlier
    /// ones fail.
    legs: tokio::sync::Mutex<Vec<Leg>>,
    /// Serialized upstream final-response ownership. The mutex itself is the
    /// transient "Sending" state: ownership stays `Unsent` until the
    /// transaction layer classifies the write as successful, so a proven
    /// zero-wire failure cannot pre-latch the response context.
    upstream_final_dispatch: tokio::sync::Mutex<UpstreamFinalDispatch>,
    /// Once set, no new sequential or redirect-created branch may start.
    stop_new_branches: AtomicBool,
    upstream_cancelled: AtomicBool,
    upstream_terminated: AtomicBool,
    /// Capacity reserved from the proxy-wide downstream transaction budget.
    /// Initial sequential forks reserve their complete target set up front;
    /// redirect-created branches extend this reservation before starting.
    reserved_downstream_slots: AtomicUsize,
    /// Exact socket targets already present in this response context's legacy
    /// address-based target set. This bounds recursive redirect handling and
    /// enforces RFC 3261 section 16.5's add-each-target-once rule for the
    /// source-compatible `RedirectDecision::ReFork` surface.
    attempted_redirect_targets: tokio::sync::Mutex<HashSet<SocketAddr>>,
    /// Request after validation, inbound Route preprocessing, and the one
    /// Max-Forwards update shared by every branch in this response context.
    /// Redirect-created branches start from this copy rather than resetting
    /// processing against the original inbound request.
    forwarding_request: Request,
}

#[derive(Clone)]
struct LegPlan {
    request: Request,
    candidates: Vec<TransportRoute>,
    label: String,
    preparation_failure: Option<String>,
}

/// Remaining RFC 3263 candidates for one logical branch after a request has
/// crossed the first-write boundary on its current candidate.
///
/// The active transaction keeps this continuation rather than treating every
/// candidate as a new fork leg. A 503 or an authoritative no-response timeout
/// consumes it exactly once and starts the next candidate with a fresh Via
/// branch.
#[derive(Clone)]
struct CandidateContinuation {
    request: Request,
    candidates: Vec<TransportRoute>,
    completed_attempts: usize,
    total_attempts: usize,
    label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpstreamFinalDispatch {
    Unsent,
    Sent,
    /// RFC 6026 terminal discard: the exact upstream server transaction can no
    /// longer safely author a response, or the write outcome is unknown.
    TerminalDiscard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FinalDispatchOutcome {
    FirstSent,
    AdditionalInvite2xxSent,
    AlreadySent,
    TerminalDiscard,
}

#[derive(Debug)]
enum ClassifiedUpstreamSend {
    Sent,
    Retryable(ProxyError),
    TerminalDiscard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegState {
    Calling,
    Proceeding,
    Completed,
    Terminated,
}

struct Leg {
    downstream_client_tx: TransactionKey,
    /// Final response received on this leg, if any. `None` while the
    /// leg is still pending; `Some(status)` once a final response has
    /// arrived. Forward-progress is tracked here so the aggregator can
    /// decide whether all legs are "done".
    final_status: Option<StatusCode>,
    state: LegState,
    /// Cancellation is latched while Calling. RFC 3261 §9.1 prohibits
    /// sending CANCEL before a provisional response establishes that the
    /// downstream INVITE is known to the UAS.
    cancel_requested: bool,
    cancel_sent: bool,
    /// Backoff for a proven zero-wire generated-CANCEL retry. The first retry
    /// is T1 and subsequent retries double up to T2.
    cancel_retry_delay: Option<Duration>,
    /// Exact proxy-generated CANCEL transaction retained until the complete
    /// response context expires. Its terminal state participates in the
    /// context drain fence.
    generated_cancel: Option<Arc<GeneratedCancelTransaction>>,
    /// The best response received on this leg, kept so §16.7 step 6
    /// "best response" selection works after all legs settle.
    last_response: Option<Response>,
    /// Remaining addresses for this logical RFC 3263 branch. This is consumed
    /// exactly once when the current candidate returns 503 or times out
    /// without a response.
    candidate_continuation: Option<CandidateContinuation>,
    /// A completed candidate attempt that was replaced by a later candidate
    /// must not regain ownership through a retransmitted final or late
    /// timeout event.
    superseded_by_candidate: bool,
    /// Exact one-owner fence while a retryable terminal condition is starting
    /// the next candidate. The old attempt is already non-live while this is
    /// set, so duplicate failures, cancellation, and a fast replacement final
    /// cannot observe two active attempts for one logical branch.
    candidate_advancement_in_progress: bool,
    proxy_branch: String,
    forwarded_request: Request,
}

impl Leg {
    fn is_finished(&self) -> bool {
        !self.candidate_advancement_in_progress
            && matches!(self.state, LegState::Completed | LegState::Terminated)
    }
}

#[derive(Debug)]
struct GeneratedCancelTransaction {
    transaction_id: TransactionKey,
    /// Shared by the leg and the exact transaction index so a terminal event
    /// that races the dispatch return cannot be overwritten by later
    /// registration.
    terminated: AtomicBool,
    /// Set only after the transaction manager proves the failed dispatch
    /// never crossed its first-write boundary. The exact terminal event for
    /// that internal generation must be consumed before the RFC key can be
    /// reused safely.
    retryable_zero_wire: AtomicBool,
    /// One side of the classification/terminal race owns releasing the
    /// zero-wire generation and scheduling its retry.
    retry_release_claimed: AtomicBool,
}

#[derive(Clone)]
struct GeneratedCancelOwner {
    fork: Arc<ForkContext>,
    invite_transaction_id: TransactionKey,
    generation: Arc<GeneratedCancelTransaction>,
}

#[derive(Clone, Eq, PartialEq)]
struct TimerCEntry {
    deadline: Instant,
    generation: u64,
    downstream_tx: TransactionKey,
}

impl Ord for TimerCEntry {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        // BinaryHeap is a max-heap. Reverse the time ordering so the
        // earliest deadline is at the top.
        other
            .deadline
            .cmp(&self.deadline)
            .then_with(|| other.generation.cmp(&self.generation))
    }
}

impl PartialOrd for TimerCEntry {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

#[derive(Default)]
struct TimerCSchedule {
    heap: BinaryHeap<TimerCEntry>,
    current: HashMap<TransactionKey, (Instant, u64)>,
}

fn compact_timer_c_heap_if_needed(schedule: &mut TimerCSchedule) {
    let live = schedule.current.len();
    if live == 0 {
        schedule.heap.clear();
        return;
    }
    let maximum_physical_entries = live
        .saturating_mul(TIMER_C_HEAP_COMPACTION_FACTOR)
        .saturating_add(TIMER_C_HEAP_COMPACTION_SLACK);
    if schedule.heap.len() <= maximum_physical_entries {
        return;
    }
    schedule.heap = schedule
        .current
        .iter()
        .map(|(downstream_tx, (deadline, generation))| TimerCEntry {
            deadline: *deadline,
            generation: *generation,
            downstream_tx: downstream_tx.clone(),
        })
        .collect();
}

fn schedule_timer_c_entry(
    schedule: &mut TimerCSchedule,
    downstream_tx: TransactionKey,
    deadline: Instant,
    generation: u64,
) {
    schedule
        .current
        .insert(downstream_tx.clone(), (deadline, generation));
    schedule.heap.push(TimerCEntry {
        deadline,
        generation,
        downstream_tx,
    });
    compact_timer_c_heap_if_needed(schedule);
}

fn take_expired_timer_c_entries(
    schedule: &mut TimerCSchedule,
    now: Instant,
) -> Vec<TransactionKey> {
    let mut expired = Vec::new();
    let mut processed = 0;
    while processed < TIMER_C_EXPIRY_BATCH
        && schedule
            .heap
            .peek()
            .is_some_and(|entry| entry.deadline <= now)
    {
        let entry = schedule.heap.pop().expect("peeked Timer C entry");
        processed += 1;
        if schedule.current.get(&entry.downstream_tx).copied()
            == Some((entry.deadline, entry.generation))
        {
            schedule.current.remove(&entry.downstream_tx);
            expired.push(entry.downstream_tx);
        }
    }
    compact_timer_c_heap_if_needed(schedule);
    expired
}

#[derive(Clone, Eq, PartialEq)]
struct ForkExpiryEntry {
    deadline: Instant,
    generation: u64,
    upstream_tx: TransactionKey,
}

impl Ord for ForkExpiryEntry {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        other
            .deadline
            .cmp(&self.deadline)
            .then_with(|| other.generation.cmp(&self.generation))
    }
}

impl PartialOrd for ForkExpiryEntry {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

#[derive(Default)]
struct ForkExpirySchedule {
    heap: BinaryHeap<ForkExpiryEntry>,
    current: HashMap<TransactionKey, (Instant, u64)>,
}

#[derive(Clone)]
struct StatelessResponseRoute {
    upstream_route: TransportRoute,
    downstream_route: TransportRoute,
    cseq_sequence: u32,
    cseq_method: Method,
    expires_at: Instant,
}

#[derive(Clone, Eq, PartialEq)]
struct StatelessResponseExpiryEntry {
    deadline: Instant,
    branch: String,
}

impl Ord for StatelessResponseExpiryEntry {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        other
            .deadline
            .cmp(&self.deadline)
            .then_with(|| other.branch.cmp(&self.branch))
    }
}

impl PartialOrd for StatelessResponseExpiryEntry {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

#[derive(Default)]
struct StatelessResponseExpirySchedule {
    heap: BinaryHeap<StatelessResponseExpiryEntry>,
    current: HashMap<String, Instant>,
}

fn compact_stateless_response_heap_if_needed(schedule: &mut StatelessResponseExpirySchedule) {
    let live = schedule.current.len();
    if live == 0 {
        schedule.heap.clear();
        return;
    }
    let maximum_physical_entries = live
        .saturating_mul(STATELESS_RESPONSE_HEAP_COMPACTION_FACTOR)
        .saturating_add(STATELESS_RESPONSE_HEAP_COMPACTION_SLACK);
    if schedule.heap.len() <= maximum_physical_entries {
        return;
    }
    schedule.heap = schedule
        .current
        .iter()
        .map(|(branch, deadline)| StatelessResponseExpiryEntry {
            deadline: *deadline,
            branch: branch.clone(),
        })
        .collect();
}

/// Stateful SIP proxy actor.
///
/// Spawn via [`StatefulProxy::run`] passing a routing function. The
/// returned `JoinHandle` runs until the underlying
/// [`TransactionManager`] event stream closes.
/// Hook the proxy fires on every 3xx response received from a
/// downstream leg. Implementations can opt to re-fork the call to a
/// new target set instead of forwarding the 3xx upstream — typical
/// use case is an application that consults its own location service
/// or recursive-redirect policy.
///
/// Returning `RedirectDecision::Forward` (or `None` via the `Option`
/// return) sends the 3xx upstream verbatim, preserving the prior
/// observability-only behaviour. Returning
/// `RedirectDecision::ReFork(...)` swallows the 3xx, marks the leg
/// as cancelled (so it doesn't influence best-failure selection),
/// and spawns fresh downstream legs for the new targets.
#[async_trait::async_trait]
pub trait RedirectInterceptor: Send + Sync {
    async fn on_redirect(&self, info: RedirectInfo) -> Option<RedirectDecision>;
}

/// Aggregate-safe snapshot of bounded proxy response-context state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProxyRetentionSnapshot {
    /// Retained upstream response contexts.
    pub response_contexts: usize,
    /// Retained downstream INVITE transaction indexes.
    pub downstream_invite_indexes: usize,
    /// Exact proxy-generated CANCEL transaction indexes.
    pub generated_cancel_transactions: usize,
    /// Live Timer C entries.
    pub timer_c_entries: usize,
    /// Physical Timer C heap entries, including bounded stale reset entries.
    pub timer_c_heap_entries: usize,
    /// Proven-zero-wire CANCEL retries waiting in the centralized deadline
    /// queue.
    pub generated_cancel_retry_entries: usize,
    /// Physical generated-CANCEL retry heap entries, including bounded stale
    /// backoff generations.
    pub generated_cancel_retry_heap_entries: usize,
    /// Response contexts waiting on their retention deadline.
    pub response_context_deadlines: usize,
    /// Stateless response routes retained for authenticated response
    /// retransmissions until expiry.
    pub stateless_response_routes: usize,
    /// Branches retained by the opt-in legacy loop detector.
    pub known_branches: usize,
    /// Downstream branch slots reserved by retained response contexts.
    pub downstream_slot_reservations: usize,
    /// Physical response-context expiry heap entries.
    pub response_context_deadline_heap_entries: usize,
    /// Live stateless response-route expiry entries.
    pub stateless_response_route_deadlines: usize,
    /// Physical stateless response-route expiry heap entries.
    pub stateless_response_route_deadline_heap_entries: usize,
}

/// Snapshot of the 3xx response handed to a [`RedirectInterceptor`].
#[derive(Debug, Clone)]
pub struct RedirectInfo {
    /// Upstream server transaction key the 3xx applies to.
    pub upstream_tx: TransactionKey,
    /// The 3xx status code that arrived.
    pub status: rvoip_sip_core::types::status::StatusCode,
    /// Contact URIs extracted from the 3xx response (RFC 3261 §16.7
    /// step 2 — redirect target set).
    pub contacts: Vec<rvoip_sip_core::Uri>,
}

/// Application decision in response to a 3xx redirect.
#[derive(Debug, Clone)]
pub enum RedirectDecision {
    /// Forward the 3xx upstream verbatim (default — same as no
    /// interceptor installed).
    Forward,
    /// Don't forward the 3xx upstream; instead spawn new downstream
    /// legs against `targets` in the supplied [`ForkMode`].
    ReFork {
        mode: ForkMode,
        targets: Vec<SocketAddr>,
    },
}

pub struct StatefulProxy {
    tm: Arc<TransactionManager>,
    config: ProxyConfig,
    options: ProxyRuntimeOptions,
    route_selector: RouteSelector,
    proxy_ingress_rx: StdMutex<Option<mpsc::Receiver<StatefulProxyIngressEvent>>>,

    /// Fork contexts keyed by the upstream server transaction.
    forks_by_upstream: DashMap<TransactionKey, Arc<ForkContext>>,
    /// Reverse lookup: downstream client-tx → fork context. Populated
    /// every time a leg is started; cleaned up when the leg terminates.
    forks_by_downstream: DashMap<TransactionKey, Arc<ForkContext>>,
    /// Exact generated-CANCEL transaction → owning INVITE leg. Entries remain
    /// until the parent response context expires, even after the CANCEL
    /// transaction itself terminates.
    generated_cancels: DashMap<TransactionKey, GeneratedCancelOwner>,
    /// Slots reserved by admitted response contexts. This is intentionally
    /// separate from the live downstream index length: sequential forks must
    /// reserve future branches before admission to avoid later overcommit.
    downstream_slot_reservations: AtomicUsize,

    /// One manager-owned Timer C deadline queue for every downstream
    /// INVITE branch. No per-transaction sleeper tasks are created.
    timer_c_schedule: StdMutex<TimerCSchedule>,
    timer_c_generation: AtomicU64,
    /// A separate centralized T1..T2 retry queue for proven-zero-wire
    /// generated CANCEL attempts. It must not reuse or extend production
    /// Timer C semantics.
    cancel_retry_schedule: StdMutex<TimerCSchedule>,
    cancel_retry_generation: AtomicU64,
    fork_expiry_schedule: StdMutex<ForkExpirySchedule>,
    fork_expiry_generation: AtomicU64,

    /// Set of `z9hG4bK-proxy-…` branches this proxy has stamped on
    /// outbound Vias. Used for RFC 3261 §16.6 step 4 loop detection:
    /// if an inbound request's Via stack contains a branch in this
    /// set, the request looped back through us and we reject with
    /// 482 Loop Detected. `DashMap<String, ()>` is used as a
    /// concurrent set (DashSet isn't in the dependency tree).
    known_branches: DashMap<String, ()>,
    /// Response routes for truly stateless requests such as unmatched
    /// CANCEL. Entries remain available for authenticated response
    /// retransmissions until their bounded retention deadline.
    stateless_response_routes: DashMap<String, StatelessResponseRoute>,
    stateless_response_expiry_schedule: StdMutex<StatelessResponseExpirySchedule>,

    /// Application-observable event stream. Receivers obtained via
    /// [`Self::subscribe_events`]. `broadcast::Sender` is cloned per
    /// subscriber and survives subscriber-side drops, so the proxy
    /// never blocks on unread events.
    event_tx: broadcast::Sender<ProxyEvent>,

    /// Optional 3xx interception hook. When installed, the proxy
    /// consults the interceptor on every 3xx and lets it choose
    /// between forwarding (default) and re-forking to a new target
    /// set. See [`RedirectInterceptor`].
    redirect_interceptor: std::sync::RwLock<Option<Arc<dyn RedirectInterceptor>>>,
}

impl std::fmt::Debug for StatefulProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StatefulProxy")
            .field("config", &self.config)
            .field("options", &self.options)
            .field("forks", &self.forks_by_upstream.len())
            .field(
                "downstream_slot_reservations",
                &self.downstream_slot_reservations.load(Ordering::Acquire),
            )
            .field("generated_cancels", &self.generated_cancels.len())
            .field(
                "timer_c_entries",
                &self
                    .timer_c_schedule
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .current
                    .len(),
            )
            .field(
                "generated_cancel_retry_entries",
                &self
                    .cancel_retry_schedule
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .current
                    .len(),
            )
            .finish()
    }
}

impl StatefulProxy {
    /// Build a proxy with default configuration.
    pub fn new(tm: Arc<TransactionManager>, route_fn: RouteFn) -> Arc<Self> {
        Self::with_config(tm, route_fn, ProxyConfig::default())
    }

    pub fn with_config(
        tm: Arc<TransactionManager>,
        route_fn: RouteFn,
        config: ProxyConfig,
    ) -> Arc<Self> {
        Self::try_with_config(tm, route_fn, config)
            .expect("StatefulProxy configuration must be valid")
    }

    /// Build a proxy after validating production protocol invariants.
    pub fn try_with_config(
        tm: Arc<TransactionManager>,
        route_fn: RouteFn,
        config: ProxyConfig,
    ) -> Result<Arc<Self>, ProxyBuildError> {
        Self::try_with_options(tm, route_fn, config, ProxyRuntimeOptions::default())
    }

    /// Build a socket-address-routed proxy with additive runtime options.
    pub fn with_options(
        tm: Arc<TransactionManager>,
        route_fn: RouteFn,
        config: ProxyConfig,
        options: ProxyRuntimeOptions,
    ) -> Arc<Self> {
        Self::try_with_options(tm, route_fn, config, options)
            .expect("StatefulProxy configuration must be valid")
    }

    /// Fallible form of [`Self::with_options`].
    pub fn try_with_options(
        tm: Arc<TransactionManager>,
        route_fn: RouteFn,
        config: ProxyConfig,
        options: ProxyRuntimeOptions,
    ) -> Result<Arc<Self>, ProxyBuildError> {
        Self::try_with_route_selector(tm, RouteSelector::Socket(route_fn), config, options)
    }

    /// Build an RFC 3263/SIPS-aware URI-routed proxy.
    pub fn with_uri_routes(
        tm: Arc<TransactionManager>,
        route_fn: UriRouteFn,
        config: ProxyConfig,
        options: ProxyRuntimeOptions,
    ) -> Arc<Self> {
        Self::try_with_uri_routes(tm, route_fn, config, options)
            .expect("StatefulProxy configuration must be valid")
    }

    /// Fallible form of [`Self::with_uri_routes`].
    pub fn try_with_uri_routes(
        tm: Arc<TransactionManager>,
        route_fn: UriRouteFn,
        config: ProxyConfig,
        options: ProxyRuntimeOptions,
    ) -> Result<Arc<Self>, ProxyBuildError> {
        Self::try_with_route_selector(tm, RouteSelector::Uri(route_fn), config, options)
    }

    /// Build a proxy that can migrate selected routes to RFC 3263 URI
    /// resolution while retaining an exact pre-resolved socket fallback.
    ///
    /// The URI callback is authoritative when it returns a decision. When it
    /// returns `None`, the socket callback is consulted. This is useful for
    /// applications that need DNS/SIPS routing on only part of their route
    /// table without changing legacy route behavior.
    pub fn with_uri_routes_and_socket_fallback(
        tm: Arc<TransactionManager>,
        uri_route_fn: UriRouteFn,
        socket_route_fn: RouteFn,
        config: ProxyConfig,
        options: ProxyRuntimeOptions,
    ) -> Arc<Self> {
        Self::try_with_uri_routes_and_socket_fallback(
            tm,
            uri_route_fn,
            socket_route_fn,
            config,
            options,
        )
        .expect("StatefulProxy configuration must be valid")
    }

    /// Fallible form of [`Self::with_uri_routes_and_socket_fallback`].
    pub fn try_with_uri_routes_and_socket_fallback(
        tm: Arc<TransactionManager>,
        uri_route_fn: UriRouteFn,
        socket_route_fn: RouteFn,
        config: ProxyConfig,
        options: ProxyRuntimeOptions,
    ) -> Result<Arc<Self>, ProxyBuildError> {
        Self::try_with_route_selector(
            tm,
            RouteSelector::UriWithSocketFallback {
                uri: uri_route_fn,
                socket: socket_route_fn,
            },
            config,
            options,
        )
    }

    /// Build a proxy with an application-supplied RFC 3263 resolver.
    ///
    /// The resolver is consulted only for URI [`ProxyTarget`] legs. Legacy
    /// pre-resolved `SocketAddr` routes remain I/O-free and fully compatible.
    pub fn with_config_and_resolver(
        tm: Arc<TransactionManager>,
        route_fn: RouteFn,
        config: ProxyConfig,
        resolver: Arc<dyn Resolver>,
    ) -> Arc<Self> {
        Self::try_with_config_and_resolver(tm, route_fn, config, resolver)
            .expect("StatefulProxy configuration must be valid")
    }

    /// Fallible form of [`Self::with_config_and_resolver`].
    pub fn try_with_config_and_resolver(
        tm: Arc<TransactionManager>,
        route_fn: RouteFn,
        config: ProxyConfig,
        resolver: Arc<dyn Resolver>,
    ) -> Result<Arc<Self>, ProxyBuildError> {
        Self::try_with_options(
            tm,
            route_fn,
            config,
            ProxyRuntimeOptions::default().with_resolver(resolver),
        )
    }

    fn try_with_route_selector(
        tm: Arc<TransactionManager>,
        route_selector: RouteSelector,
        config: ProxyConfig,
        options: ProxyRuntimeOptions,
    ) -> Result<Arc<Self>, ProxyBuildError> {
        options.validate(&config)?;
        let proxy_ingress_rx = tm
            .try_claim_stateful_proxy_ingress(1_024)
            .map_err(|error| ProxyBuildError::Transaction(error.to_string()))?;
        Ok(Arc::new(Self {
            tm,
            config,
            options,
            route_selector,
            proxy_ingress_rx: StdMutex::new(Some(proxy_ingress_rx)),
            forks_by_upstream: DashMap::new(),
            forks_by_downstream: DashMap::new(),
            generated_cancels: DashMap::new(),
            downstream_slot_reservations: AtomicUsize::new(0),
            timer_c_schedule: StdMutex::new(TimerCSchedule::default()),
            timer_c_generation: AtomicU64::new(0),
            cancel_retry_schedule: StdMutex::new(TimerCSchedule::default()),
            cancel_retry_generation: AtomicU64::new(0),
            fork_expiry_schedule: StdMutex::new(ForkExpirySchedule::default()),
            fork_expiry_generation: AtomicU64::new(0),
            known_branches: DashMap::new(),
            stateless_response_routes: DashMap::new(),
            stateless_response_expiry_schedule: StdMutex::new(
                StatelessResponseExpirySchedule::default(),
            ),
            event_tx: broadcast::channel(64).0,
            redirect_interceptor: std::sync::RwLock::new(None),
        }))
    }

    /// Install (or replace) a [`RedirectInterceptor`]. Apps that want
    /// to re-fork on 3xx instead of forwarding the redirect upstream
    /// supply an interceptor here.
    pub fn set_redirect_interceptor(&self, interceptor: Option<Arc<dyn RedirectInterceptor>>) {
        *self
            .redirect_interceptor
            .write()
            .expect("redirect_interceptor RwLock poisoned") = interceptor;
    }

    fn redirect_interceptor(&self) -> Option<Arc<dyn RedirectInterceptor>> {
        self.redirect_interceptor
            .read()
            .expect("redirect_interceptor RwLock poisoned")
            .clone()
    }

    /// Subscribe to observable proxy events ([`ProxyEvent`]). Drop
    /// the returned receiver to unsubscribe. Lagging subscribers may
    /// miss events (broadcast semantics) — applications that care
    /// about every redirect should drain the receiver promptly.
    pub fn subscribe_events(&self) -> broadcast::Receiver<ProxyEvent> {
        self.event_tx.subscribe()
    }

    /// Return aggregate retention counts without exposing call identifiers.
    pub fn retention_snapshot(&self) -> ProxyRetentionSnapshot {
        let timer_c_schedule = self
            .timer_c_schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let timer_c_entries = timer_c_schedule.current.len();
        let timer_c_heap_entries = timer_c_schedule.heap.len();
        drop(timer_c_schedule);
        let cancel_retry_schedule = self
            .cancel_retry_schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let generated_cancel_retry_entries = cancel_retry_schedule.current.len();
        let generated_cancel_retry_heap_entries = cancel_retry_schedule.heap.len();
        drop(cancel_retry_schedule);
        let fork_expiry_schedule = self
            .fork_expiry_schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let response_context_deadlines = fork_expiry_schedule.current.len();
        let response_context_deadline_heap_entries = fork_expiry_schedule.heap.len();
        drop(fork_expiry_schedule);
        let stateless_schedule = self
            .stateless_response_expiry_schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let stateless_response_route_deadlines = stateless_schedule.current.len();
        let stateless_response_route_deadline_heap_entries = stateless_schedule.heap.len();
        drop(stateless_schedule);
        ProxyRetentionSnapshot {
            response_contexts: self.forks_by_upstream.len(),
            downstream_invite_indexes: self.forks_by_downstream.len(),
            generated_cancel_transactions: self.generated_cancels.len(),
            timer_c_entries,
            timer_c_heap_entries,
            generated_cancel_retry_entries,
            generated_cancel_retry_heap_entries,
            response_context_deadlines,
            stateless_response_routes: self.stateless_response_routes.len(),
            known_branches: self.known_branches.len(),
            downstream_slot_reservations: self.downstream_slot_reservations.load(Ordering::Acquire),
            response_context_deadline_heap_entries,
            stateless_response_route_deadlines,
            stateless_response_route_deadline_heap_entries,
        }
    }

    /// Spawn the proxy event loop, consuming the primary
    /// `TransactionEvent` stream returned by `TransactionManager::new`.
    /// The returned handle runs until the stream closes.
    ///
    /// Use the primary stream — not [`TransactionManager::subscribe`] —
    /// because `subscribe()` registers asynchronously and would race
    /// with the first inbound request. The proxy MUST be the sole
    /// consumer of the primary stream for the lifetime of the manager;
    /// mixed-mode (proxy + dialog UA on the same manager) is out of
    /// scope for Phase 6.
    pub fn run(self: Arc<Self>, events: mpsc::Receiver<TransactionEvent>) -> JoinHandle<()> {
        let proxy_ingress = self
            .proxy_ingress_rx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .expect("StatefulProxy::run may only be called once");
        tokio::spawn(async move {
            self.event_loop(events, proxy_ingress).await;
        })
    }

    async fn event_loop(
        self: Arc<Self>,
        mut rx: mpsc::Receiver<TransactionEvent>,
        mut proxy_ingress: mpsc::Receiver<StatefulProxyIngressEvent>,
    ) {
        info!("StatefulProxy event loop started");
        loop {
            let deadline = [
                self.next_timer_c_deadline(),
                self.next_cancel_retry_deadline(),
                self.next_fork_expiry_deadline(),
                self.next_stateless_response_expiry_deadline(),
            ]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(24 * 60 * 60));
            tokio::select! {
                event = rx.recv() => {
                    let Some(event) = event else {
                        break;
                    };
                    self.clone().handle_transaction_event(event).await;
                }
                ingress = proxy_ingress.recv() => {
                    let Some(ingress) = ingress else {
                        warn!("StatefulProxy exact ingress channel closed");
                        break;
                    };
                    self.clone().handle_proxy_ingress_event(ingress).await;
                }
                _ = tokio::time::sleep_until(deadline) => {
                    self.expire_timer_c().await;
                    self.expire_cancel_retries().await;
                    self.expire_fork_contexts().await;
                    self.expire_stateless_response_routes();
                }
            }
        }
        info!("StatefulProxy event loop exited");
    }

    async fn handle_proxy_ingress_event(self: Arc<Self>, event: StatefulProxyIngressEvent) {
        match event {
            StatefulProxyIngressEvent::UnmatchedCancelRequest {
                request,
                response_route,
                ..
            } => {
                if let Err(error) = self
                    .forward_stateless_request(request, Some(response_route))
                    .await
                {
                    warn!(
                        "proxy: stateless unmatched CANCEL forwarding failed: {}",
                        error
                    );
                }
            }
            StatefulProxyIngressEvent::StrayResponse {
                response,
                source,
                response_route,
            } => {
                if let Err(error) = self
                    .forward_stateless_response(response, source, response_route.as_ref())
                    .await
                {
                    warn!("proxy: stateless response forwarding failed: {}", error);
                }
            }
        }
    }

    async fn handle_transaction_event(self: Arc<Self>, event: TransactionEvent) {
        match event {
            TransactionEvent::InviteRequest {
                transaction_id,
                request,
                source,
            } => {
                if let Err(e) = self
                    .clone()
                    .handle_inbound_request(transaction_id, request, source, true)
                    .await
                {
                    warn!("proxy: forward INVITE failed: {}", e);
                }
            }
            TransactionEvent::NonInviteRequest {
                transaction_id,
                request,
                source,
            } => {
                if request.method() == Method::Cancel {
                    let upstream_route = self
                        .tm
                        .server_transaction_response_route(&transaction_id)
                        .unwrap_or_else(|| TransportRoute::new(source));
                    if let Err(e) = self
                        .forward_stateless_request(request, Some(upstream_route))
                        .await
                    {
                        warn!("proxy: stateless unmatched CANCEL forwarding failed: {}", e);
                    }
                } else if let Err(e) = self
                    .clone()
                    .handle_inbound_request(transaction_id, request, source, false)
                    .await
                {
                    warn!("proxy: forward request failed: {}", e);
                }
            }
            TransactionEvent::ProvisionalResponse {
                transaction_id,
                response,
            } => {
                if self.generated_cancels.contains_key(&transaction_id) {
                    trace!("proxy: consumed provisional response to generated CANCEL");
                } else if let Err(e) = self
                    .aggregate_response(transaction_id, response, /* final */ false)
                    .await
                {
                    warn!("proxy: aggregate 1xx failed: {}", e);
                }
            }
            TransactionEvent::SuccessResponse {
                transaction_id,
                response,
                ..
            } => {
                if self.generated_cancels.contains_key(&transaction_id) {
                    trace!("proxy: consumed successful response to generated CANCEL");
                } else if let Err(e) = self
                    .aggregate_response(transaction_id, response, /* final */ true)
                    .await
                {
                    warn!("proxy: aggregate 2xx failed: {}", e);
                }
            }
            TransactionEvent::FailureResponse {
                transaction_id,
                response,
            } => {
                if self.generated_cancels.contains_key(&transaction_id) {
                    trace!("proxy: consumed failure response to generated CANCEL");
                } else if let Err(e) = self
                    .aggregate_response(transaction_id, response, /* final */ true)
                    .await
                {
                    warn!("proxy: aggregate final failed: {}", e);
                }
            }
            TransactionEvent::CancelRequest {
                transaction_id: cancel_tx,
                target_transaction_id,
                request,
                ..
            } => {
                // RFC 3261 §16.10: the CANCEL server transaction is
                // independent and is completed immediately. The target
                // INVITE response context then fans cancellation out.
                if let Err(e) = self
                    .respond_locally(&cancel_tx, &request, StatusCode::Ok)
                    .await
                {
                    warn!("proxy: matched CANCEL 200 response failed: {}", e);
                }
                self.handle_upstream_cancel(&target_transaction_id).await;
            }
            TransactionEvent::AckReceived { .. } => {
                // A non-2xx ACK is owned hop-by-hop by the upstream INVITE
                // server transaction. The downstream INVITE client
                // transaction has already generated its own ACK on the exact
                // branch route, so forwarding this observation would create a
                // duplicate ACK.
                trace!("proxy: consumed transaction-owned non-2xx ACK observation");
            }
            TransactionEvent::AckRequest { request, .. }
            | TransactionEvent::StrayAckRequest { request, .. } => {
                if let Err(e) = self.forward_stateless_request(request, None).await {
                    warn!("proxy: end-to-end ACK forwarding failed: {}", e);
                }
            }
            TransactionEvent::StrayResponse { .. } => {
                // The source-compatible public event is observational.
                // Stateful stateless-response handling requires the exact
                // transport-bound ingress delivered on the private channel.
                trace!("proxy: ignored observational StrayResponse");
            }
            TransactionEvent::TransportError { transaction_id } => {
                if self.generated_cancels.contains_key(&transaction_id) {
                    trace!("proxy: consumed transport failure for generated CANCEL");
                } else if let Err(e) = self
                    .aggregate_local_failure(
                        transaction_id,
                        StatusCode::ServiceUnavailable,
                        "Transport Error",
                    )
                    .await
                {
                    warn!("proxy: transport failure aggregation failed: {}", e);
                }
            }
            TransactionEvent::TransactionTimeout { transaction_id } => {
                if self.generated_cancels.contains_key(&transaction_id) {
                    trace!("proxy: consumed timeout for generated CANCEL");
                } else {
                    let is_non_invite = self
                        .forks_by_downstream
                        .get(&transaction_id)
                        .is_some_and(|fork| !fork.is_invite);
                    let result = if is_non_invite {
                        self.aggregate_non_invite_timeout(transaction_id).await
                    } else {
                        self.aggregate_local_failure(
                            transaction_id,
                            StatusCode::RequestTimeout,
                            "Request Timeout",
                        )
                        .await
                    };
                    if let Err(e) = result {
                        warn!("proxy: transaction timeout aggregation failed: {}", e);
                    }
                }
            }
            TransactionEvent::TransactionTerminated { transaction_id } => {
                self.cleanup_fork(&transaction_id).await;
            }
            _ => {
                trace!("proxy: ignoring event {:?}", event);
            }
        }
    }

    async fn respond_to_request_rejection(
        &self,
        upstream_tx_id: &TransactionKey,
        original_request: &Request,
        rejection: &RequestRejection,
    ) -> Result<(), ProxyError> {
        let status = match rejection {
            RequestRejection::UnsupportedUriScheme => StatusCode::UnsupportedUriScheme,
            RequestRejection::UnsupportedProxyRequire(_) => StatusCode::BadExtension,
            RequestRejection::MalformedProxyRequire | RequestRejection::MalformedRoute => {
                StatusCode::BadRequest
            }
        };
        let mut response = crate::local_response::local_response_from_request(
            original_request,
            upstream_tx_id,
            status,
            None,
        );
        if let RequestRejection::UnsupportedProxyRequire(tags) = rejection {
            response
                .headers
                .push(routing::unsupported_header(tags.clone()));
        }
        self.tm
            .send_response(upstream_tx_id, response)
            .await
            .map_err(|error| ProxyError::Transaction(error.to_string()))
    }

    fn preprocess_inbound_maddr(&self, upstream_tx_id: &TransactionKey, request: &mut Request) {
        let Some(route) = self.tm.server_transaction_response_route(upstream_tx_id) else {
            return;
        };
        let Some(transport) = route.transport_type else {
            return;
        };
        let Some(local_addr) = self
            .tm
            .get_transport_info(transport)
            .and_then(|info| info.local_addr)
        else {
            return;
        };
        let _ =
            routing::preprocess_local_maddr(request, &self.options.routing, local_addr, transport);
    }

    fn select_route(&self, request: &Request) -> Option<SelectedRouteDecision> {
        match &self.route_selector {
            RouteSelector::Socket(route_fn) => route_fn(request).map(SelectedRouteDecision::Socket),
            RouteSelector::Uri(route_fn) => route_fn(request).map(SelectedRouteDecision::Uri),
            RouteSelector::UriWithSocketFallback { uri, socket } => uri(request)
                .map(SelectedRouteDecision::Uri)
                .or_else(|| socket(request).map(SelectedRouteDecision::Socket)),
        }
    }

    fn try_reserve_downstream_slots(&self, additional: usize) -> bool {
        if additional == 0 {
            return true;
        }
        self.downstream_slot_reservations
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current
                    .checked_add(additional)
                    .filter(|next| *next <= self.options.downstream_transaction_capacity)
            })
            .is_ok()
    }

    fn release_downstream_slots(&self, released: usize) {
        if released == 0 {
            return;
        }
        let result = self.downstream_slot_reservations.fetch_update(
            Ordering::AcqRel,
            Ordering::Acquire,
            |current| current.checked_sub(released),
        );
        debug_assert!(
            result.is_ok(),
            "released more proxy downstream capacity than was reserved"
        );
    }

    fn try_extend_fork_reservation(&self, fork: &ForkContext, additional: usize) -> bool {
        if additional == 0 {
            return true;
        }
        loop {
            let current = fork.reserved_downstream_slots.load(Ordering::Acquire);
            let Some(next) = current.checked_add(additional) else {
                return false;
            };
            if next > self.options.branches_per_response_context {
                return false;
            }
            if fork
                .reserved_downstream_slots
                .compare_exchange(current, next, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                continue;
            }
            if self.try_reserve_downstream_slots(additional) {
                return true;
            }
            let previous = fork
                .reserved_downstream_slots
                .fetch_sub(additional, Ordering::AcqRel);
            debug_assert!(previous >= additional);
            return false;
        }
    }

    async fn reject_overload(
        &self,
        upstream_tx_id: &TransactionKey,
        original_request: &Request,
        reason: &'static str,
    ) -> Result<(), ProxyError> {
        warn!(
            upstream_transaction = %upstream_tx_id,
            reason,
            "proxy: rejecting new response context with 503 at capacity"
        );
        self.respond_locally(
            upstream_tx_id,
            original_request,
            StatusCode::ServiceUnavailable,
        )
        .await
    }

    async fn prepare_leg_plans(
        &self,
        request: &Request,
        decision: &SelectedRouteDecision,
    ) -> Vec<LegPlan> {
        let mut plans = Vec::new();

        if let SelectedRouteDecision::Uri(decision) = decision {
            for target in &decision.targets {
                plans.push(self.prepare_uri_leg(request, target).await);
            }
            return plans;
        }

        let SelectedRouteDecision::Socket(decision) = decision else {
            unreachable!("URI decision returned above");
        };
        let legacy_legs: Vec<Vec<SocketAddr>> = if decision.leg_candidates.is_empty() {
            decision
                .targets
                .iter()
                .copied()
                .map(|target| vec![target])
                .collect()
        } else {
            decision.leg_candidates.clone()
        };
        if !legacy_legs.is_empty() {
            let target = ProxyTarget::new(request.uri().clone());
            match routing::prepare_target(request, &target, &self.options.routing) {
                Ok(prepared) => {
                    let transport = rvoip_sip_transport::resolver::select_transport_for_uri(
                        &prepared.next_hop_uri,
                    );
                    let authority = authority_for_uri(&prepared.next_hop_uri);
                    for candidates in legacy_legs {
                        let routes = match &authority {
                            Ok(authority) => candidates
                                .iter()
                                .copied()
                                .map(|destination| {
                                    TransportRoute::new(destination)
                                        .with_transport_type(transport)
                                        .with_authority(authority.clone())
                                })
                                .collect(),
                            Err(_) => Vec::new(),
                        };
                        plans.push(LegPlan {
                            request: prepared.request.clone(),
                            candidates: routes,
                            label: candidates
                                .first()
                                .map(ToString::to_string)
                                .unwrap_or_else(|| "legacy-empty".into()),
                            preparation_failure: authority
                                .as_ref()
                                .err()
                                .map(ToString::to_string)
                                .or_else(|| {
                                    candidates
                                        .is_empty()
                                        .then(|| "legacy leg has no candidates".into())
                                }),
                        });
                    }
                }
                Err(error) => {
                    for candidates in legacy_legs {
                        plans.push(LegPlan {
                            request: request.clone(),
                            candidates: Vec::new(),
                            label: candidates
                                .first()
                                .map(ToString::to_string)
                                .unwrap_or_else(|| "legacy-invalid".into()),
                            preparation_failure: Some(format!(
                                "legacy target preparation failed: {error:?}"
                            )),
                        });
                    }
                }
            }
        }
        plans
    }

    async fn prepare_uri_leg(&self, request: &Request, target: &ProxyTarget) -> LegPlan {
        let label = target.uri.to_string();
        let prepared = match routing::prepare_target(request, target, &self.options.routing) {
            Ok(prepared) => prepared,
            Err(error) => {
                return LegPlan {
                    request: request.clone(),
                    candidates: Vec::new(),
                    label,
                    preparation_failure: Some(format!("URI target preparation failed: {error:?}")),
                };
            }
        };

        match self.options.resolver.resolve(&prepared.next_hop_uri).await {
            Ok(candidates) => {
                let secure = matches!(prepared.next_hop_uri.scheme(), Scheme::Sips)
                    || matches!(prepared.request.uri().scheme(), Scheme::Sips);
                let routes: Vec<_> = candidates
                    .into_iter()
                    .filter(|candidate| {
                        !secure
                            || matches!(
                                candidate.transport,
                                TransportType::Tls | TransportType::Wss
                            )
                    })
                    .filter_map(|candidate| {
                        resolved_target_route(candidate, &prepared.next_hop_uri).ok()
                    })
                    .collect();
                let failure = routes.is_empty().then(|| {
                    if secure {
                        "RFC 3263 produced no secure SIPS candidates".into()
                    } else {
                        "RFC 3263 produced no candidates".into()
                    }
                });
                LegPlan {
                    request: prepared.request,
                    candidates: routes,
                    label,
                    preparation_failure: failure,
                }
            }
            Err(error) => LegPlan {
                request: prepared.request,
                candidates: Vec::new(),
                label,
                preparation_failure: Some(format!("RFC 3263 resolution failed: {error}")),
            },
        }
    }

    fn via_sent_by(&self, transport: TransportType) -> Result<SocketAddr, ProxyError> {
        if let Some(advertised) = self.options.advertised_via.get(&transport) {
            return Ok(*advertised);
        }
        self.tm
            .get_transport_info(transport)
            .and_then(|info| info.local_addr)
            .ok_or_else(|| {
                ProxyError::Transport(format!(
                    "selected {transport} transport has no concrete local Via address"
                ))
            })
    }

    async fn prepare_exact_outbound_route(
        &self,
        request: &mut Request,
        mut route: TransportRoute,
        branch: &str,
    ) -> Result<TransportRoute, ProxyError> {
        let mut transport = route.transport_type.ok_or_else(|| {
            ProxyError::Transport("resolver candidate omitted transport type".into())
        })?;
        if routing::request_requires_secure_routing(request).map_err(request_rejection_error)?
            && !matches!(transport, TransportType::Tls | TransportType::Wss)
        {
            return Err(ProxyError::Transport(
                "SIPS target cannot use a plaintext transport".into(),
            ));
        }

        let mut sent_by = self.via_sent_by(transport)?;
        push_proxy_via(request, sent_by, transport, branch)?;

        if transport == TransportType::Udp
            && rvoip_sip_core::Message::Request(request.clone())
                .to_bytes()
                .len()
                > self.tm.transport().max_safe_message_size()
        {
            if !self.tm.transport().supports_tcp() {
                return Err(ProxyError::Transport(
                    "request exceeds safe UDP size and TCP is unavailable".into(),
                ));
            }
            transport = TransportType::Tcp;
            route.transport_type = Some(transport);
            sent_by = self.via_sent_by(transport)?;
            remove_top_request_via(request);
            push_proxy_via(request, sent_by, transport, branch)?;
        }

        let message = rvoip_sip_core::Message::Request(request.clone());
        let prepared = self
            .tm
            .transport()
            .prepare_message_route(&message, route)
            .await
            .map_err(|error| ProxyError::Transport(error.to_string()))?;
        if prepared.transport_type != Some(transport) {
            return Err(ProxyError::Transport(
                "transport preparation changed the selected transport".into(),
            ));
        }
        Ok(prepared)
    }

    async fn handle_inbound_request(
        self: Arc<Self>,
        upstream_tx_id: TransactionKey,
        request: Request,
        _source: SocketAddr,
        is_invite: bool,
    ) -> Result<(), ProxyError> {
        let original_request = request.clone();
        let mut request = request;

        if let Err(rejection) = routing::validate_request(&request, &self.options.routing) {
            self.respond_to_request_rejection(&upstream_tx_id, &original_request, &rejection)
                .await?;
            return Ok(());
        }
        if let Err(rejection) =
            routing::preprocess_inbound_route(&mut request, &self.options.routing)
        {
            self.respond_to_request_rejection(&upstream_tx_id, &original_request, &rejection)
                .await?;
            return Ok(());
        }
        self.preprocess_inbound_maddr(&upstream_tx_id, &mut request);

        // RFC 3261 §16.6 step 4 — loop detection. If any branch in
        // the inbound Via stack matches a branch this proxy has
        // previously stamped, the request has looped back through us
        // and we MUST reject with 482 (Loop Detected).
        if self.options.legacy_branch_loop_detection {
            if let Some(looped_branch) = self.find_known_branch_in_request(&request) {
                warn!(
                    "proxy: loop detected — inbound Via carries our previously-stamped branch {}; sending 482",
                    looped_branch
                );
                self.respond_locally(&upstream_tx_id, &original_request, StatusCode::LoopDetected)
                    .await?;
                return Err(ProxyError::LoopDetected);
            }
        }

        // RFC 3261 §16.6 step 3 — decrement Max-Forwards. If zero on
        // arrival, reject with 483 (too many hops) per §16.3 rule 6.
        if self.config.enforce_max_forwards {
            match self.decrement_max_forwards(&mut request) {
                Ok(()) => {}
                Err(ProxyError::MaxForwardsExhausted) => {
                    self.respond_locally(
                        &upstream_tx_id,
                        &original_request,
                        StatusCode::TooManyHops,
                    )
                    .await?;
                    return Err(ProxyError::MaxForwardsExhausted);
                }
                Err(e) => return Err(e),
            }
        }

        if self.forks_by_upstream.len() >= self.options.response_context_capacity {
            self.reject_overload(
                &upstream_tx_id,
                &original_request,
                "response-context capacity exhausted",
            )
            .await?;
            return Ok(());
        }

        // Routing decision from the application.
        let decision = match self.select_route(&request) {
            None => {
                self.respond_locally(&upstream_tx_id, &original_request, StatusCode::NotFound)
                    .await?;
                return Ok(());
            }
            Some(decision) if decision.is_empty() => {
                self.respond_locally(
                    &upstream_tx_id,
                    &original_request,
                    StatusCode::TemporarilyUnavailable,
                )
                .await?;
                return Ok(());
            }
            Some(decision) => decision,
        };
        if decision.leg_count() > self.options.branches_per_response_context {
            self.reject_overload(
                &upstream_tx_id,
                &original_request,
                "per-context downstream branch capacity exceeded",
            )
            .await?;
            return Ok(());
        }
        let leg_plans = self.prepare_leg_plans(&request, &decision).await;
        if leg_plans.len() > self.options.branches_per_response_context {
            self.reject_overload(
                &upstream_tx_id,
                &original_request,
                "per-context downstream branch capacity exceeded",
            )
            .await?;
            return Ok(());
        }
        let reserved_downstream_slots = leg_plans.len();
        if !self.try_reserve_downstream_slots(reserved_downstream_slots) {
            self.reject_overload(
                &upstream_tx_id,
                &original_request,
                "downstream transaction capacity exhausted",
            )
            .await?;
            return Ok(());
        }
        let attempted_redirect_targets = leg_plans
            .iter()
            .flat_map(|plan| plan.candidates.iter().map(|route| route.destination))
            .collect();

        // Build the fork context up-front so every leg's downstream
        // tx_id can look back to it via `forks_by_downstream`.
        let fork = Arc::new(ForkContext {
            upstream_server_tx: upstream_tx_id.clone(),
            is_invite,
            mode: decision.mode(),
            original_request: original_request.clone(),
            leg_plans,
            legs_started: AtomicUsize::new(0),
            startup_failures: AtomicUsize::new(0),
            legs: tokio::sync::Mutex::new(Vec::new()),
            upstream_final_dispatch: tokio::sync::Mutex::new(UpstreamFinalDispatch::Unsent),
            stop_new_branches: AtomicBool::new(false),
            upstream_cancelled: AtomicBool::new(false),
            upstream_terminated: AtomicBool::new(false),
            reserved_downstream_slots: AtomicUsize::new(reserved_downstream_slots),
            attempted_redirect_targets: tokio::sync::Mutex::new(attempted_redirect_targets),
            forwarding_request: request.clone(),
        });
        match self.forks_by_upstream.entry(upstream_tx_id.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(fork.clone());
            }
            Entry::Occupied(_) => {
                self.release_downstream_slots(reserved_downstream_slots);
                trace!(
                    upstream_transaction = %upstream_tx_id,
                    "proxy: ignored duplicate response-context admission"
                );
                return Ok(());
            }
        }

        let leg_count = fork.leg_plans.len();
        match decision.mode() {
            ForkMode::Parallel => {
                // Fire every leg in one batch. Each leg may carry a
                // candidate list (RFC 3263 §4.3) — start_leg walks
                // them internally on transport failure.
                let mut started = 0usize;
                for idx in 0..leg_count {
                    fork.legs_started.store(idx + 1, Ordering::Release);
                    let plan = &fork.leg_plans[idx];
                    match self.start_leg(&fork, plan).await {
                        Ok(()) => started += 1,
                        Err(error) => {
                            self.record_startup_failure(&fork, idx, plan, &error);
                        }
                    }
                }
                if started == 0 {
                    self.forward_best_failure(&fork).await?;
                }
            }
            ForkMode::Sequential => {
                // Start with the first leg only. Subsequent legs
                // are kicked off in `aggregate_failure`.
                if leg_count > 0 {
                    let started = self.start_next_sequential_leg(&fork, 0).await;
                    if !started {
                        self.forward_best_failure(&fork).await?;
                    }
                }
            }
        }

        Ok(())
    }

    fn record_startup_failure(
        &self,
        fork: &Arc<ForkContext>,
        leg_index: usize,
        plan: &LegPlan,
        error: &ProxyError,
    ) {
        fork.startup_failures.fetch_add(1, Ordering::AcqRel);
        warn!(
            "proxy: downstream leg {} ({}) exhausted before first write; recording branch-local 503 (candidates {:?}): {}",
            leg_index, plan.label, plan.candidates, error
        );
    }

    /// Start the first viable sequential logical leg at or after `start_idx`.
    ///
    /// A logical leg whose complete candidate set fails before first write is
    /// retained as a branch-local 503 and the next logical leg is attempted.
    /// Returning `false` means every remaining logical leg was exhausted.
    async fn start_next_sequential_leg(&self, fork: &Arc<ForkContext>, start_idx: usize) -> bool {
        let leg_total = fork.leg_plans.len();

        for idx in start_idx..leg_total {
            if fork.stop_new_branches.load(Ordering::Acquire) {
                return false;
            }
            let plan = &fork.leg_plans[idx];
            fork.legs_started.store(idx + 1, Ordering::Release);
            match self.start_leg(fork, plan).await {
                Ok(()) => return true,
                Err(error) => self.record_startup_failure(fork, idx, plan, &error),
            }
        }

        fork.legs_started.store(leg_total, Ordering::Release);
        false
    }

    /// Forward a request without creating a downstream client transaction.
    /// RFC 3261 §16.10 requires this for CANCEL when no matching response
    /// context exists; ACK for an INVITE 2xx is also end-to-end/stateless.
    async fn forward_stateless_request(
        &self,
        mut request: Request,
        upstream_route: Option<TransportRoute>,
    ) -> Result<(), ProxyError> {
        routing::validate_request(&request, &self.options.routing)
            .map_err(request_rejection_error)?;
        routing::preprocess_inbound_route(&mut request, &self.options.routing)
            .map_err(request_rejection_error)?;
        if self.config.enforce_max_forwards {
            self.decrement_max_forwards(&mut request)?;
        }
        let decision = self
            .select_route(&request)
            .ok_or_else(|| ProxyError::Transport("no stateless route decision".into()))?;
        let plans = self.prepare_leg_plans(&request, &decision).await;
        let Some(plan) = plans.first() else {
            return Err(ProxyError::Transport(
                "stateless route decision contained no targets".into(),
            ));
        };
        if plan.candidates.is_empty() {
            return Err(ProxyError::Transport(
                plan.preparation_failure
                    .clone()
                    .unwrap_or_else(|| "stateless target resolved to no candidates".into()),
            ));
        }

        let mut last_error = None;
        for candidate in &plan.candidates {
            let mut forwarded = plan.request.clone();
            let transport = candidate.transport_type.ok_or_else(|| {
                ProxyError::Transport("stateless candidate has no selected transport".into())
            })?;
            let sent_by = self.via_sent_by(transport)?;
            let branch = stateless_proxy_branch(&forwarded, sent_by);
            let downstream_route = match self
                .prepare_exact_outbound_route(&mut forwarded, candidate.clone(), &branch)
                .await
            {
                Ok(route) => route,
                Err(error) => {
                    last_error = Some(error);
                    continue;
                }
            };
            let message = rvoip_sip_core::Message::Request(forwarded);

            let mut newly_registered_deadline = None;
            if let Some(upstream_route) = upstream_route.clone() {
                let request = match &message {
                    rvoip_sip_core::Message::Request(request) => request,
                    rvoip_sip_core::Message::Response(_) => {
                        unreachable!("request constructed above")
                    }
                };
                let cseq = request.cseq().ok_or_else(|| {
                    ProxyError::Transport(
                        "stateless request requiring a response route has no CSeq".into(),
                    )
                })?;
                let retained = StatelessResponseRoute {
                    upstream_route,
                    downstream_route: downstream_route.clone(),
                    cseq_sequence: cseq.sequence(),
                    cseq_method: cseq.method().clone(),
                    expires_at: Instant::now() + RESPONSE_CONTEXT_RETENTION,
                };
                if self.register_stateless_response_route(branch.clone(), retained.clone())? {
                    newly_registered_deadline = Some(retained.expires_at);
                }
            }
            self.retain_known_branch(branch.clone());

            let result = self
                .tm
                .transport()
                .send_message_via(message, downstream_route)
                .await
                .map_err(|error| ProxyError::Transport(error.to_string()));
            if let Err(error) = result {
                if let Some(deadline) = newly_registered_deadline {
                    self.remove_stateless_response_route(&branch, Some(deadline));
                } else if !self.stateless_response_routes.contains_key(&branch) {
                    self.known_branches.remove(&branch);
                }
                last_error = Some(error);
                continue;
            }
            if newly_registered_deadline.is_none()
                && !self.stateless_response_routes.contains_key(&branch)
            {
                // ACK has no response path. Do not leave its deterministic
                // branch in the loop-detection set indefinitely.
                self.known_branches.remove(&branch);
            }
            return Ok(());
        }

        Err(last_error.unwrap_or_else(|| {
            ProxyError::Transport("all stateless RFC 3263 candidates failed".into())
        }))
    }

    async fn forward_stateless_response(
        &self,
        mut response: Response,
        source: SocketAddr,
        response_route: Option<&TransportRoute>,
    ) -> Result<(), ProxyError> {
        let Some(branch) = response
            .first_via()
            .and_then(|via| via.branch().map(str::to_owned))
        else {
            return Ok(());
        };
        let Some(retained_ref) = self.stateless_response_routes.get(&branch) else {
            return Ok(());
        };
        let retained = retained_ref.clone();
        drop(retained_ref);
        if retained.expires_at <= Instant::now() {
            self.remove_stateless_response_route(&branch, Some(retained.expires_at));
            return Ok(());
        }
        let Some(actual_route) = response_route else {
            trace!(
                branch = %branch,
                "proxy: dropped stateless response without authenticated ingress route"
            );
            return Ok(());
        };
        if source != actual_route.destination
            || !stateless_response_route_matches(&retained.downstream_route, actual_route)
        {
            trace!(
                branch = %branch,
                source = %source,
                "proxy: dropped stateless response from mismatched route"
            );
            return Ok(());
        }
        let Some(cseq) = response.cseq() else {
            trace!(branch = %branch, "proxy: dropped stateless response without CSeq");
            return Ok(());
        };
        if cseq.sequence() != retained.cseq_sequence || cseq.method() != &retained.cseq_method {
            trace!(
                branch = %branch,
                "proxy: dropped stateless response with mismatched CSeq"
            );
            return Ok(());
        }
        remove_top_via_header(&mut response);
        self.tm
            .transport()
            .send_message_via(
                rvoip_sip_core::Message::Response(response),
                retained.upstream_route,
            )
            .await
            .map_err(|error| ProxyError::Transport(error.to_string()))
    }

    async fn aggregate_local_failure(
        self: &Arc<Self>,
        downstream_tx: TransactionKey,
        status: StatusCode,
        reason: &'static str,
    ) -> Result<(), ProxyError> {
        let Some(fork_ref) = self.forks_by_downstream.get(&downstream_tx) else {
            return Ok(());
        };
        let fork = fork_ref.clone();
        drop(fork_ref);
        if self
            .candidate_attempt_is_inactive(&fork, &downstream_tx)
            .await
        {
            return Ok(());
        }
        if matches!(
            status,
            StatusCode::ServiceUnavailable | StatusCode::RequestTimeout
        ) && self
            .try_advance_candidate(&fork, &downstream_tx, Some(status))
            .await
        {
            return Ok(());
        }
        let request = {
            let legs = fork.legs.lock().await;
            legs.iter()
                .find(|leg| leg.downstream_client_tx == downstream_tx)
                .map(|leg| leg.forwarded_request.clone())
        };
        let Some(request) = request else {
            return Ok(());
        };
        let response = rvoip_sip_core::builder::SimpleResponseBuilder::response_from_request(
            &request,
            status,
            Some(reason),
        )
        .build();
        self.aggregate_failure(&fork, downstream_tx, response).await
    }

    /// Complete a timed-out non-INVITE branch without manufacturing a 408.
    ///
    /// RFC 4320 updates RFC 3261 §16.8: a stateful proxy does not generate a
    /// 408 when a non-INVITE client transaction times out. Other fork branches
    /// are still allowed to produce the selected final response. If every
    /// branch times out silently, the upstream server transaction is
    /// terminated without a final response.
    async fn aggregate_non_invite_timeout(
        self: &Arc<Self>,
        downstream_tx: TransactionKey,
    ) -> Result<(), ProxyError> {
        let Some(fork_ref) = self.forks_by_downstream.get(&downstream_tx) else {
            return Ok(());
        };
        let fork = fork_ref.clone();
        drop(fork_ref);
        if self
            .candidate_attempt_is_inactive(&fork, &downstream_tx)
            .await
        {
            return Ok(());
        }
        if fork.is_invite {
            return self
                .aggregate_local_failure(
                    downstream_tx,
                    StatusCode::RequestTimeout,
                    "Request Timeout",
                )
                .await;
        }
        if self
            .try_advance_candidate(&fork, &downstream_tx, None)
            .await
        {
            return Ok(());
        }

        let (newly_completed, all_finished) = {
            let mut legs = fork.legs.lock().await;
            let Some(leg) = legs
                .iter_mut()
                .find(|leg| leg.downstream_client_tx == downstream_tx)
            else {
                return Ok(());
            };
            if matches!(leg.state, LegState::Completed | LegState::Terminated) {
                (false, legs.iter().all(Leg::is_finished))
            } else {
                leg.state = LegState::Completed;
                leg.final_status = None;
                leg.last_response = None;
                (true, legs.iter().all(Leg::is_finished))
            }
        };
        if !newly_completed {
            return if all_finished {
                self.finish_non_invite_timeout_set(&fork).await
            } else {
                Ok(())
            };
        }

        match fork.mode {
            ForkMode::Sequential if !fork.stop_new_branches.load(Ordering::Acquire) => {
                let next_idx = fork.legs_started.load(Ordering::Acquire);
                if self.start_next_sequential_leg(&fork, next_idx).await {
                    return Ok(());
                }
                self.finish_non_invite_timeout_set(&fork).await
            }
            ForkMode::Sequential => {
                if all_finished {
                    self.finish_non_invite_timeout_set(&fork).await
                } else {
                    Ok(())
                }
            }
            ForkMode::Parallel => {
                if all_finished {
                    self.finish_non_invite_timeout_set(&fork).await
                } else {
                    Ok(())
                }
            }
        }
    }

    async fn finish_non_invite_timeout_set(
        self: &Arc<Self>,
        fork: &Arc<ForkContext>,
    ) -> Result<(), ProxyError> {
        let has_selectable_failure = fork.startup_failures.load(Ordering::Acquire) > 0
            || fork
                .legs
                .lock()
                .await
                .iter()
                .any(|leg| leg.last_response.is_some());
        if has_selectable_failure {
            self.forward_best_failure(fork).await
        } else {
            // RFC 4320 requires silence when no branch returned a final.
            // Explicitly end the upstream server transaction so the retained
            // response context can drain rather than leaving a Trying
            // transaction alive indefinitely.
            match self
                .tm
                .terminate_transaction(&fork.upstream_server_tx)
                .await
            {
                Ok(()) => Ok(()),
                Err(error) if fork.upstream_terminated.load(Ordering::Acquire) => {
                    trace!(
                        upstream_transaction = %fork.upstream_server_tx,
                        %error,
                        "proxy: all-timeout non-INVITE upstream was already terminated"
                    );
                    Ok(())
                }
                Err(error) => Err(ProxyError::Transaction(error.to_string())),
            }
        }
    }

    /// Push a fresh transport-exact proxy Via onto a target-specific request,
    /// build a downstream client transaction on one RFC 3263 candidate route,
    /// register the leg with the fork context, and send the request.
    ///
    /// When `candidates.len() > 1`, the method walks the list in order
    /// on transport-level send failures (RFC 3263 §4.3 multi-candidate
    /// failover at the leg level). Each retry stamps a fresh proxy
    /// branch so the Via stack stays §16.6-valid across attempts.
    /// Returns the first successful send; otherwise the last error.
    async fn start_leg(&self, fork: &Arc<ForkContext>, plan: &LegPlan) -> Result<(), ProxyError> {
        self.start_leg_candidates(
            fork,
            &plan.request,
            &plan.candidates,
            0,
            plan.candidates.len(),
            &plan.label,
            plan.preparation_failure.as_deref(),
        )
        .await
    }

    async fn start_leg_candidates(
        &self,
        fork: &Arc<ForkContext>,
        request: &Request,
        candidates: &[TransportRoute],
        completed_attempts: usize,
        total_attempts: usize,
        label: &str,
        preparation_failure: Option<&str>,
    ) -> Result<(), ProxyError> {
        if fork.stop_new_branches.load(Ordering::Acquire) {
            return Ok(());
        }
        if candidates.is_empty() {
            return Err(ProxyError::Transport(
                preparation_failure
                    .unwrap_or("start_leg called with no candidates")
                    .to_owned(),
            ));
        }

        let mut last_err: Option<ProxyError> = None;

        for (idx, candidate) in candidates.iter().enumerate() {
            let attempt = completed_attempts + idx + 1;
            // Each attempt gets a fresh request clone + fresh proxy
            // Via with a unique branch so RFC 3261 §16.6 branch
            // uniqueness holds across the candidate walk.
            let mut leg_request = request.clone();
            let proxy_branch = format!("z9hG4bK-proxy-{}", uuid::Uuid::new_v4().simple());
            let exact_route = match self
                .prepare_exact_outbound_route(&mut leg_request, candidate.clone(), &proxy_branch)
                .await
            {
                Ok(route) => route,
                Err(error) => {
                    last_err = Some(error);
                    continue;
                }
            };
            let destination = exact_route.destination;
            let downstream_tx_id = match self
                .tm
                .create_client_transaction_on_route(leg_request.clone(), exact_route)
                .await
            {
                Ok(id) => id,
                Err(e) => {
                    last_err = Some(ProxyError::Transaction(format!(
                        "RFC 3263 §4.3 leg candidate {}/{} ({}): create_client_transaction: {}",
                        attempt, total_attempts, destination, e
                    )));
                    continue;
                }
            };

            // Register the leg before sending so a fast inbound
            // response can find the fork context via
            // `forks_by_downstream`.
            {
                let mut legs = fork.legs.lock().await;
                legs.push(Leg {
                    downstream_client_tx: downstream_tx_id.clone(),
                    final_status: None,
                    state: LegState::Calling,
                    cancel_requested: false,
                    cancel_sent: false,
                    cancel_retry_delay: None,
                    generated_cancel: None,
                    last_response: None,
                    candidate_continuation: (idx + 1 < candidates.len()).then(|| {
                        CandidateContinuation {
                            request: request.clone(),
                            candidates: candidates[idx + 1..].to_vec(),
                            completed_attempts: attempt,
                            total_attempts,
                            label: label.to_owned(),
                        }
                    }),
                    superseded_by_candidate: false,
                    candidate_advancement_in_progress: false,
                    proxy_branch: proxy_branch.clone(),
                    forwarded_request: leg_request,
                });
            }
            self.forks_by_downstream
                .insert(downstream_tx_id.clone(), fork.clone());
            self.retain_known_branch(proxy_branch.clone());

            match self.tm.send_request(&downstream_tx_id).await {
                Ok(()) => {
                    if attempt > 1 {
                        debug!(
                            "proxy: leg candidate {}/{} ({}) succeeded after {} prior failure(s)",
                            attempt,
                            total_attempts,
                            destination,
                            attempt.saturating_sub(1)
                        );
                    }
                    debug!(
                        "proxy: started leg to {} (upstream tx={} downstream tx={} mode={:?})",
                        destination, fork.upstream_server_tx, downstream_tx_id, fork.mode
                    );
                    if fork.is_invite {
                        self.schedule_timer_c(downstream_tx_id);
                    }
                    return Ok(());
                }
                Err(e) => {
                    // Treat any send_request failure as a recoverable
                    // transport-level error for §4.3 purposes —
                    // transaction-core wraps the underlying transport
                    // err as a string, so we can't distinguish
                    // recoverable from non-recoverable here. The risk
                    // of over-retrying is low: each attempt is bounded
                    // by Timer C and the candidate list is short.
                    debug!(
                        "proxy: leg candidate {}/{} ({}) failed: {}; trying next",
                        attempt, total_attempts, destination, e
                    );
                    // Drop the leg from the fork map — the next
                    // candidate will register its own.
                    self.forks_by_downstream.remove(&downstream_tx_id);
                    {
                        let mut legs = fork.legs.lock().await;
                        legs.retain(|leg| leg.downstream_client_tx != downstream_tx_id);
                    }
                    self.known_branches.remove(&proxy_branch);
                    last_err = Some(ProxyError::Transaction(format!(
                        "leg candidate {} ({}): {}",
                        attempt, destination, e
                    )));
                    continue;
                }
            }
        }

        Err(last_err.unwrap_or_else(|| {
            ProxyError::Transport(format!(
                "RFC 3263 §4.3 leg failover exhausted after candidate {}/{} ({})",
                completed_attempts + candidates.len(),
                total_attempts,
                label
            ))
        }))
    }

    async fn candidate_attempt_is_inactive(
        &self,
        fork: &Arc<ForkContext>,
        downstream_tx: &TransactionKey,
    ) -> bool {
        fork.legs.lock().await.iter().any(|leg| {
            leg.downstream_client_tx == *downstream_tx
                && (leg.superseded_by_candidate || leg.candidate_advancement_in_progress)
        })
    }

    /// Advance one logical RFC 3263 branch after a retryable candidate
    /// terminal condition.
    ///
    /// The continuation is taken before any await, so duplicate 503/timeout
    /// observations cannot start two candidates. A replacement candidate gets
    /// a new client transaction and fresh Via branch. If every remaining
    /// candidate fails before its first write, the caller retains and
    /// aggregates the original branch-local failure.
    async fn try_advance_candidate(
        self: &Arc<Self>,
        fork: &Arc<ForkContext>,
        downstream_tx: &TransactionKey,
        failure_status: Option<StatusCode>,
    ) -> bool {
        if fork.stop_new_branches.load(Ordering::Acquire) {
            return false;
        }

        let (
            continuation,
            previous_state,
            previous_final_status,
            previous_last_response,
            previous_cancel_retry_delay,
        ) = {
            let mut legs = fork.legs.lock().await;
            let Some(leg) = legs
                .iter_mut()
                .find(|leg| leg.downstream_client_tx == *downstream_tx)
            else {
                return false;
            };
            // A duplicate retryable failure is already owned. Treat it as
            // handled so it cannot fall through and aggregate the old branch
            // while the owner is awaiting candidate startup.
            if leg.superseded_by_candidate || leg.candidate_advancement_in_progress {
                return true;
            }
            if leg.is_finished() || leg.cancel_requested || leg.cancel_sent {
                return false;
            }
            let Some(continuation) = leg.candidate_continuation.take() else {
                return false;
            };
            let previous_state = leg.state;
            let previous_final_status = leg.final_status;
            let previous_last_response = leg.last_response.take();
            let previous_cancel_retry_delay = leg.cancel_retry_delay.take();
            leg.candidate_advancement_in_progress = true;
            leg.state = LegState::Completed;
            leg.final_status = failure_status;
            (
                continuation,
                previous_state,
                previous_final_status,
                previous_last_response,
                previous_cancel_retry_delay,
            )
        };

        self.cancel_timer_c(downstream_tx);
        self.cancel_cancel_retry(downstream_tx);
        let started = self
            .start_leg_candidates(
                fork,
                &continuation.request,
                &continuation.candidates,
                continuation.completed_attempts,
                continuation.total_attempts,
                &continuation.label,
                None,
            )
            .await;

        match started {
            Ok(()) => {
                let mut legs = fork.legs.lock().await;
                let Some(previous) = legs
                    .iter_mut()
                    .find(|leg| leg.downstream_client_tx == *downstream_tx)
                else {
                    warn!(
                        downstream_transaction = %downstream_tx,
                        "proxy: candidate replacement started but prior attempt disappeared"
                    );
                    return true;
                };
                previous.candidate_advancement_in_progress = false;
                previous.superseded_by_candidate = true;
                debug!(
                    downstream_transaction = %downstream_tx,
                    completed_attempts = continuation.completed_attempts,
                    total_attempts = continuation.total_attempts,
                    "proxy: advanced logical RFC 3263 branch to its next candidate"
                );
                true
            }
            Err(error) => {
                let mut legs = fork.legs.lock().await;
                if let Some(previous) = legs
                    .iter_mut()
                    .find(|leg| leg.downstream_client_tx == *downstream_tx)
                {
                    previous.state = previous_state;
                    previous.final_status = previous_final_status;
                    previous.last_response = previous_last_response;
                    previous.cancel_retry_delay = previous_cancel_retry_delay;
                    previous.candidate_advancement_in_progress = false;
                }
                drop(legs);
                debug!(
                    downstream_transaction = %downstream_tx,
                    completed_attempts = continuation.completed_attempts,
                    total_attempts = continuation.total_attempts,
                    %error,
                    "proxy: remaining RFC 3263 candidates exhausted; aggregating branch-local failure"
                );
                false
            }
        }
    }

    /// Aggregator entry for every downstream response. Routes via
    /// RFC 3261 §16.7 step-by-step:
    ///
    /// - 100 → consume locally. For INVITE, 101-199 is forwarded upstream and
    ///   marks the branch Proceeding. RFC 4320 requires every non-INVITE
    ///   provisional response to be consumed locally.
    /// - 2xx → forward every response upstream and request cancellation
    ///   of other live legs.
    /// - 3xx-6xx → record on the leg. Sequential mode advances to the
    ///   next target. Parallel mode waits until every leg has a final
    ///   then picks the best response (§16.7 step 6).
    async fn aggregate_response(
        self: &Arc<Self>,
        downstream_tx_id: TransactionKey,
        response: Response,
        is_final: bool,
    ) -> Result<(), ProxyError> {
        let Some(fork_ref) = self.forks_by_downstream.get(&downstream_tx_id) else {
            return Ok(());
        };
        let fork = fork_ref.clone();
        drop(fork_ref);

        if self
            .candidate_attempt_is_inactive(&fork, &downstream_tx_id)
            .await
        {
            trace!(
                downstream_transaction = %downstream_tx_id,
                "proxy: dropped response from superseded RFC 3263 candidate"
            );
            return Ok(());
        }

        if !fork.is_invite {
            let silently_completed = fork.legs.lock().await.iter().any(|leg| {
                leg.downstream_client_tx == downstream_tx_id
                    && leg.is_finished()
                    && leg.last_response.is_none()
            });
            if silently_completed {
                trace!(
                    downstream_transaction = %downstream_tx_id,
                    "proxy: dropped late non-INVITE response after branch timeout"
                );
                return Ok(());
            }
        }

        let status = response.status();

        // RFC 3263 §4.3 treats a 503 from one resolved candidate as a
        // candidate-local failure. Advance within this logical branch before
        // ordinary fork aggregation. A received 408 intentionally does not
        // enter this path.
        if is_final
            && status == StatusCode::ServiceUnavailable
            && self
                .try_advance_candidate(&fork, &downstream_tx_id, Some(status))
                .await
        {
            return Ok(());
        }

        if !is_final {
            let suppress_upstream = status.as_u16() == 100 || !fork.is_invite;

            let dispatch_latched_cancel = {
                let mut legs = fork.legs.lock().await;
                let Some(leg) = legs
                    .iter_mut()
                    .find(|leg| leg.downstream_client_tx == downstream_tx_id)
                else {
                    return Ok(());
                };
                if leg.state == LegState::Calling {
                    leg.state = LegState::Proceeding;
                }
                let dispatch = leg.cancel_requested && !leg.cancel_sent;
                if dispatch {
                    leg.cancel_sent = true;
                }
                dispatch
            };

            // 100 establishes Proceeding (and therefore releases a pending
            // CANCEL), but RFC 3261 §16.8 resets Timer C only on 101-199.
            if fork.is_invite && !suppress_upstream {
                self.schedule_timer_c(downstream_tx_id.clone());
            }
            if dispatch_latched_cancel {
                self.send_cancel(&fork, &downstream_tx_id).await;
            }
            if suppress_upstream || self.upstream_final_is_terminal(&fork).await {
                return Ok(());
            }
            let _ = self
                .forward_provisional_to_upstream(&fork, response)
                .await?;
            return Ok(());
        }

        let class = status.as_u16() / 100;
        if class == 2 {
            return self
                .aggregate_success(&fork, downstream_tx_id, response)
                .await;
        }

        // 3xx / 4xx / 5xx / 6xx
        self.aggregate_failure(&fork, downstream_tx_id, response)
            .await
    }

    async fn aggregate_success(
        self: &Arc<Self>,
        fork: &Arc<ForkContext>,
        downstream_tx_id: TransactionKey,
        response: Response,
    ) -> Result<(), ProxyError> {
        self.cancel_timer_c(&downstream_tx_id);
        self.cancel_cancel_retry(&downstream_tx_id);

        // Mark this branch complete.
        {
            let mut legs = fork.legs.lock().await;
            for leg in legs.iter_mut() {
                if leg.downstream_client_tx == downstream_tx_id {
                    leg.final_status = Some(response.status());
                    leg.last_response = Some(response.clone());
                    leg.state = LegState::Completed;
                    leg.cancel_retry_delay = None;
                    break;
                }
            }
        }

        // Every INVITE 2xx is forwarded, including forked and late 2xx.
        // A non-INVITE server transaction can forward only its selected final.
        // Ownership is committed only after an authoritative successful write.
        let outcome = self
            .dispatch_upstream_final(fork, response, true, fork.is_invite)
            .await?;

        if fork.is_invite && outcome == FinalDispatchOutcome::FirstSent {
            fork.stop_new_branches.store(true, Ordering::Release);
            self.request_cancel_siblings(fork, Some(&downstream_tx_id))
                .await;
        }
        Ok(())
    }

    async fn aggregate_failure(
        self: &Arc<Self>,
        fork: &Arc<ForkContext>,
        downstream_tx_id: TransactionKey,
        response: Response,
    ) -> Result<(), ProxyError> {
        // RFC 3261 §16.7 redirect — surface 3xx to subscribers and
        // optionally consult the installed [`RedirectInterceptor`].
        // If the interceptor returns `ReFork`, the 3xx does NOT
        // propagate upstream and fresh legs are spawned against the
        // app-supplied target set.
        let status = response.status();
        if status.as_u16() / 100 == 3 {
            let contacts = extract_contact_uris(&response);
            let _ = self.event_tx.send(ProxyEvent::RedirectReceived {
                upstream_tx: fork.upstream_server_tx.clone(),
                status,
                contacts: contacts.clone(),
            });

            if let Some(interceptor) = self.redirect_interceptor() {
                let info = RedirectInfo {
                    upstream_tx: fork.upstream_server_tx.clone(),
                    status,
                    contacts: contacts.clone(),
                };
                match interceptor.on_redirect(info).await {
                    Some(RedirectDecision::ReFork { mode, targets }) if !targets.is_empty() => {
                        if fork.stop_new_branches.load(Ordering::Acquire) {
                            return Ok(());
                        }

                        // The legacy public redirect surface identifies
                        // targets by SocketAddr. Preserve input order while
                        // removing duplicates both within this decision and
                        // across the complete response context.
                        let mut decision_targets = HashSet::new();
                        let attempted = fork.attempted_redirect_targets.lock().await;
                        let targets: Vec<_> = targets
                            .into_iter()
                            .filter(|target| {
                                decision_targets.insert(*target) && !attempted.contains(target)
                            })
                            .collect();
                        drop(attempted);

                        if targets.is_empty() {
                            debug!(
                                upstream_transaction = %fork.upstream_server_tx,
                                "proxy: redirect contained no new legacy socket targets; forwarding original redirect"
                            );
                            // Fall through so the response remains available
                            // for ordinary best-final aggregation.
                        } else if !self.try_extend_fork_reservation(fork, targets.len()) {
                            warn!(
                                upstream_transaction = %fork.upstream_server_tx,
                                requested_branches = targets.len(),
                                "proxy: redirect re-fork rejected at downstream capacity; forwarding original redirect"
                            );
                            // Capacity exhaustion must not consume the only
                            // final response or evict a live branch. Fall
                            // through to ordinary 3xx aggregation.
                        } else {
                            fork.attempted_redirect_targets
                                .lock()
                                .await
                                .extend(targets.iter().copied());
                            debug!(
                            "proxy: 3xx interceptor requested re-fork to {} target(s) in {:?} mode",
                            targets.len(),
                            mode
                        );
                            // Mark the leg as cancelled so the failure
                            // doesn't influence best-failure selection
                            // — the 3xx is "consumed" by the re-fork.
                            {
                                let mut legs = fork.legs.lock().await;
                                if let Some(leg) = legs
                                    .iter_mut()
                                    .find(|l| l.downstream_client_tx == downstream_tx_id)
                                {
                                    leg.cancel_requested = true;
                                    leg.cancel_sent = true;
                                    leg.cancel_retry_delay = None;
                                    leg.state = LegState::Completed;
                                    leg.final_status = Some(response.status());
                                    // The interceptor consumed this redirect.
                                    // Its attempted Contacts must not remain a
                                    // candidate for later best-final selection.
                                    leg.last_response = None;
                                }
                            }
                            let redirecting_leg_request = {
                                let legs = fork.legs.lock().await;
                                legs.iter()
                                    .find(|leg| leg.downstream_client_tx == downstream_tx_id)
                                    .map(|leg| leg.forwarded_request.clone())
                                    .unwrap_or_else(|| fork.forwarding_request.clone())
                            };
                            let redirect_request = routing::prepare_redirect_request(
                                &fork.forwarding_request,
                                &redirecting_leg_request,
                            );
                            // Spawn the requested legs. Treat each
                            // target as a single-candidate leg under
                            // the requested mode.
                            let mut started = 0usize;
                            for target in targets {
                                let decision =
                                    SelectedRouteDecision::Socket(RouteDecision::to(target));
                                let plans =
                                    self.prepare_leg_plans(&redirect_request, &decision).await;
                                let result = match plans.first() {
                                    Some(plan) => self.start_leg(fork, plan).await,
                                    None => Err(ProxyError::Transport(
                                        "redirect target produced no leg plan".into(),
                                    )),
                                };
                                match result {
                                    Ok(()) => started += 1,
                                    Err(e) => {
                                        fork.startup_failures.fetch_add(1, Ordering::AcqRel);
                                        warn!(
                                            "proxy: 3xx re-fork start_leg to {} failed: {}",
                                            target, e
                                        );
                                    }
                                }
                            }
                            if started == 0 {
                                self.forward_best_failure(fork).await?;
                            }
                            // Don't fall through to the failure path —
                            // the 3xx is now consumed.
                            return Ok(());
                        }
                    }
                    Some(RedirectDecision::Forward) | None => {
                        // Default: fall through to the normal
                        // failure-aggregation path, which forwards
                        // the 3xx via `forward_best_failure`.
                    }
                    Some(RedirectDecision::ReFork { .. }) => {
                        debug!(
                            "proxy: 3xx interceptor returned ReFork with no targets — forwarding upstream"
                        );
                    }
                }
            }
        }

        self.cancel_timer_c(&downstream_tx_id);
        self.cancel_cancel_retry(&downstream_tx_id);

        // Record this leg's final.
        let all_finished;
        let was_cancelled_leg;
        {
            let mut legs = fork.legs.lock().await;
            was_cancelled_leg = legs
                .iter()
                .any(|l| l.downstream_client_tx == downstream_tx_id && l.cancel_sent);
            for leg in legs.iter_mut() {
                if leg.downstream_client_tx == downstream_tx_id {
                    leg.final_status = Some(response.status());
                    leg.last_response = Some(response.clone());
                    leg.state = LegState::Completed;
                    leg.cancel_retry_delay = None;
                }
            }
            all_finished = legs.iter().all(Leg::is_finished);
        }

        if was_cancelled_leg && self.upstream_final_is_terminal(fork).await {
            // A 487 / similar on a leg we CANCELed — expected, don't
            // surface upstream.
            return Ok(());
        }

        // A 6xx is a global failure. Stop branch creation and cancel eligible
        // siblings, but retain the response context until they settle: a
        // racing 2xx still has precedence and must be forwarded.
        if status.as_u16() / 100 == 6 {
            fork.stop_new_branches.store(true, Ordering::Release);
            self.request_cancel_siblings(fork, Some(&downstream_tx_id))
                .await;
        }

        match fork.mode {
            ForkMode::Sequential => {
                if fork.stop_new_branches.load(Ordering::Acquire) {
                    return if all_finished {
                        self.forward_best_failure(fork).await
                    } else {
                        Ok(())
                    };
                }
                // `legs_started` is the next logical-leg index. Walk past
                // branch-local 503 startup failures until one leg reaches its
                // first transport write or the route is exhausted.
                let next_idx = fork.legs_started.load(Ordering::Acquire);
                if self.start_next_sequential_leg(fork, next_idx).await {
                    return Ok(());
                }
                // Exhausted — forward the best collected failure.
                self.forward_best_failure(fork).await
            }
            ForkMode::Parallel => {
                if all_finished {
                    self.forward_best_failure(fork).await
                } else {
                    Ok(())
                }
            }
        }
    }

    async fn forward_best_failure(
        self: &Arc<Self>,
        fork: &Arc<ForkContext>,
    ) -> Result<(), ProxyError> {
        // RFC 3261 §16.7 step 6. Prefer responses that contain useful
        // information for resubmission, then 3xx, remaining 4xx, and 5xx.
        // 6xx is handled immediately in aggregate_failure.
        let startup_failures = fork.startup_failures.load(Ordering::Acquire);
        let (best, only_service_unavailable, startup_503_is_best) = {
            let legs = fork.legs.lock().await;
            let responses = legs
                .iter()
                .filter_map(|leg| leg.last_response.clone())
                .collect::<Vec<_>>();
            let only_service_unavailable = (startup_failures > 0 || !responses.is_empty())
                && responses
                    .iter()
                    .all(|response| response.status() == StatusCode::ServiceUnavailable);
            let mut best = responses
                .into_iter()
                .min_by_key(|response| response_preference(response.status()));
            let startup_503_is_best = startup_failures > 0
                && best.as_ref().map_or(true, |response| {
                    response_preference(StatusCode::ServiceUnavailable)
                        < response_preference(response.status())
                });

            // RFC 3261 §16.7 requires combining authentication challenges
            // from every selected-class response.
            if let Some(selected) = best.as_mut() {
                aggregate_authenticate_headers(
                    selected,
                    legs.iter().filter_map(|leg| leg.last_response.as_ref()),
                );
            }
            (best, only_service_unavailable, startup_503_is_best)
        };

        // RFC 3261 §16.7 step 6 recommends replacing an aggregate 503 with a
        // locally generated 500. Build it from the upstream request so its Via
        // stack is correct; a downstream response would carry this proxy's Via
        // and take the normal pop-one path instead.
        if only_service_unavailable || startup_503_is_best {
            return self
                .forward_generated_failure(fork, StatusCode::ServerInternalError)
                .await;
        }

        if let Some(response) = best {
            self.forward_selected_failure(fork, response).await?;
        }
        Ok(())
    }

    async fn forward_generated_failure(
        &self,
        fork: &Arc<ForkContext>,
        status: StatusCode,
    ) -> Result<(), ProxyError> {
        let response = rvoip_sip_core::builder::SimpleResponseBuilder::response_from_request(
            &fork.original_request,
            status,
            None,
        )
        .build();
        let _ = self
            .dispatch_upstream_final(fork, response, false, false)
            .await?;
        Ok(())
    }

    async fn forward_selected_failure(
        &self,
        fork: &Arc<ForkContext>,
        response: Response,
    ) -> Result<(), ProxyError> {
        let _ = self
            .dispatch_upstream_final(fork, response, true, false)
            .await?;
        Ok(())
    }

    async fn upstream_final_is_terminal(&self, fork: &Arc<ForkContext>) -> bool {
        !matches!(
            *fork.upstream_final_dispatch.lock().await,
            UpstreamFinalDispatch::Unsent
        )
    }

    /// Serialize final-response selection and commit response-context ownership
    /// only after transaction-core classifies the exact write.
    ///
    /// `strip_proxy_via` distinguishes a selected downstream response from a
    /// locally generated one. `allow_additional_invite_2xx` preserves RFC 3261
    /// §16.7's requirement to forward every forked INVITE 2xx after the first
    /// successful final.
    async fn dispatch_upstream_final(
        &self,
        fork: &Arc<ForkContext>,
        response: Response,
        strip_proxy_via: bool,
        allow_additional_invite_2xx: bool,
    ) -> Result<FinalDispatchOutcome, ProxyError> {
        let mut dispatch = fork.upstream_final_dispatch.lock().await;
        match *dispatch {
            UpstreamFinalDispatch::TerminalDiscard => {
                return Ok(FinalDispatchOutcome::TerminalDiscard);
            }
            UpstreamFinalDispatch::Sent if !allow_additional_invite_2xx => {
                return Ok(FinalDispatchOutcome::AlreadySent);
            }
            UpstreamFinalDispatch::Sent => {
                let mut response = response;
                if strip_proxy_via {
                    remove_top_via_header(&mut response);
                }
                // Once the INVITE server transaction is Accepted, each later
                // 2xx owns an independent forwarding attempt. A send failure
                // is consumed under RFC 6026; it must not alter the already
                // committed first-final state or suppress later forked 2xx.
                match self
                    .tm
                    .send_response(&fork.upstream_server_tx, response)
                    .await
                {
                    Ok(()) => return Ok(FinalDispatchOutcome::AdditionalInvite2xxSent),
                    Err(error) => {
                        trace!(
                            upstream_transaction = %fork.upstream_server_tx,
                            %error,
                            "proxy: discarded additional INVITE 2xx after upstream send failure"
                        );
                        return Ok(FinalDispatchOutcome::TerminalDiscard);
                    }
                }
            }
            UpstreamFinalDispatch::Unsent => {}
        }

        match self
            .send_first_upstream_final_classified(fork, response, strip_proxy_via)
            .await
        {
            ClassifiedUpstreamSend::Sent => {
                *dispatch = UpstreamFinalDispatch::Sent;
                Ok(FinalDispatchOutcome::FirstSent)
            }
            ClassifiedUpstreamSend::Retryable(error) => {
                // The mutex serialized this attempt. Leaving the stable state
                // Unsent permits a later forked 2xx or aggregation retry to
                // become the authoritative first final.
                Err(error)
            }
            ClassifiedUpstreamSend::TerminalDiscard => {
                *dispatch = UpstreamFinalDispatch::TerminalDiscard;
                Ok(FinalDispatchOutcome::TerminalDiscard)
            }
        }
    }

    async fn send_first_upstream_final_classified(
        &self,
        fork: &Arc<ForkContext>,
        mut response: Response,
        strip_proxy_via: bool,
    ) -> ClassifiedUpstreamSend {
        if strip_proxy_via {
            remove_top_via_header(&mut response);
        }
        match self
            .tm
            .send_response(&fork.upstream_server_tx, response)
            .await
        {
            Ok(()) => ClassifiedUpstreamSend::Sent,
            Err(error) => {
                let disposition = self
                    .tm
                    .classify_final_response_completion(&fork.upstream_server_tx)
                    .await;
                match disposition {
                    FinalResponseCompletionDisposition::WrittenSuccessTerminal => {
                        ClassifiedUpstreamSend::Sent
                    }
                    FinalResponseCompletionDisposition::ZeroWireRetryable => {
                        ClassifiedUpstreamSend::Retryable(ProxyError::Transaction(
                            error.to_string(),
                        ))
                    }
                    FinalResponseCompletionDisposition::WireUnknownErrorTerminal => {
                        trace!(
                            upstream_transaction = %fork.upstream_server_tx,
                            %error,
                            "proxy: terminally discarded final response after an unknown write outcome"
                        );
                        ClassifiedUpstreamSend::TerminalDiscard
                    }
                }
            }
        }
    }

    /// Forward a provisional response through the exact upstream transaction.
    /// Provisional writes do not participate in final-response ownership.
    async fn forward_provisional_to_upstream(
        &self,
        fork: &Arc<ForkContext>,
        mut response: Response,
    ) -> Result<bool, ProxyError> {
        remove_top_via_header(&mut response);
        match self
            .tm
            .send_response(&fork.upstream_server_tx, response)
            .await
        {
            Ok(()) => Ok(true),
            Err(error) => {
                trace!(
                    upstream_transaction = %fork.upstream_server_tx,
                    %error,
                    "proxy: discarded provisional response because the upstream server transaction could not send it"
                );
                Ok(false)
            }
        }
    }

    /// Request cancellation for every pending leg except `winner`.
    /// Proceeding branches are dispatched now; Calling branches retain a
    /// latch that is dispatched on their first 101-199 response.
    async fn request_cancel_siblings(
        &self,
        fork: &Arc<ForkContext>,
        winner: Option<&TransactionKey>,
    ) {
        let cancel_targets: Vec<TransactionKey> = {
            let mut legs = fork.legs.lock().await;
            let mut to_cancel = Vec::new();
            for leg in legs.iter_mut() {
                if winner.is_some_and(|winner| leg.downstream_client_tx == *winner)
                    || leg.final_status.is_some()
                {
                    continue;
                }
                if !leg.cancel_requested {
                    leg.cancel_requested = true;
                }
                if leg.state == LegState::Proceeding && !leg.cancel_sent {
                    leg.cancel_sent = true;
                    to_cancel.push(leg.downstream_client_tx.clone());
                }
            }
            to_cancel
        };
        for tx_id in cancel_targets {
            self.send_cancel(fork, &tx_id).await;
        }
    }

    async fn send_cancel(&self, fork: &Arc<ForkContext>, tx_id: &TransactionKey) {
        // RFC 3261 §9.1 requires the generated CANCEL to reuse the INVITE
        // branch while changing the method. Install both ownership indexes
        // before dispatch so a loopback response or terminal event cannot
        // outrun registration.
        let expected_cancel_tx = tx_id.with_method(Method::Cancel);
        let generation = Arc::new(GeneratedCancelTransaction {
            transaction_id: expected_cancel_tx.clone(),
            terminated: AtomicBool::new(false),
            retryable_zero_wire: AtomicBool::new(false),
            retry_release_claimed: AtomicBool::new(false),
        });
        let owner = GeneratedCancelOwner {
            fork: Arc::clone(fork),
            invite_transaction_id: tx_id.clone(),
            generation: Arc::clone(&generation),
        };
        {
            let mut legs = fork.legs.lock().await;
            let Some(leg) = legs
                .iter_mut()
                .find(|leg| leg.downstream_client_tx == *tx_id)
            else {
                warn!(
                    "proxy: refusing generated CANCEL without its retained INVITE leg tx={}",
                    tx_id
                );
                return;
            };
            if leg.final_status.is_some() || leg.state == LegState::Terminated {
                leg.cancel_sent = false;
                self.cancel_cancel_retry(tx_id);
                return;
            }
            if leg.generated_cancel.is_some() {
                // A successful or wire-unknown generation already owns the
                // RFC transaction key. Never manufacture a duplicate.
                return;
            }
            match self.generated_cancels.entry(expected_cancel_tx.clone()) {
                Entry::Vacant(entry) => {
                    leg.generated_cancel = Some(Arc::clone(&generation));
                    entry.insert(owner.clone());
                }
                Entry::Occupied(_) => {
                    // A branch collision must not let this call steal another
                    // response context's generated transaction.
                    leg.cancel_sent = false;
                    let delay = leg
                        .cancel_retry_delay
                        .unwrap_or(self.tm.timer_settings().t1);
                    leg.cancel_retry_delay =
                        Some(delay.saturating_mul(2).min(self.tm.timer_settings().t2));
                    self.schedule_cancel_retry(tx_id.clone(), delay);
                    warn!(
                        "proxy: refusing generated CANCEL transaction-key collision tx={} cancel={}",
                        tx_id, expected_cancel_tx
                    );
                    return;
                }
            }
        }

        match self.tm.cancel_invite_transaction_classified(tx_id).await {
            CancelInviteTransactionDispatch::Success { transaction_id } => {
                debug_assert_eq!(
                    transaction_id, expected_cancel_tx,
                    "RFC CANCEL transaction key must reuse the INVITE branch"
                );
                self.cancel_cancel_retry(tx_id);
                {
                    let mut legs = fork.legs.lock().await;
                    if let Some(leg) = legs
                        .iter_mut()
                        .find(|leg| leg.downstream_client_tx == *tx_id)
                    {
                        leg.cancel_retry_delay = None;
                    }
                }
                debug!(
                    "proxy: CANCEL sent on branch tx={} cancel={}",
                    tx_id, transaction_id
                );
            }
            CancelInviteTransactionDispatch::WireUnknown {
                transaction_id,
                error,
            } => {
                debug_assert_eq!(
                    transaction_id, expected_cancel_tx,
                    "RFC CANCEL transaction key must reuse the INVITE branch"
                );
                self.cancel_cancel_retry(tx_id);
                {
                    let mut legs = fork.legs.lock().await;
                    if let Some(leg) = legs
                        .iter_mut()
                        .find(|leg| leg.downstream_client_tx == *tx_id)
                    {
                        leg.cancel_retry_delay = None;
                    }
                }
                warn!(
                    "proxy: CANCEL write outcome unknown; retaining exact generation tx={} cancel={}: {}",
                    tx_id, transaction_id, error
                );
            }
            CancelInviteTransactionDispatch::ZeroWire {
                error,
                retired_transaction_id,
            } => {
                generation
                    .retryable_zero_wire
                    .store(true, Ordering::Release);
                if let Some(retired_transaction_id) = retired_transaction_id {
                    debug_assert_eq!(
                        retired_transaction_id, expected_cancel_tx,
                        "retired zero-wire CANCEL must retain its exact RFC key"
                    );
                } else {
                    // Failure occurred before a CANCEL transaction generation
                    // was admitted, so no terminal event is owed.
                    generation.terminated.store(true, Ordering::Release);
                }
                // The exact internal transaction may have terminated before
                // classification returned. Otherwise cleanup_fork performs
                // this release when its ordered terminal event arrives. In
                // either ordering, the old same-key terminal event is consumed
                // before a retry is admitted, avoiding an ABA teardown race.
                if generation.terminated.load(Ordering::Acquire) {
                    self.release_zero_wire_cancel_and_schedule_retry(&owner)
                        .await;
                }
                warn!(
                    "proxy: CANCEL failed before the first wire attempt; retry remains owned tx={}: {}",
                    tx_id, error
                );
            }
        }
    }

    fn schedule_timer_c(&self, downstream_tx: TransactionKey) {
        let deadline = Instant::now() + self.config.timer_c;
        let generation = self.timer_c_generation.fetch_add(1, Ordering::Relaxed) + 1;
        let mut schedule = self
            .timer_c_schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        schedule_timer_c_entry(&mut schedule, downstream_tx, deadline, generation);
    }

    fn cancel_timer_c(&self, downstream_tx: &TransactionKey) {
        let mut schedule = self
            .timer_c_schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        schedule.current.remove(downstream_tx);
        compact_timer_c_heap_if_needed(&mut schedule);
    }

    fn schedule_cancel_retry(&self, downstream_tx: TransactionKey, delay: Duration) {
        let deadline = Instant::now() + delay;
        let generation = self.cancel_retry_generation.fetch_add(1, Ordering::Relaxed) + 1;
        let mut schedule = self
            .cancel_retry_schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        schedule_timer_c_entry(&mut schedule, downstream_tx, deadline, generation);
    }

    fn cancel_cancel_retry(&self, downstream_tx: &TransactionKey) {
        let mut schedule = self
            .cancel_retry_schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        schedule.current.remove(downstream_tx);
        compact_timer_c_heap_if_needed(&mut schedule);
    }

    fn next_cancel_retry_deadline(&self) -> Option<Instant> {
        self.cancel_retry_schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .heap
            .peek()
            .map(|entry| entry.deadline)
    }

    fn take_expired_cancel_retries(&self, now: Instant) -> Vec<TransactionKey> {
        let mut schedule = self
            .cancel_retry_schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        take_expired_timer_c_entries(&mut schedule, now)
    }

    async fn expire_cancel_retries(self: &Arc<Self>) {
        for downstream_tx in self.take_expired_cancel_retries(Instant::now()) {
            let Some(fork_ref) = self.forks_by_downstream.get(&downstream_tx) else {
                continue;
            };
            let fork = fork_ref.clone();
            drop(fork_ref);

            let dispatch = {
                let mut legs = fork.legs.lock().await;
                let Some(leg) = legs
                    .iter_mut()
                    .find(|leg| leg.downstream_client_tx == downstream_tx)
                else {
                    continue;
                };
                if leg.state == LegState::Proceeding
                    && leg.final_status.is_none()
                    && leg.cancel_requested
                    && !leg.cancel_sent
                    && leg.generated_cancel.is_none()
                {
                    leg.cancel_sent = true;
                    true
                } else {
                    false
                }
            };
            if dispatch {
                self.send_cancel(&fork, &downstream_tx).await;
            }
        }
    }

    fn next_timer_c_deadline(&self) -> Option<Instant> {
        self.timer_c_schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .heap
            .peek()
            .map(|entry| entry.deadline)
    }

    fn take_expired_timer_c(&self, now: Instant) -> Vec<TransactionKey> {
        let mut schedule = self
            .timer_c_schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        take_expired_timer_c_entries(&mut schedule, now)
    }

    async fn expire_timer_c(self: &Arc<Self>) {
        for downstream_tx in self.take_expired_timer_c(Instant::now()) {
            let Some(fork_ref) = self.forks_by_downstream.get(&downstream_tx) else {
                continue;
            };
            let fork = fork_ref.clone();
            drop(fork_ref);

            let (state, request) = {
                let legs = fork.legs.lock().await;
                let Some(leg) = legs
                    .iter()
                    .find(|leg| leg.downstream_client_tx == downstream_tx)
                else {
                    continue;
                };
                (leg.state, leg.forwarded_request.clone())
            };

            match state {
                LegState::Calling => {
                    let advanced = self
                        .try_advance_candidate(
                            &fork,
                            &downstream_tx,
                            Some(StatusCode::RequestTimeout),
                        )
                        .await;
                    let aggregate_result = if advanced {
                        Ok(())
                    } else {
                        let response =
                            rvoip_sip_core::builder::SimpleResponseBuilder::response_from_request(
                                &request,
                                StatusCode::RequestTimeout,
                                Some("Request Timeout"),
                            )
                            .build();
                        self.aggregate_failure(&fork, downstream_tx.clone(), response)
                            .await
                    };
                    // Timer C owns a proxy-local branch timeout, not the
                    // downstream INVITE transaction's RFC timers. Explicitly
                    // terminate that exact client generation after advancing
                    // or recording its branch-local 408 so it cannot linger
                    // until Timer B.
                    if let Err(error) = self.tm.terminate_transaction(&downstream_tx).await {
                        warn!(
                            downstream_transaction = %downstream_tx,
                            %error,
                            "proxy: Timer C could not terminate expired Calling transaction"
                        );
                    }
                    if let Err(error) = aggregate_result {
                        warn!("proxy: Timer C Calling-branch failure: {}", error);
                    }
                }
                LegState::Proceeding => {
                    {
                        let mut legs = fork.legs.lock().await;
                        if let Some(leg) = legs
                            .iter_mut()
                            .find(|leg| leg.downstream_client_tx == downstream_tx)
                        {
                            if leg.cancel_sent {
                                continue;
                            }
                            leg.cancel_requested = true;
                            leg.cancel_sent = true;
                        }
                    }
                    self.send_cancel(&fork, &downstream_tx).await;
                }
                LegState::Completed | LegState::Terminated => {}
            }
        }
    }

    fn schedule_fork_expiry(&self, upstream_tx: TransactionKey) {
        let deadline = Instant::now() + RESPONSE_CONTEXT_RETENTION;
        let generation = self.fork_expiry_generation.fetch_add(1, Ordering::Relaxed) + 1;
        let mut schedule = self
            .fork_expiry_schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if schedule.current.contains_key(&upstream_tx) {
            return;
        }
        schedule
            .current
            .insert(upstream_tx.clone(), (deadline, generation));
        schedule.heap.push(ForkExpiryEntry {
            deadline,
            generation,
            upstream_tx,
        });
    }

    fn next_fork_expiry_deadline(&self) -> Option<Instant> {
        let mut schedule = self
            .fork_expiry_schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            let entry = schedule.heap.peek()?;
            if schedule.current.get(&entry.upstream_tx).copied()
                == Some((entry.deadline, entry.generation))
            {
                return Some(entry.deadline);
            }
            schedule.heap.pop();
        }
    }

    fn take_expired_forks(&self, now: Instant) -> Vec<TransactionKey> {
        let mut schedule = self
            .fork_expiry_schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut expired = Vec::new();
        while expired.len() < FORK_EXPIRY_BATCH
            && schedule
                .heap
                .peek()
                .is_some_and(|entry| entry.deadline <= now)
        {
            let entry = schedule.heap.pop().expect("peeked fork expiry");
            if schedule.current.get(&entry.upstream_tx).copied()
                == Some((entry.deadline, entry.generation))
            {
                schedule.current.remove(&entry.upstream_tx);
                expired.push(entry.upstream_tx);
            }
        }
        expired
    }

    async fn expire_fork_contexts(&self) {
        for upstream_tx in self.take_expired_forks(Instant::now()) {
            let Some((_, fork)) = self.forks_by_upstream.remove(&upstream_tx) else {
                continue;
            };
            self.release_downstream_slots(fork.reserved_downstream_slots.swap(0, Ordering::AcqRel));
            let legs = fork.legs.lock().await;
            for leg in legs.iter() {
                self.forks_by_downstream
                    .remove_if(&leg.downstream_client_tx, |_, current| {
                        Arc::ptr_eq(current, &fork)
                    });
                self.cancel_timer_c(&leg.downstream_client_tx);
                self.cancel_cancel_retry(&leg.downstream_client_tx);
                self.known_branches.remove(&leg.proxy_branch);
                if let Some(cancel) = &leg.generated_cancel {
                    self.generated_cancels
                        .remove_if(&cancel.transaction_id, |_, owner| {
                            Arc::ptr_eq(&owner.fork, &fork)
                                && owner.invite_transaction_id == leg.downstream_client_tx
                                && Arc::ptr_eq(&owner.generation, cancel)
                        });
                }
            }
        }
    }

    fn register_stateless_response_route(
        &self,
        branch: String,
        retained: StatelessResponseRoute,
    ) -> Result<bool, ProxyError> {
        self.expire_stateless_response_routes();

        if let Some(existing_ref) = self.stateless_response_routes.get(&branch) {
            let existing = existing_ref.clone();
            drop(existing_ref);
            if existing.expires_at <= Instant::now() {
                self.remove_stateless_response_route(&branch, Some(existing.expires_at));
            } else if same_stateless_response_correlation(&existing, &retained) {
                // A retransmitted stateless request deliberately receives the
                // same deterministic branch. Keep the original bounded
                // deadline and correlation rather than creating stale timer
                // entries or allowing a new ingress flow to seize the route.
                return Ok(false);
            } else {
                return Err(ProxyError::Transport(
                    "stateless branch collision with different response correlation".into(),
                ));
            }
        }

        if self.stateless_response_routes.len() >= self.options.stateless_response_route_capacity {
            return Err(ProxyError::Transport(
                "stateless response-route capacity exhausted".into(),
            ));
        }

        let deadline = retained.expires_at;
        self.stateless_response_routes
            .insert(branch.clone(), retained);
        let mut schedule = self
            .stateless_response_expiry_schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        schedule.current.insert(branch.clone(), deadline);
        schedule
            .heap
            .push(StatelessResponseExpiryEntry { deadline, branch });
        compact_stateless_response_heap_if_needed(&mut schedule);
        Ok(true)
    }

    fn remove_stateless_response_route(
        &self,
        branch: &str,
        expected_deadline: Option<Instant>,
    ) -> bool {
        let removed = self
            .stateless_response_routes
            .remove_if(branch, |_, route| {
                expected_deadline.is_none_or(|deadline| route.expires_at == deadline)
            })
            .is_some();
        if !removed {
            return false;
        }

        let mut schedule = self
            .stateless_response_expiry_schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if schedule
            .current
            .get(branch)
            .is_some_and(|deadline| expected_deadline.is_none_or(|expected| *deadline == expected))
        {
            schedule.current.remove(branch);
        }
        compact_stateless_response_heap_if_needed(&mut schedule);
        drop(schedule);
        self.known_branches.remove(branch);
        true
    }

    fn next_stateless_response_expiry_deadline(&self) -> Option<Instant> {
        let mut schedule = self
            .stateless_response_expiry_schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            let entry = schedule.heap.peek()?;
            if schedule.current.get(&entry.branch).copied() == Some(entry.deadline) {
                return Some(entry.deadline);
            }
            schedule.heap.pop();
        }
    }

    fn take_expired_stateless_response_routes(&self, now: Instant) -> Vec<(String, Instant)> {
        let mut schedule = self
            .stateless_response_expiry_schedule
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut expired = Vec::new();
        let mut processed = 0;
        while processed < STATELESS_RESPONSE_EXPIRY_BATCH
            && schedule
                .heap
                .peek()
                .is_some_and(|entry| entry.deadline <= now)
        {
            let entry = schedule.heap.pop().expect("peeked stateless expiry");
            processed += 1;
            if schedule.current.get(&entry.branch).copied() == Some(entry.deadline) {
                schedule.current.remove(&entry.branch);
                expired.push((entry.branch, entry.deadline));
            }
        }
        compact_stateless_response_heap_if_needed(&mut schedule);
        expired
    }

    fn expire_stateless_response_routes(&self) {
        for (branch, deadline) in self.take_expired_stateless_response_routes(Instant::now()) {
            if self
                .stateless_response_routes
                .remove_if(&branch, |_, route| {
                    route.expires_at == deadline && route.expires_at <= Instant::now()
                })
                .is_some()
            {
                self.known_branches.remove(&branch);
            }
        }
    }

    async fn maybe_schedule_fork_expiry(&self, fork: &Arc<ForkContext>) {
        if !fork.upstream_terminated.load(Ordering::Acquire) {
            return;
        }
        let all_terminated = fork.legs.lock().await.iter().all(|leg| {
            leg.state == LegState::Terminated
                && leg
                    .generated_cancel
                    .as_ref()
                    .is_none_or(|cancel| cancel.terminated.load(Ordering::Acquire))
        });
        if all_terminated {
            self.schedule_fork_expiry(fork.upstream_server_tx.clone());
        }
    }

    async fn release_zero_wire_cancel_and_schedule_retry(&self, owner: &GeneratedCancelOwner) {
        if owner
            .generation
            .retry_release_claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        self.generated_cancels
            .remove_if(&owner.generation.transaction_id, |_, current| {
                Arc::ptr_eq(&current.generation, &owner.generation)
                    && Arc::ptr_eq(&current.fork, &owner.fork)
                    && current.invite_transaction_id == owner.invite_transaction_id
            });

        let retry_delay = {
            let mut legs = owner.fork.legs.lock().await;
            let Some(leg) = legs
                .iter_mut()
                .find(|leg| leg.downstream_client_tx == owner.invite_transaction_id)
            else {
                return;
            };
            if leg
                .generated_cancel
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &owner.generation))
            {
                leg.generated_cancel = None;
            }
            leg.cancel_sent = false;
            if leg.final_status.is_some()
                || matches!(leg.state, LegState::Completed | LegState::Terminated)
            {
                leg.cancel_retry_delay = None;
                None
            } else {
                // Only a proven pre-wire failure may release ownership and
                // retry. The old same-key terminal event has now been
                // consumed, so the central T1..T2 queue can safely admit the
                // next exact generation.
                let delay = leg
                    .cancel_retry_delay
                    .unwrap_or(self.tm.timer_settings().t1);
                leg.cancel_retry_delay =
                    Some(delay.saturating_mul(2).min(self.tm.timer_settings().t2));
                Some(delay)
            }
        };
        if let Some(delay) = retry_delay {
            self.schedule_cancel_retry(owner.invite_transaction_id.clone(), delay);
        }
        self.maybe_schedule_fork_expiry(&owner.fork).await;
    }

    /// Best-effort cleanup when a transaction terminates. Either the
    /// upstream server-tx or any downstream client-tx may be the
    /// terminating one; we resolve to the owning fork and reap when
    /// the upstream side is gone.
    async fn cleanup_fork(&self, tx_id: &TransactionKey) {
        if let Some(owner_ref) = self.generated_cancels.get(tx_id) {
            let owner = owner_ref.value().clone();
            drop(owner_ref);
            owner.generation.terminated.store(true, Ordering::Release);
            if owner.generation.retryable_zero_wire.load(Ordering::Acquire) {
                self.release_zero_wire_cancel_and_schedule_retry(&owner)
                    .await;
                return;
            }
            self.maybe_schedule_fork_expiry(&owner.fork).await;
            return;
        }

        if let Some(fork_ref) = self.forks_by_upstream.get(tx_id) {
            let fork = fork_ref.clone();
            drop(fork_ref);
            fork.upstream_terminated.store(true, Ordering::Release);
            self.maybe_schedule_fork_expiry(&fork).await;
            return;
        }
        if let Some(fork_ref) = self.forks_by_downstream.get(tx_id) {
            let fork = fork_ref.clone();
            drop(fork_ref);
            self.cancel_timer_c(tx_id);
            self.cancel_cancel_retry(tx_id);
            let mut legs = fork.legs.lock().await;
            if let Some(leg) = legs
                .iter_mut()
                .find(|leg| leg.downstream_client_tx == *tx_id)
            {
                leg.state = LegState::Terminated;
                self.known_branches.remove(&leg.proxy_branch);
            }
            drop(legs);
            self.maybe_schedule_fork_expiry(&fork).await;
        }
    }

    async fn handle_upstream_cancel(&self, upstream_tx_id: &TransactionKey) {
        if let Some(fork_ref) = self.forks_by_upstream.get(upstream_tx_id) {
            let fork = fork_ref.clone();
            drop(fork_ref);
            fork.upstream_cancelled.store(true, Ordering::Release);
            fork.stop_new_branches.store(true, Ordering::Release);
            self.request_cancel_siblings(&fork, None).await;
        }
    }

    /// Scan every Via entry on `request` and return the first branch
    /// value that matches a branch this proxy has previously stamped.
    /// Returning `Some(branch)` is the RFC 3261 §16.6 step-4
    /// loop-detected condition; the caller responds 482 upstream.
    fn retain_known_branch(&self, branch: String) {
        if self.options.legacy_branch_loop_detection {
            self.known_branches.insert(branch, ());
        }
    }

    fn find_known_branch_in_request(&self, request: &Request) -> Option<String> {
        if self.known_branches.is_empty() {
            return None;
        }
        use rvoip_sip_core::types::param::Param;
        for via in request.via_headers() {
            for entry in &via.0 {
                let branch = entry.params.iter().find_map(|p| match p {
                    Param::Branch(b) => Some(b.as_str()),
                    _ => None,
                });
                if let Some(b) = branch {
                    if self.known_branches.contains_key(b) {
                        return Some(b.to_string());
                    }
                }
            }
        }
        None
    }

    async fn respond_locally(
        &self,
        upstream_tx_id: &TransactionKey,
        original_request: &Request,
        status: StatusCode,
    ) -> Result<(), ProxyError> {
        let response = crate::local_response::local_response_from_request(
            original_request,
            upstream_tx_id,
            status,
            None,
        );
        self.tm
            .send_response(upstream_tx_id, response)
            .await
            .map_err(|e| ProxyError::Transaction(e.to_string()))
    }

    fn decrement_max_forwards(&self, request: &mut Request) -> Result<(), ProxyError> {
        for header in &mut request.headers {
            if let TypedHeader::MaxForwards(mf) = header {
                if mf.0 == 0 {
                    return Err(ProxyError::MaxForwardsExhausted);
                }
                mf.0 -= 1;
                return Ok(());
            }
        }
        // RFC 3261 §16.6 step 3 — add 70 when the request did not carry
        // Max-Forwards. Only an existing value is decremented.
        request
            .headers
            .push(TypedHeader::MaxForwards(MaxForwards::new(70)));
        Ok(())
    }
}

fn same_stateless_response_correlation(
    left: &StatelessResponseRoute,
    right: &StatelessResponseRoute,
) -> bool {
    left.upstream_route == right.upstream_route
        && left.downstream_route == right.downstream_route
        && left.cseq_sequence == right.cseq_sequence
        && left.cseq_method == right.cseq_method
}

fn stateless_response_route_matches(expected: &TransportRoute, actual: &TransportRoute) -> bool {
    if expected.destination != actual.destination
        || expected.transport_type != actual.transport_type
    {
        return false;
    }

    match expected.transport_type {
        Some(rvoip_sip_transport::transport::TransportType::Udp) => {
            expected.flow_id.is_none() && actual.flow_id.is_none()
        }
        Some(
            rvoip_sip_transport::transport::TransportType::Tcp
            | rvoip_sip_transport::transport::TransportType::Tls
            | rvoip_sip_transport::transport::TransportType::Ws
            | rvoip_sip_transport::transport::TransportType::Wss,
        ) => expected.flow_id.is_some() && expected.flow_id == actual.flow_id,
        None => expected.flow_id == actual.flow_id,
    }
}

fn response_preference(status: StatusCode) -> (u8, u16) {
    let code = status.as_u16();
    let class_rank = match code {
        600..=699 => 0,
        300..=399 => 1,
        400..=499 => 2,
        // RFC 3261 §16.7 makes 503 a last resort: it must not be
        // selected if the response context received any other final.
        503 => u8::MAX,
        500..=599 => 3,
        _ => 4,
    };
    // Within the selected class, prefer responses that commonly let the UAC
    // reformulate and resubmit the request.
    let within_class = match code {
        401 | 407 | 415 | 420 | 484 => 0,
        _ => code,
    };
    (class_rank, within_class)
}

fn aggregate_authenticate_headers<'a>(
    selected: &mut Response,
    responses: impl Iterator<Item = &'a Response>,
) {
    if !matches!(selected.status().as_u16(), 401 | 407) {
        return;
    }

    // RFC 3261 §16.7 step 7 requires a selected 401 or 407 response to
    // contain *both* authentication header families collected from every
    // received 401 and 407. Restricting aggregation to the selected status
    // loses either origin-server or proxy challenges in a mixed fork.
    selected.headers.retain(|header| {
        !matches!(
            header,
            TypedHeader::WwwAuthenticate(_) | TypedHeader::ProxyAuthenticate(_)
        )
    });
    for response in responses.filter(|response| matches!(response.status().as_u16(), 401 | 407)) {
        selected.headers.extend(
            response
                .headers
                .iter()
                .filter(|header| {
                    matches!(
                        header,
                        TypedHeader::WwwAuthenticate(_) | TypedHeader::ProxyAuthenticate(_)
                    )
                })
                .cloned(),
        );
    }
}

/// RFC 3261 §16.11 branch for a statelessly forwarded request. The input is
/// deliberately method-independent so an INVITE, its CANCEL, and its non-2xx
/// ACK correlate when the invariant routing fields are identical.
fn stateless_proxy_branch(request: &Request, local_addr: SocketAddr) -> String {
    fn part(hasher: &mut Sha256, label: &[u8], value: &[u8]) {
        hasher.update((label.len() as u32).to_be_bytes());
        hasher.update(label);
        hasher.update((value.len() as u32).to_be_bytes());
        hasher.update(value);
    }

    let mut hasher = Sha256::new();
    part(&mut hasher, b"proxy", local_addr.to_string().as_bytes());
    part(&mut hasher, b"uri", request.uri().to_string().as_bytes());
    let mut saw_top_via = false;
    for header in &request.headers {
        match header {
            TypedHeader::Via(_) if !saw_top_via => {
                saw_top_via = true;
                part(&mut hasher, b"via", header.to_string().as_bytes());
            }
            TypedHeader::From(_) => {
                part(&mut hasher, b"from", header.to_string().as_bytes());
            }
            TypedHeader::CallId(_) => {
                part(&mut hasher, b"call-id", header.to_string().as_bytes());
            }
            TypedHeader::CSeq(cseq) => {
                part(&mut hasher, b"cseq", cseq.seq.to_string().as_bytes());
            }
            TypedHeader::Route(_) => {
                part(&mut hasher, b"route", header.to_string().as_bytes());
            }
            TypedHeader::ProxyRequire(_) => {
                part(&mut hasher, b"proxy-require", header.to_string().as_bytes());
            }
            _ => {}
        }
    }
    let digest = hasher.finalize();
    format!("z9hG4bK-proxy-{:x}", digest)
}

fn push_proxy_via(
    request: &mut Request,
    local_addr: SocketAddr,
    transport: TransportType,
    branch: &str,
) -> Result<(), ProxyError> {
    let transport = transport_token(transport);
    let host = local_addr.ip().to_string();
    let port = Some(local_addr.port());

    // Build a fresh single-entry Via for the proxy.
    let mut via = Via(Vec::new());
    via.push_proxy_branch(transport, host, port, branch)
        .map_err(|e| ProxyError::Transport(format!("push Via: {}", e)))?;

    // Insert as a NEW typed-header at the position of the first
    // existing Via, pushing the UAC's Via down by one. This keeps the
    // proxy and UAC entries in separate typed-headers so on the
    // response-forwarding path we can remove the proxy's typed-header
    // wholesale without leaving an empty Via behind.
    let pos = request
        .headers
        .iter()
        .position(|h| matches!(h, TypedHeader::Via(_)))
        .unwrap_or(request.headers.len());
    request.headers.insert(pos, TypedHeader::Via(via));
    Ok(())
}

fn remove_top_request_via(request: &mut Request) {
    if let Some(index) = request
        .headers
        .iter()
        .position(|header| matches!(header, TypedHeader::Via(_)))
    {
        request.headers.remove(index);
    }
}

fn request_rejection_error(rejection: RequestRejection) -> ProxyError {
    ProxyError::Transport(format!(
        "request rejected by RFC 3261 routing policy: {rejection:?}"
    ))
}

fn authority_for_uri(uri: &Uri) -> Result<TransportAuthority, ProxyError> {
    match &uri.host {
        Host::Domain(domain) => TransportAuthority::dns(domain.clone())
            .map_err(|error| ProxyError::Transport(error.to_string())),
        Host::Address(address) => Ok(TransportAuthority::ip(*address)),
    }
}

fn resolved_target_route(
    target: ResolvedTarget,
    next_hop_uri: &Uri,
) -> Result<TransportRoute, ProxyError> {
    let authority = match target.authority {
        Some(authority) => authority,
        None => authority_for_uri(next_hop_uri)?,
    };
    Ok(TransportRoute::new(target.addr)
        .with_transport_type(target.transport)
        .with_authority(authority))
}

/// Extract every Contact URI from a (typically 3xx) response, in
/// header-then-entry order. Returns an empty Vec when no Contact is
/// present or all entries are wildcard. Used by the redirect-event
/// emitter to surface candidate retry targets to applications.
fn extract_contact_uris(response: &Response) -> Vec<Uri> {
    let mut out = Vec::new();
    for header in &response.headers {
        if let TypedHeader::Contact(contact) = header {
            for addr in contact.addresses() {
                out.push(addr.uri.clone());
            }
        }
    }
    out
}

fn remove_top_via_header(response: &mut Response) {
    if let Some(pos) = response
        .headers
        .iter()
        .position(|h| matches!(h, TypedHeader::Via(_)))
    {
        let remove_header = match &mut response.headers[pos] {
            TypedHeader::Via(via) if via.0.len() > 1 => {
                via.0.remove(0);
                false
            }
            TypedHeader::Via(_) => true,
            _ => false,
        };
        if remove_header {
            response.headers.remove(pos);
        }
    }
}

/// Pick a Via `sent-protocol` transport token (UDP / TCP / TLS / WS /
/// WSS) for a forwarded request. We honour the next-hop URI's
/// `;transport=` parameter / scheme just like the originating UAC,
/// since the proxy's Via is what the downstream uses for symmetric
/// response routing.
fn transport_token(transport: TransportType) -> &'static str {
    match transport {
        TransportType::Udp => "UDP",
        TransportType::Tcp => "TCP",
        TransportType::Tls => "TLS",
        TransportType::Ws => "WS",
        TransportType::Wss => "WSS",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
    use rvoip_sip_core::types::content_length::ContentLength;
    use rvoip_sip_core::types::param::Param;

    #[test]
    fn response_processing_pops_one_value_from_a_packed_via_header() {
        let mut packed = Via::new(
            "SIP",
            "2.0",
            "UDP",
            "proxy.example.com",
            Some(5060),
            vec![Param::branch("z9hG4bK-proxy-packed")],
        )
        .unwrap();
        let upstream = Via::new(
            "SIP",
            "2.0",
            "UDP",
            "uac.example.com",
            Some(5060),
            vec![Param::branch("z9hG4bK-uac-packed")],
        )
        .unwrap();
        packed.0.extend(upstream.0);

        let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", None)
            .call_id("packed-via")
            .cseq(1)
            .header(TypedHeader::Via(packed))
            .header(TypedHeader::ContentLength(ContentLength::new(0)))
            .build();
        let mut response =
            SimpleResponseBuilder::response_from_request(&request, StatusCode::Ok, None).build();

        remove_top_via_header(&mut response);

        let remaining = response.first_via().expect("upstream Via retained");
        assert_eq!(remaining.branch(), Some("z9hG4bK-uac-packed"));
        assert_eq!(response.via_headers().len(), 1);
    }

    #[test]
    fn stateless_branch_is_method_independent_but_routing_sensitive() {
        let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", None)
            .call_id("stateless-correlation")
            .cseq(42)
            .header(TypedHeader::Via(
                Via::new(
                    "SIP",
                    "2.0",
                    "UDP",
                    "uac.example.com",
                    Some(5060),
                    vec![Param::branch("z9hG4bK-uac-correlation")],
                )
                .unwrap(),
            ))
            .header(TypedHeader::ContentLength(ContentLength::new(0)))
            .build();
        let mut cancel = invite.clone();
        cancel.method = Method::Cancel;
        for header in &mut cancel.headers {
            if let TypedHeader::CSeq(cseq) = header {
                cseq.method = Method::Cancel;
            }
        }
        let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        assert_eq!(
            stateless_proxy_branch(&invite, local_addr),
            stateless_proxy_branch(&cancel, local_addr)
        );
        let mut ack = invite.clone();
        ack.method = Method::Ack;
        for header in &mut ack.headers {
            if let TypedHeader::CSeq(cseq) = header {
                cseq.method = Method::Ack;
            }
        }
        assert_eq!(
            stateless_proxy_branch(&invite, local_addr),
            stateless_proxy_branch(&ack, local_addr)
        );

        let mut rerouted = cancel;
        rerouted.uri = "sip:bob@other.example.com".parse().unwrap();
        assert_ne!(
            stateless_proxy_branch(&invite, local_addr),
            stateless_proxy_branch(&rerouted, local_addr)
        );
    }

    #[test]
    fn production_configuration_rejects_timer_c_at_or_below_three_minutes() {
        let mut config = ProxyConfig::default();
        config.timer_c = Duration::from_secs(180);
        assert!(matches!(
            ProxyRuntimeOptions::default().validate(&config),
            Err(ProxyBuildError::InvalidConfiguration(_))
        ));
        config.timer_c = Duration::from_secs(181);
        assert!(ProxyRuntimeOptions::default().validate(&config).is_ok());
    }

    #[test]
    fn production_configuration_rejects_zero_state_capacities() {
        let config = ProxyConfig::default();
        assert!(ProxyRuntimeOptions::default()
            .with_response_context_capacity(0)
            .validate(&config)
            .is_err());
        assert!(ProxyRuntimeOptions::default()
            .with_downstream_transaction_capacity(0)
            .validate(&config)
            .is_err());
        assert!(ProxyRuntimeOptions::default()
            .with_branches_per_response_context(0)
            .validate(&config)
            .is_err());
        assert!(ProxyRuntimeOptions::default()
            .with_stateless_response_route_capacity(0)
            .validate(&config)
            .is_err());
    }

    #[test]
    fn legacy_loop_detector_is_disabled_by_default() {
        assert!(!ProxyRuntimeOptions::default().legacy_branch_loop_detection);
    }

    #[tokio::test(start_paused = true)]
    async fn timer_c_thousands_of_resets_keep_one_live_entry_and_a_bounded_heap() {
        let mut schedule = TimerCSchedule::default();
        let downstream_tx =
            TransactionKey::new("z9hG4bK-reset-stress".into(), Method::Invite, false);
        let started_at = Instant::now();

        for generation in 1..=10_000_u64 {
            schedule_timer_c_entry(
                &mut schedule,
                downstream_tx.clone(),
                started_at + Duration::from_secs(181) + Duration::from_micros(generation),
                generation,
            );
        }

        assert_eq!(schedule.current.len(), 1);
        let physical_bound = schedule
            .current
            .len()
            .saturating_mul(TIMER_C_HEAP_COMPACTION_FACTOR)
            .saturating_add(TIMER_C_HEAP_COMPACTION_SLACK);
        assert!(
            schedule.heap.len() <= physical_bound,
            "10,000 resets retained {} physical entries for one live branch (bound {})",
            schedule.heap.len(),
            physical_bound
        );
        assert_eq!(
            schedule
                .current
                .get(&downstream_tx)
                .map(|(_, generation)| *generation),
            Some(10_000)
        );

        tokio::time::advance(Duration::from_secs(182)).await;
        let expired = take_expired_timer_c_entries(&mut schedule, Instant::now());
        assert_eq!(expired, vec![downstream_tx]);
        assert!(schedule.current.is_empty());
        assert!(schedule.heap.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn timer_c_due_work_is_bounded_and_remains_immediately_runnable() {
        let mut schedule = TimerCSchedule::default();
        let deadline = Instant::now() + Duration::from_secs(1);
        for index in 0..1_025_u64 {
            let transaction = TransactionKey::new(
                format!("z9hG4bK-expiry-batch-{index}"),
                Method::Invite,
                false,
            );
            schedule_timer_c_entry(&mut schedule, transaction, deadline, index + 1);
        }
        assert_eq!(schedule.current.len(), 1_025);
        assert_eq!(schedule.heap.len(), 1_025);

        tokio::time::advance(Duration::from_secs(2)).await;
        let now = Instant::now();
        let mut batch_sizes = Vec::new();
        while !schedule.current.is_empty() {
            let expired = take_expired_timer_c_entries(&mut schedule, now);
            assert!(
                expired.len() <= TIMER_C_EXPIRY_BATCH,
                "one event-loop pass processed more than the configured batch"
            );
            assert!(
                !expired.is_empty(),
                "due Timer C work failed to make progress"
            );
            batch_sizes.push(expired.len());
            if !schedule.current.is_empty() {
                assert!(
                    schedule
                        .heap
                        .peek()
                        .is_some_and(|entry| entry.deadline <= now),
                    "remaining due work must force an immediate event-loop wakeup"
                );
            }
        }

        assert_eq!(batch_sizes, vec![256, 256, 256, 256, 1]);
        assert!(schedule.heap.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn timer_c_stale_due_entries_are_bounded_and_converge_without_expiring_live_resets() {
        let mut schedule = TimerCSchedule::default();
        let stale_deadline = Instant::now() + Duration::from_secs(1);
        let live_deadline = Instant::now() + Duration::from_secs(10);
        let mut transactions = Vec::new();
        for index in 0..1_025_u64 {
            let transaction = TransactionKey::new(
                format!("z9hG4bK-stale-expiry-batch-{index}"),
                Method::Invite,
                false,
            );
            schedule_timer_c_entry(
                &mut schedule,
                transaction.clone(),
                stale_deadline,
                index + 1,
            );
            transactions.push(transaction);
        }
        for (index, transaction) in transactions.into_iter().enumerate() {
            schedule_timer_c_entry(
                &mut schedule,
                transaction,
                live_deadline,
                2_000 + index as u64,
            );
        }
        assert_eq!(schedule.current.len(), 1_025);
        assert_eq!(schedule.heap.len(), 2_050);

        tokio::time::advance(Duration::from_secs(2)).await;
        let now = Instant::now();
        let mut stale_batch_sizes = Vec::new();
        while schedule
            .heap
            .peek()
            .is_some_and(|entry| entry.deadline <= now)
        {
            let before = schedule.heap.len();
            let expired = take_expired_timer_c_entries(&mut schedule, now);
            let processed = before - schedule.heap.len();
            assert!(
                processed <= TIMER_C_EXPIRY_BATCH,
                "one pass removed more than the bounded stale-entry batch"
            );
            assert!(processed > 0, "stale due work failed to converge");
            assert!(
                expired.is_empty(),
                "a stale reset deadline must not expire its live replacement"
            );
            stale_batch_sizes.push(processed);
        }

        assert_eq!(stale_batch_sizes, vec![256, 256, 256, 256, 1]);
        assert_eq!(schedule.current.len(), 1_025);
        assert_eq!(schedule.heap.len(), 1_025);
        assert!(schedule
            .heap
            .peek()
            .is_some_and(|entry| entry.deadline > now));
    }

    #[tokio::test(start_paused = true)]
    async fn stateless_response_deadline_churn_keeps_the_physical_heap_bounded() {
        let mut schedule = StatelessResponseExpirySchedule::default();
        let live_branch = "z9hG4bK-live-stateless".to_string();
        let live_deadline = Instant::now() + Duration::from_secs(64);
        schedule.current.insert(live_branch.clone(), live_deadline);
        schedule.heap.push(StatelessResponseExpiryEntry {
            deadline: live_deadline,
            branch: live_branch,
        });

        for index in 0..10_000_u64 {
            let branch = format!("z9hG4bK-failed-stateless-{index}");
            let deadline = Instant::now() + Duration::from_secs(64);
            schedule.current.insert(branch.clone(), deadline);
            schedule.heap.push(StatelessResponseExpiryEntry {
                deadline,
                branch: branch.clone(),
            });
            schedule.current.remove(&branch);
            compact_stateless_response_heap_if_needed(&mut schedule);
        }

        let physical_bound = schedule
            .current
            .len()
            .saturating_mul(STATELESS_RESPONSE_HEAP_COMPACTION_FACTOR)
            .saturating_add(STATELESS_RESPONSE_HEAP_COMPACTION_SLACK);
        assert_eq!(schedule.current.len(), 1);
        assert!(
            schedule.heap.len() <= physical_bound,
            "stateless send-failure churn retained {} physical deadlines (bound {})",
            schedule.heap.len(),
            physical_bound
        );

        tokio::time::advance(Duration::from_secs(65)).await;
        while schedule
            .heap
            .peek()
            .is_some_and(|entry| entry.deadline <= Instant::now())
        {
            let entry = schedule.heap.pop().expect("peeked stateless expiry");
            if schedule.current.get(&entry.branch).copied() == Some(entry.deadline) {
                schedule.current.remove(&entry.branch);
            }
        }
        compact_stateless_response_heap_if_needed(&mut schedule);
        assert!(schedule.current.is_empty());
        assert!(schedule.heap.is_empty());
    }
}
