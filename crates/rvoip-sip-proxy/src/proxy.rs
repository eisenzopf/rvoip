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

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use rvoip_sip_core::types::headers::header_name::HeaderName;
use rvoip_sip_core::types::max_forwards::MaxForwards;
use rvoip_sip_core::types::status::StatusCode;
use rvoip_sip_core::types::uri::Uri;
use rvoip_sip_core::types::via::Via;
use rvoip_sip_core::types::TypedHeader;
use rvoip_sip_core::{Method, Request, Response};
use rvoip_sip_dialog::transaction::{TransactionEvent, TransactionKey, TransactionManager};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tracing::{debug, info, trace, warn};

use crate::error::ProxyError;

/// Default Timer C duration — RFC 3261 §16.8 recommends "greater than
/// 3 minutes". We pick 3 min on the nose; applications override via
/// [`ProxyConfig::timer_c`].
pub const DEFAULT_TIMER_C: Duration = Duration::from_secs(180);

/// Application-supplied routing function.
///
/// Called for every inbound request that needs forwarding. Returns
/// `Some(RouteDecision)` to forward the request to a destination,
/// `None` to drop the request (the proxy will reply 404 upstream).
///
/// The closure runs on the proxy event-loop task, so it must not
/// block — defer slow lookups to a separate task and return the
/// resolved address asynchronously via a channel + cache if needed.
pub type RouteFn = Arc<dyn Fn(&Request) -> Option<RouteDecision> + Send + Sync + 'static>;

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
///   candidate list the proxy walks on transport failure).
/// - `RouteDecision::sequential_with_failover(vec![vec![..], ..])` —
///   same shape, sequential mode.
#[derive(Debug, Clone)]
pub struct RouteDecision {
    pub mode: ForkMode,
    pub targets: Vec<SocketAddr>,
    /// Optional per-leg RFC 3263 §4.3 candidate lists. When non-empty,
    /// each `Vec<SocketAddr>` is one leg's candidate list — the proxy
    /// tries entries in order on transport-level failure. When empty,
    /// `targets` is used as a 1-element-per-leg candidate list (the
    /// pre-failover behaviour). Outer length defines the fork count;
    /// when both `targets` and `leg_candidates` are set, the latter
    /// wins.
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
    /// in order on transport-level failure (RFC 3263 §4.3).
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

    /// Candidate list for leg index `idx`. Returns a single-element
    /// slice from `targets` when no per-leg candidates were supplied,
    /// otherwise returns the configured candidate vec.
    pub(crate) fn candidates_for_leg(&self, idx: usize) -> Vec<SocketAddr> {
        if let Some(candidates) = self.leg_candidates.get(idx) {
            candidates.clone()
        } else {
            self.targets.get(idx).copied().into_iter().collect()
        }
    }

    /// Number of legs the decision describes — driven by
    /// `leg_candidates` when present, otherwise by `targets`.
    pub(crate) fn leg_count(&self) -> usize {
        if !self.leg_candidates.is_empty() {
            self.leg_candidates.len()
        } else {
            self.targets.len()
        }
    }
}

/// Configuration knobs for the stateful proxy. Apply via
/// [`StatefulProxy::with_config`] or by constructing via
/// [`StatefulProxy::builder`] (TODO Phase 7).
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
    /// All downstream targets the application asked us to try (one
    /// representative `SocketAddr` per leg; for per-leg failover this
    /// is the first candidate).
    targets: Vec<SocketAddr>,
    /// Per-leg candidate lists for RFC 3263 §4.3 failover at the leg
    /// level. Empty when the application did not request per-leg
    /// failover — in that case each leg has a single candidate
    /// (`targets[idx]`).
    leg_candidates: Vec<Vec<SocketAddr>>,
    /// Number of legs already started — used by sequential mode to
    /// advance to the next leg index on failure (replaces the prior
    /// address-set scan which broke once a leg could have multiple
    /// candidates).
    legs_started: std::sync::atomic::AtomicUsize,
    /// Per-leg state. In Parallel mode, all legs are populated up-front.
    /// In Sequential mode, legs are populated one at a time as earlier
    /// ones fail.
    legs: tokio::sync::Mutex<Vec<Leg>>,
    /// Has an upstream response been forwarded yet? Used to short-
    /// circuit late responses (e.g., second 2xx after first wins).
    upstream_responded: std::sync::atomic::AtomicBool,
}

struct Leg {
    downstream_client_tx: TransactionKey,
    destination: SocketAddr,
    /// Final response received on this leg, if any. `None` while the
    /// leg is still pending; `Some(status)` once a final response has
    /// arrived. Forward-progress is tracked here so the aggregator can
    /// decide whether all legs are "done".
    final_status: Option<StatusCode>,
    /// CANCELed by us (after a 2xx-wins on a sibling leg, or an
    /// upstream CANCEL). Pending CANCEL responses on this leg are
    /// expected and not forwarded upstream.
    cancelled: bool,
    /// The best response received on this leg, kept so §16.7 step 6
    /// "best response" selection works after all legs settle.
    last_response: Option<Response>,
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
    route_fn: RouteFn,

    /// Fork contexts keyed by the upstream server transaction.
    forks_by_upstream: DashMap<TransactionKey, Arc<ForkContext>>,
    /// Reverse lookup: downstream client-tx → fork context. Populated
    /// every time a leg is started; cleaned up when the leg terminates.
    forks_by_downstream: DashMap<TransactionKey, Arc<ForkContext>>,

    // Timer C task handles, keyed by upstream server-tx id. Aborted
    // on final response upstream. Resets per 1xx per RFC 3261 §16.8.
    timer_c_tasks: DashMap<TransactionKey, JoinHandle<()>>,

    /// Set of `z9hG4bK-proxy-…` branches this proxy has stamped on
    /// outbound Vias. Used for RFC 3261 §16.6 step 4 loop detection:
    /// if an inbound request's Via stack contains a branch in this
    /// set, the request looped back through us and we reject with
    /// 482 Loop Detected. `DashMap<String, ()>` is used as a
    /// concurrent set (DashSet isn't in the dependency tree).
    known_branches: DashMap<String, ()>,

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
            .field("forks", &self.forks_by_upstream.len())
            .field("timer_c_tasks", &self.timer_c_tasks.len())
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
        Arc::new(Self {
            tm,
            config,
            route_fn,
            forks_by_upstream: DashMap::new(),
            forks_by_downstream: DashMap::new(),
            timer_c_tasks: DashMap::new(),
            known_branches: DashMap::new(),
            event_tx: broadcast::channel(64).0,
            redirect_interceptor: std::sync::RwLock::new(None),
        })
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
        tokio::spawn(async move {
            self.event_loop(events).await;
        })
    }

    async fn event_loop(self: Arc<Self>, mut rx: mpsc::Receiver<TransactionEvent>) {
        info!("StatefulProxy event loop started");
        while let Some(event) = rx.recv().await {
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
                    if let Err(e) = self
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
                    if let Err(e) = self
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
                    if let Err(e) = self
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
                    if let Err(e) = self
                        .aggregate_response(transaction_id, response, /* final */ true)
                        .await
                    {
                        warn!("proxy: aggregate final failed: {}", e);
                    }
                }
                TransactionEvent::CancelReceived {
                    transaction_id: upstream_tx,
                    ..
                } => {
                    // Upstream UAC sent CANCEL — fan out CANCEL to all
                    // still-pending downstream legs.
                    self.handle_upstream_cancel(&upstream_tx).await;
                }
                TransactionEvent::TransactionTerminated { transaction_id } => {
                    self.cleanup_fork(&transaction_id).await;
                }
                _ => {
                    trace!("proxy: ignoring event {:?}", event);
                }
            }
        }
        info!("StatefulProxy event loop exited");
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

        // RFC 3261 §16.6 step 4 — loop detection. If any branch in
        // the inbound Via stack matches a branch this proxy has
        // previously stamped, the request has looped back through us
        // and we MUST reject with 482 (Loop Detected).
        if let Some(looped_branch) = self.find_known_branch_in_request(&request) {
            warn!(
                "proxy: loop detected — inbound Via carries our previously-stamped branch {}; sending 482",
                looped_branch
            );
            self.respond_locally(&upstream_tx_id, &original_request, StatusCode::LoopDetected)
                .await?;
            return Err(ProxyError::LoopDetected);
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

        // Routing decision from the application.
        let decision = match (self.route_fn)(&request) {
            Some(d) if !d.targets.is_empty() => d,
            _ => {
                self.respond_locally(&upstream_tx_id, &original_request, StatusCode::NotFound)
                    .await?;
                return Ok(());
            }
        };

        // Build the fork context up-front so every leg's downstream
        // tx_id can look back to it via `forks_by_downstream`.
        let fork = Arc::new(ForkContext {
            upstream_server_tx: upstream_tx_id.clone(),
            is_invite,
            mode: decision.mode,
            original_request: original_request.clone(),
            targets: decision.targets.clone(),
            leg_candidates: decision.leg_candidates.clone(),
            legs_started: std::sync::atomic::AtomicUsize::new(0),
            legs: tokio::sync::Mutex::new(Vec::new()),
            upstream_responded: std::sync::atomic::AtomicBool::new(false),
        });
        self.forks_by_upstream
            .insert(upstream_tx_id.clone(), fork.clone());

        // Timer C: per fork, only for INVITE.
        if is_invite {
            self.start_timer_c(upstream_tx_id.clone());
        }

        let leg_count = decision.leg_count();
        match decision.mode {
            ForkMode::Parallel => {
                // Fire every leg in one batch. Each leg may carry a
                // candidate list (RFC 3263 §4.3) — start_leg walks
                // them internally on transport failure.
                for idx in 0..leg_count {
                    let candidates = decision.candidates_for_leg(idx);
                    fork.legs_started
                        .store(idx + 1, std::sync::atomic::Ordering::Release);
                    if let Err(e) = self.start_leg(&fork, &request, &candidates).await {
                        warn!(
                            "proxy: failed to start parallel leg {} (candidates {:?}): {}",
                            idx, candidates, e
                        );
                    }
                }
            }
            ForkMode::Sequential => {
                // Start with the first leg only. Subsequent legs
                // are kicked off in `aggregate_failure`.
                if leg_count > 0 {
                    let candidates = decision.candidates_for_leg(0);
                    fork.legs_started
                        .store(1, std::sync::atomic::Ordering::Release);
                    self.start_leg(&fork, &request, &candidates).await?;
                }
            }
        }

        Ok(())
    }

    /// Push a fresh proxy Via onto `request`, build a downstream client
    /// transaction targeting one of `candidates`, register the leg
    /// with the fork context, and send the request.
    ///
    /// When `candidates.len() > 1`, the method walks the list in order
    /// on transport-level send failures (RFC 3263 §4.3 multi-candidate
    /// failover at the leg level). Each retry stamps a fresh proxy
    /// branch so the Via stack stays §16.6-valid across attempts.
    /// Returns the first successful send; otherwise the last error.
    async fn start_leg(
        &self,
        fork: &Arc<ForkContext>,
        base_request: &Request,
        candidates: &[SocketAddr],
    ) -> Result<(), ProxyError> {
        if candidates.is_empty() {
            return Err(ProxyError::Transport(
                "start_leg called with no candidates".into(),
            ));
        }

        let local_addr = self
            .tm
            .transport()
            .local_addr()
            .map_err(|e| ProxyError::Transport(e.to_string()))?;

        let total = candidates.len();
        let mut last_err: Option<ProxyError> = None;

        for (idx, destination) in candidates.iter().enumerate() {
            let attempt = idx + 1;
            // Each attempt gets a fresh request clone + fresh proxy
            // Via with a unique branch so RFC 3261 §16.6 branch
            // uniqueness holds across the candidate walk.
            let mut leg_request = base_request.clone();
            let proxy_branch = format!("z9hG4bK-proxy-{}", uuid::Uuid::new_v4().simple());
            if let Err(e) = push_proxy_via(&mut leg_request, local_addr, &proxy_branch) {
                last_err = Some(e);
                continue;
            }
            self.known_branches.insert(proxy_branch.clone(), ());

            let downstream_tx_id = match self
                .tm
                .create_client_transaction(leg_request, *destination)
                .await
            {
                Ok(id) => id,
                Err(e) => {
                    last_err = Some(ProxyError::Transaction(format!(
                        "RFC 3263 §4.3 leg candidate {}/{} ({}): create_client_transaction: {}",
                        attempt, total, destination, e
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
                    destination: *destination,
                    final_status: None,
                    cancelled: false,
                    last_response: None,
                });
            }
            self.forks_by_downstream
                .insert(downstream_tx_id.clone(), fork.clone());

            match self.tm.send_request(&downstream_tx_id).await {
                Ok(()) => {
                    if attempt > 1 {
                        debug!(
                            "proxy: leg candidate {}/{} ({}) succeeded after {} prior failure(s)",
                            attempt,
                            total,
                            destination,
                            attempt - 1
                        );
                    }
                    debug!(
                        "proxy: started leg to {} (upstream tx={} downstream tx={} mode={:?})",
                        destination, fork.upstream_server_tx, downstream_tx_id, fork.mode
                    );
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
                        attempt, total, destination, e
                    );
                    // Drop the leg from the fork map — the next
                    // candidate will register its own.
                    self.forks_by_downstream.remove(&downstream_tx_id);
                    {
                        let mut legs = fork.legs.lock().await;
                        legs.retain(|leg| leg.downstream_client_tx != downstream_tx_id);
                    }
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
                "RFC 3263 §4.3 leg failover exhausted: all {} candidate(s) failed",
                total
            ))
        }))
    }

    /// Aggregator entry for every downstream response. Routes via
    /// RFC 3261 §16.7 step-by-step:
    ///
    /// - 1xx → forward upstream verbatim (Via popped).
    /// - 2xx → forward upstream, CANCEL all other live legs, mark
    ///   `upstream_responded=true` so siblings' arriving finals are
    ///   dropped.
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

        let status = response.status();

        if !is_final {
            // 1xx: forward upstream as-is, leaving the leg open.
            // RFC 3261 §16.8 — Timer C MUST be reset on every 1xx.
            // Only INVITE forks run Timer C in the first place; the
            // reset is a no-op when no timer is active.
            if fork.is_invite {
                self.reset_timer_c(fork.upstream_server_tx.clone());
            }
            return self.forward_to_upstream(&fork, response).await;
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
        // Mark the winner.
        {
            let mut legs = fork.legs.lock().await;
            for leg in legs.iter_mut() {
                if leg.downstream_client_tx == downstream_tx_id {
                    leg.final_status = Some(response.status());
                    leg.last_response = Some(response.clone());
                    break;
                }
            }
        }

        // Forward the 2xx upstream. Only the FIRST 2xx makes it
        // through — siblings arriving later see upstream_responded
        // and bail.
        if fork
            .upstream_responded
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .is_ok()
        {
            self.forward_to_upstream(fork, response).await?;
        }

        // CANCEL all sibling legs that are still pending.
        self.cancel_siblings(fork, &downstream_tx_id).await;
        self.cancel_timer_c(&fork.upstream_server_tx);
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
                                leg.cancelled = true;
                                leg.final_status = Some(response.status());
                                leg.last_response = Some(response.clone());
                            }
                        }
                        // Spawn the requested legs. Treat each
                        // target as a single-candidate leg under
                        // the requested mode.
                        for target in targets {
                            if let Err(e) = self
                                .start_leg(fork, &fork.original_request, &[target])
                                .await
                            {
                                warn!("proxy: 3xx re-fork start_leg to {} failed: {}", target, e);
                            }
                        }
                        // Don't fall through to the failure path —
                        // the 3xx is now consumed.
                        return Ok(());
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

        // Record this leg's final.
        let all_finished;
        let was_cancelled_leg;
        {
            let mut legs = fork.legs.lock().await;
            was_cancelled_leg = legs
                .iter()
                .any(|l| l.downstream_client_tx == downstream_tx_id && l.cancelled);
            for leg in legs.iter_mut() {
                if leg.downstream_client_tx == downstream_tx_id {
                    leg.final_status = Some(response.status());
                    leg.last_response = Some(response.clone());
                }
            }
            all_finished = legs.iter().all(|l| l.final_status.is_some());
        }

        if was_cancelled_leg {
            // A 487 / similar on a leg we CANCELed — expected, don't
            // surface upstream.
            return Ok(());
        }

        match fork.mode {
            ForkMode::Sequential => {
                // Advance to the next leg index. With per-leg failover,
                // each "leg" may have walked multiple candidate addrs
                // internally — tracking the next leg by index instead
                // of by SocketAddr is correct under both single- and
                // multi-candidate shapes.
                let leg_total = if !fork.leg_candidates.is_empty() {
                    fork.leg_candidates.len()
                } else {
                    fork.targets.len()
                };
                let next_idx = fork
                    .legs_started
                    .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                if next_idx < leg_total {
                    let candidates: Vec<SocketAddr> =
                        if let Some(c) = fork.leg_candidates.get(next_idx) {
                            c.clone()
                        } else {
                            fork.targets.get(next_idx).copied().into_iter().collect()
                        };
                    // legs_started was already bumped by fetch_add;
                    // start_leg uses it implicitly via the success
                    // path's existing accounting.
                    self.start_leg(fork, &fork.original_request, &candidates)
                        .await?;
                    return Ok(());
                }
                // Counter overshot — restore so the value stays
                // representative of "next index to start" (== leg_total
                // when exhausted).
                fork.legs_started
                    .store(leg_total, std::sync::atomic::Ordering::Release);
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
        // RFC 3261 §16.7 step 6 — choose the "best" response.
        // Simplified policy: any 6xx wins (global failure); else pick
        // the lowest status code seen. Ties broken by first-seen order
        // (preserves leg-start order).
        let best = {
            let legs = fork.legs.lock().await;
            let any_6xx = legs
                .iter()
                .find(|l| matches!(l.final_status, Some(s) if s.as_u16() / 100 == 6));
            if let Some(leg) = any_6xx {
                leg.last_response.clone()
            } else {
                legs.iter()
                    .filter_map(|l| {
                        l.last_response
                            .clone()
                            .map(|r| (l.final_status.unwrap().as_u16(), r))
                    })
                    .min_by_key(|(code, _)| *code)
                    .map(|(_, r)| r)
            }
        };
        if let Some(response) = best {
            if fork
                .upstream_responded
                .compare_exchange(
                    false,
                    true,
                    std::sync::atomic::Ordering::AcqRel,
                    std::sync::atomic::Ordering::Acquire,
                )
                .is_ok()
            {
                self.forward_to_upstream(fork, response).await?;
            }
        }
        self.cancel_timer_c(&fork.upstream_server_tx);
        Ok(())
    }

    /// Strip the proxy's top Via and forward the response upstream.
    async fn forward_to_upstream(
        &self,
        fork: &Arc<ForkContext>,
        mut response: Response,
    ) -> Result<(), ProxyError> {
        remove_top_via_header(&mut response);
        self.tm
            .send_response(&fork.upstream_server_tx, response)
            .await
            .map_err(|e| ProxyError::Transaction(e.to_string()))
    }

    /// CANCEL every leg in `fork` except `winner`. Marks them as
    /// cancelled so the resulting 487 isn't surfaced upstream.
    async fn cancel_siblings(&self, fork: &Arc<ForkContext>, winner: &TransactionKey) {
        let cancel_targets: Vec<TransactionKey> = {
            let mut legs = fork.legs.lock().await;
            let mut to_cancel = Vec::new();
            for leg in legs.iter_mut() {
                if leg.downstream_client_tx == *winner
                    || leg.final_status.is_some()
                    || leg.cancelled
                {
                    continue;
                }
                leg.cancelled = true;
                to_cancel.push(leg.downstream_client_tx.clone());
            }
            to_cancel
        };
        for tx_id in cancel_targets {
            match self.tm.cancel_invite_transaction(&tx_id).await {
                Ok(_cancel_tx) => {
                    debug!("proxy: CANCEL sent on sibling leg tx={}", tx_id);
                }
                Err(e) => {
                    warn!("proxy: CANCEL failed for tx={}: {}", tx_id, e);
                }
            }
        }
    }

    fn start_timer_c(self: &Arc<Self>, upstream_tx_id: TransactionKey) {
        let me = self.clone();
        let tx_id = upstream_tx_id.clone();
        let duration = self.config.timer_c;
        let handle = tokio::spawn(async move {
            tokio::time::sleep(duration).await;
            warn!(
                "proxy: Timer C fired on stalled INVITE fork — upstream tx={}",
                tx_id
            );
            // CANCEL every still-pending downstream leg, then 408
            // upstream. Treat the fork as fully terminated so any
            // arriving final responses are dropped as cancel-induced.
            if let Some(fork_ref) = me.forks_by_upstream.get(&tx_id) {
                let fork = fork_ref.clone();
                drop(fork_ref);
                me.cancel_siblings(
                    &fork,
                    &TransactionKey::new(String::new(), Method::Invite, false),
                )
                .await;
                if fork
                    .upstream_responded
                    .compare_exchange(
                        false,
                        true,
                        std::sync::atomic::Ordering::AcqRel,
                        std::sync::atomic::Ordering::Acquire,
                    )
                    .is_ok()
                {
                    if let Err(e) = me
                        .respond_locally(&tx_id, &fork.original_request, StatusCode::RequestTimeout)
                        .await
                    {
                        warn!("proxy: Timer C 408 send failed for tx={}: {}", tx_id, e);
                    }
                }
            }
        });
        self.timer_c_tasks.insert(upstream_tx_id, handle);
    }

    fn cancel_timer_c(&self, upstream_tx_id: &TransactionKey) {
        if let Some((_, handle)) = self.timer_c_tasks.remove(upstream_tx_id) {
            handle.abort();
        }
    }

    /// RFC 3261 §16.8 — restart Timer C from zero. Called on every
    /// 1xx so a long-ringing INVITE doesn't time out, while a stalled
    /// one (no 1xx within `timer_c`) still triggers 408 upstream.
    fn reset_timer_c(self: &Arc<Self>, upstream_tx_id: TransactionKey) {
        if self.timer_c_tasks.contains_key(&upstream_tx_id) {
            self.cancel_timer_c(&upstream_tx_id);
            self.start_timer_c(upstream_tx_id);
        }
    }

    /// Best-effort cleanup when a transaction terminates. Either the
    /// upstream server-tx or any downstream client-tx may be the
    /// terminating one; we resolve to the owning fork and reap when
    /// the upstream side is gone.
    async fn cleanup_fork(&self, tx_id: &TransactionKey) {
        if let Some((_, fork)) = self.forks_by_upstream.remove(tx_id) {
            // Upstream is gone — drop all downstream lookups.
            let legs = fork.legs.lock().await;
            for leg in legs.iter() {
                self.forks_by_downstream.remove(&leg.downstream_client_tx);
            }
            drop(legs);
            self.cancel_timer_c(tx_id);
        } else if let Some((_, fork)) = self.forks_by_downstream.remove(tx_id) {
            // A downstream leg terminated — keep the fork alive for
            // any siblings, just prune the dead leg from the maps.
            let _ = &fork; // (no further bookkeeping for now)
        }
    }

    async fn handle_upstream_cancel(&self, upstream_tx_id: &TransactionKey) {
        if let Some(fork_ref) = self.forks_by_upstream.get(upstream_tx_id) {
            let fork = fork_ref.clone();
            drop(fork_ref);
            // No winner — cancel every live leg. Use a sentinel key
            // that doesn't match any real leg so the helper cancels
            // them all.
            let sentinel = TransactionKey::new(String::new(), Method::Invite, false);
            self.cancel_siblings(&fork, &sentinel).await;
        }
    }

    /// Scan every Via entry on `request` and return the first branch
    /// value that matches a branch this proxy has previously stamped.
    /// Returning `Some(branch)` is the RFC 3261 §16.6 step-4
    /// loop-detected condition; the caller responds 482 upstream.
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
        // RFC 3261 §8.2.6.2 — the response must carry From/To/Call-ID/
        // CSeq and the full Via stack from the request. Use the
        // canonical builder so retransmits, routing and downstream
        // validation all stay happy.
        let response = rvoip_sip_core::builder::SimpleResponseBuilder::response_from_request(
            original_request,
            status,
            None,
        )
        .build();
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
        // RFC 3261 §16.6 step 3 — if no Max-Forwards present, add one
        // with value 70. Anything else risks unbounded forwarding.
        request
            .headers
            .push(TypedHeader::MaxForwards(MaxForwards::new(69)));
        Ok(())
    }
}

fn push_proxy_via(
    request: &mut Request,
    local_addr: SocketAddr,
    branch: &str,
) -> Result<(), ProxyError> {
    let transport = transport_token_for_request(request);
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
        response.headers.remove(pos);
    }
}

/// Pick a Via `sent-protocol` transport token (UDP / TCP / TLS / WS /
/// WSS) for a forwarded request. We honour the next-hop URI's
/// `;transport=` parameter / scheme just like the originating UAC,
/// since the proxy's Via is what the downstream uses for symmetric
/// response routing.
fn transport_token_for_request(request: &Request) -> &'static str {
    use rvoip_sip_transport::transport::TransportType;
    let uri = request.uri();
    match rvoip_sip_transport::resolver::select_transport_for_uri(uri) {
        TransportType::Udp => "UDP",
        TransportType::Tcp => "TCP",
        TransportType::Tls => "TLS",
        TransportType::Ws => "WS",
        TransportType::Wss => "WSS",
    }
}
