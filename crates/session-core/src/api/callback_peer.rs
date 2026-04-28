//! CallbackPeer — trait-based reactive SIP peer for servers and proxies.
//!
//! Implement [`CallHandler`] and pass it to [`CallbackPeer::new()`]. Call
//! [`run()`][CallbackPeer::run] to start the event loop; it returns when the peer
//! is shut down.
//!
//! # Use cases
//!
//! - **Proxy server**: `on_incoming_call` makes a fast routing decision and returns
//!   `Accept`, `Reject`, or `Redirect`.
//! - **IVR / call center**: `on_incoming_call` returns `Defer`, storing the
//!   [`IncomingCallGuard`] in a queue until an agent is available.
//! - **B2BUA leg**: `on_call_established` bridges the accepted session to a second
//!   outgoing leg managed in the higher-layer b2bua crate.

use async_trait::async_trait;
use std::sync::Arc;

use crate::api::events::Event;
use crate::api::handle::{CallId, SessionHandle};
use crate::api::incoming::{IncomingCall, IncomingCallGuard};
use crate::api::unified::{Config, UnifiedCoordinator};
use crate::errors::Result;

// ===== ShutdownHandle =====

/// Cloneable handle for stopping a [`CallbackPeer`] from another task.
///
/// Obtained via [`CallbackPeer::shutdown_handle()`] **before** calling
/// [`run()`](CallbackPeer::run).
#[derive(Clone)]
pub struct ShutdownHandle {
    tx: tokio::sync::watch::Sender<bool>,
}

impl ShutdownHandle {
    /// Signal the peer to stop its event loop.
    pub fn shutdown(&self) {
        let _ = self.tx.send(true);
    }

    /// Internal constructor for peers that want to mint a shutdown handle
    /// from an existing watch channel. Scoped to the crate so external
    /// callers go through [`CallbackPeer::shutdown_handle`] /
    /// [`StreamPeer::shutdown_handle`].
    pub(crate) fn from_sender(tx: tokio::sync::watch::Sender<bool>) -> Self {
        Self { tx }
    }
}

// ===== CallHandlerDecision =====

/// How the [`CallHandler`] wants to resolve an incoming call.
pub enum CallHandlerDecision {
    /// Accept immediately, using auto-negotiated SDP.
    Accept,
    /// Accept with a custom SDP answer.
    AcceptWithSdp(String),
    /// Reject immediately with a SIP status code and reason phrase.
    Reject { status: u16, reason: String },
    /// Redirect the caller to another URI (sends 3xx).
    Redirect(String),
    /// Hold the call in `Ringing` state; the framework waits for the guard to resolve.
    Defer(IncomingCallGuard),
}

// ===== EndReason =====

/// Why a call ended.
#[derive(Debug, Clone)]
pub enum EndReason {
    /// Clean BYE exchange.
    Normal,
    /// Remote party rejected or the call was never answered.
    Rejected,
    /// No response within the configured timeout.
    Timeout,
    /// Transport or protocol error.
    NetworkError,
    /// Other reason (check the string for details).
    Other(String),
}

impl From<String> for EndReason {
    fn from(s: String) -> Self {
        match s.to_lowercase().as_str() {
            r if r.contains("timeout") => EndReason::Timeout,
            r if r.contains("reject") || r.contains("decline") || r.contains("busy") => {
                EndReason::Rejected
            }
            r if r.contains("network") || r.contains("transport") => EndReason::NetworkError,
            _ => EndReason::Other(s),
        }
    }
}

// ===== CallHandler trait =====

/// Implement this trait to handle SIP call events reactively.
///
/// All methods are `async`. The library spawns a task for each handler invocation,
/// so you can freely await without blocking other calls.
///
/// # Example — simple accept-all UAS
///
/// ```rust,no_run
/// use rvoip_session_core::api::callback_peer::{CallHandler, CallHandlerDecision};
/// use rvoip_session_core::{SessionHandle, CallId, IncomingCall};
/// use rvoip_session_core::api::callback_peer::EndReason;
/// use async_trait::async_trait;
///
/// struct AcceptAll;
///
/// #[async_trait]
/// impl CallHandler for AcceptAll {
///     async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
///         println!("Incoming call from {}", call.from);
///         CallHandlerDecision::Accept
///     }
///
///     async fn on_call_ended(&self, call_id: CallId, reason: EndReason) {
///         println!("Call {} ended: {:?}", call_id, reason);
///     }
/// }
/// ```
#[async_trait]
pub trait CallHandler: Send + Sync + 'static {
    /// Decide what to do with an incoming call.
    ///
    /// This is the only required method. The call waits in `Ringing` state until
    /// this future returns or the session ringing timeout expires.
    ///
    /// Either:
    /// - Consume the [`IncomingCall`] directly by calling `accept().await`,
    ///   `reject(...)`, `defer(...)`, or `redirect(...)` on it, or
    /// - Return a [`CallHandlerDecision`] and the dispatch will apply it.
    ///
    /// Both paths converge to the same state machine transitions.
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision;

    /// Called when an outgoing or accepted incoming call is fully established.
    #[allow(unused_variables)]
    async fn on_call_established(&self, handle: SessionHandle) {}

    /// Called when any call (incoming or outgoing) ends.
    #[allow(unused_variables)]
    async fn on_call_ended(&self, call_id: CallId, reason: EndReason) {}

    /// Called when a DTMF digit is received on an active call.
    #[allow(unused_variables)]
    async fn on_dtmf(&self, handle: SessionHandle, digit: char) {}

    /// Called when a REFER (transfer request) is received.
    ///
    /// Return `true` to allow the transfer (send 202 Accepted); `false` to reject.
    #[allow(unused_variables)]
    async fn on_transfer_request(&self, handle: SessionHandle, target: String) -> bool {
        false
    }

    /// Called when an outgoing call receives a 401/407 and the coordinator is
    /// about to retry with `Authorization` / `Proxy-Authorization` (RFC 3261
    /// §22.2). Informational — the retry proceeds automatically if credentials
    /// are on file via [`Config.credentials`] or
    /// [`UnifiedCoordinator::make_call_with_auth`]; this hook does not alter
    /// flow. Useful for logging or surfacing auth activity in a UI.
    ///
    /// [`Config.credentials`]: crate::api::unified::Config::credentials
    /// [`UnifiedCoordinator::make_call_with_auth`]: crate::api::unified::UnifiedCoordinator::make_call_with_auth
    #[allow(unused_variables)]
    async fn on_auth_retrying(&self, call_id: CallId, status_code: u16, realm: String) {}
}

// ===== CallbackPeer =====

/// A SIP peer driven by a [`CallHandler`] implementation.
///
/// # Example
///
/// ```rust,no_run
/// # async fn example() -> anyhow::Result<()> {
/// use rvoip_session_core::api::callback_peer::{CallbackPeer, CallHandler, CallHandlerDecision};
/// use rvoip_session_core::{IncomingCall, Config};
/// use async_trait::async_trait;
///
/// struct Router;
///
/// #[async_trait]
/// impl CallHandler for Router {
///     async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
///         if call.from.contains("blocked") {
///             CallHandlerDecision::Reject { status: 403, reason: "Forbidden".into() }
///         } else {
///             CallHandlerDecision::Accept
///         }
///     }
/// }
///
/// let config = Config { sip_port: 5060, ..Default::default() };
/// let peer = CallbackPeer::new(Router, config).await?;
/// peer.run().await?;
/// # Ok(())
/// # }
/// ```
pub struct CallbackPeer<H: CallHandler> {
    handler: Arc<H>,
    coordinator: Arc<UnifiedCoordinator>,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
}

impl<H: CallHandler> CallbackPeer<H> {
    /// Create a new `CallbackPeer`.
    ///
    /// Set `config.credentials` to enable automatic RFC 3261 §22.2 INVITE
    /// digest-auth retry on 401/407 challenges from the server:
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_session_core::Result<()> {
    /// use rvoip_session_core::{CallbackPeer, Config, types::Credentials};
    /// use rvoip_session_core::api::handlers::AutoAnswerHandler;
    ///
    /// let config = Config {
    ///     credentials: Some(Credentials::new("alice", "secret")),
    ///     ..Config::default()
    /// };
    /// let peer = CallbackPeer::new(AutoAnswerHandler, config).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// For per-call overrides, use
    /// [`UnifiedCoordinator::make_call_with_auth`](crate::api::unified::UnifiedCoordinator::make_call_with_auth)
    /// via [`coordinator()`](Self::coordinator).
    pub async fn new(handler: H, config: Config) -> Result<Self> {
        let coordinator = UnifiedCoordinator::new(config).await?;
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        Ok(Self {
            handler: Arc::new(handler),
            coordinator,
            shutdown_tx,
            shutdown_rx,
        })
    }

    /// Access the underlying coordinator for initiating outgoing calls or
    /// performing advanced operations.
    pub fn coordinator(&self) -> &Arc<UnifiedCoordinator> {
        &self.coordinator
    }

    // ===== Registration (symmetric with StreamPeer) =====
    //
    // A CallbackPeer acting as a B2BUA leg may need to register itself
    // upstream (with a carrier / SBC) before or while answering inbound
    // calls. These are thin wrappers over the coordinator — same surface
    // as `StreamPeer::register_with` / `is_registered` / `unregister`.

    /// Register with a SIP server using a [`Registration`] builder.
    pub async fn register_with(
        &self,
        reg: crate::api::unified::Registration,
    ) -> Result<crate::api::unified::RegistrationHandle> {
        self.coordinator.register_with(reg).await
    }

    /// Query whether a registration handle is currently registered.
    ///
    /// Returns `true` once the registrar has replied 200 OK to the REGISTER
    /// (including after a 423 Interval Too Brief retry or 401 auth retry),
    /// and `false` if the registration was rejected, unregistered, or has
    /// not yet completed.
    pub async fn is_registered(
        &self,
        handle: &crate::api::unified::RegistrationHandle,
    ) -> Result<bool> {
        self.coordinator.is_registered(handle).await
    }

    /// Unregister (sends REGISTER with `Expires: 0`).
    pub async fn unregister(&self, handle: &crate::api::unified::RegistrationHandle) -> Result<()> {
        self.coordinator.unregister(handle).await
    }

    /// Signal shutdown. The `run()` future will return after the current event
    /// is processed.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Return a handle that can signal shutdown from another task.
    ///
    /// Obtain this **before** calling [`run()`], which consumes `self`.
    ///
    /// ```rust,no_run
    /// # async fn demo() -> rvoip_session_core::Result<()> {
    /// # use rvoip_session_core::*;
    /// let peer = CallbackPeer::with_auto_answer(Config::default()).await?;
    /// let stop = peer.shutdown_handle();
    /// tokio::spawn(async move { peer.run().await });
    /// // … later …
    /// stop.shutdown();
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`run()`]: Self::run
    pub fn shutdown_handle(&self) -> ShutdownHandle {
        ShutdownHandle {
            tx: self.shutdown_tx.clone(),
        }
    }

    /// Start the event loop.
    ///
    /// Processes events until [`shutdown()`] is called or the coordinator is dropped.
    /// In-flight handler invocations are tracked in a `JoinSet`; on shutdown,
    /// `run()` awaits all pending handlers before returning, so `Ok(())` means
    /// "all user callbacks have observed their final events and returned." This
    /// matters for tests (and b2bua) that tear down and re-create peers and
    /// need to guarantee no user-code is still mutating shared state.
    ///
    /// [`shutdown()`]: Self::shutdown
    pub async fn run(self) -> Result<()> {
        let mut event_rx = self.coordinator.subscribe_events().await?;
        let mut shutdown_rx = self.shutdown_rx.clone();
        let mut handlers: tokio::task::JoinSet<()> = tokio::task::JoinSet::new();

        loop {
            tokio::select! {
                // Check for shutdown signal
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        tracing::info!("[CallbackPeer] Shutdown signal received");
                        break;
                    }
                }
                // Reap completed handlers so the JoinSet doesn't grow
                // unboundedly on a long-lived peer. This branch is only
                // selected when there's at least one pending handler.
                Some(join_result) = handlers.join_next(), if !handlers.is_empty() => {
                    if let Err(e) = join_result {
                        if !e.is_cancelled() {
                            tracing::warn!("[CallbackPeer] Handler task panicked or errored: {}", e);
                        }
                    }
                }
                // Process next event
                raw = event_rx.recv() => {
                    let Some(raw_event) = raw else {
                        tracing::info!("[CallbackPeer] Event channel closed, stopping");
                        break;
                    };

                    // Downcast from cross-crate event wrapper to our Event type
                    let Some(session_event) = raw_event
                        .as_any()
                        .downcast_ref::<crate::adapters::SessionApiCrossCrateEvent>()
                    else {
                        continue;
                    };

                    let event = session_event.event.clone();
                    self.dispatch(event, &mut handlers).await;
                }
            }
        }

        // Wait for all in-flight handler invocations to return before we tear
        // down the coordinator. This is the whole point of the JoinSet: user
        // code should never be interrupted mid-handler by a shutdown.
        while let Some(join_result) = handlers.join_next().await {
            if let Err(e) = join_result {
                if !e.is_cancelled() {
                    tracing::warn!(
                        "[CallbackPeer] Handler task panicked or errored on drain: {}",
                        e
                    );
                }
            }
        }

        // Shut down the coordinator's background tasks too
        self.coordinator.shutdown();
        Ok(())
    }

    /// Dispatch a single event to the appropriate handler method. Each spawn
    /// is tracked in `handlers` so `run()` can drain them on shutdown.
    async fn dispatch(&self, event: Event, handlers: &mut tokio::task::JoinSet<()>) {
        let handler = self.handler.clone();
        let coordinator = self.coordinator.clone();

        match event {
            Event::IncomingCall {
                call_id,
                from,
                to,
                sdp,
            } => {
                // The handler may resolve the call itself (accept/reject/defer)
                // by consuming IncomingCall. If it only returns a decision
                // without consuming the call, the dispatch applies it via the
                // coordinator below. Handler Drop still acts as a safety net.
                let incoming =
                    IncomingCall::new(call_id.clone(), from, to, sdp, coordinator.clone());
                let coord = coordinator.clone();
                let cid = call_id;
                handlers.spawn(async move {
                    let decision = handler.on_incoming_call(incoming).await;
                    // These coordinator calls are idempotent — if the handler
                    // already resolved the call, the session has transitioned
                    // out of Ringing and the call becomes a no-op error we ignore.
                    match decision {
                        CallHandlerDecision::Accept => {
                            let _ = coord.accept_call(&cid).await;
                        }
                        CallHandlerDecision::AcceptWithSdp(sdp) => {
                            let _ = coord.accept_call_with_sdp(&cid, sdp).await;
                        }
                        CallHandlerDecision::Reject { status, reason } => {
                            let _ = coord.reject_call(&cid, status, &reason).await;
                        }
                        CallHandlerDecision::Redirect(target) => {
                            let _ = coord.redirect_call(&cid, 302, vec![target]).await;
                        }
                        CallHandlerDecision::Defer(_guard) => {
                            // Handler kept the guard alive; call stays in Ringing.
                        }
                    }
                });
            }

            Event::CallAnswered { call_id, .. } => {
                let handle = SessionHandle::new(call_id, coordinator);
                handlers.spawn(async move {
                    handler.on_call_established(handle).await;
                });
            }

            Event::CallEnded { call_id, reason } => {
                let end_reason = EndReason::from(reason);
                handlers.spawn(async move {
                    handler.on_call_ended(call_id, end_reason).await;
                });
            }

            Event::DtmfReceived { call_id, digit } => {
                let handle = SessionHandle::new(call_id, coordinator);
                handlers.spawn(async move {
                    handler.on_dtmf(handle, digit).await;
                });
            }

            Event::ReferReceived {
                call_id, refer_to, ..
            } => {
                let handle = SessionHandle::new(call_id, coordinator);
                handlers.spawn(async move {
                    let accepted = handler.on_transfer_request(handle.clone(), refer_to).await;
                    let result = if accepted {
                        handle.accept_refer().await
                    } else {
                        handle.reject_refer(603, "Decline").await
                    };
                    if let Err(e) = result {
                        tracing::warn!("Failed to apply REFER handler decision: {}", e);
                    }
                });
            }

            Event::CallAuthRetrying {
                call_id,
                status_code,
                realm,
            } => {
                handlers.spawn(async move {
                    handler.on_auth_retrying(call_id, status_code, realm).await;
                });
            }

            _ => {
                // Other events (CallFailed, RegistrationSuccess, etc.) are not
                // dispatched to handler methods in this version. Users who need
                // them should use StreamPeer or UnifiedCoordinator directly.
            }
        }
    }
}

// ===== Convenience constructors using built-in handlers =====

use crate::api::handlers::{AutoAnswerHandler, RejectAllHandler};

impl CallbackPeer<AutoAnswerHandler> {
    /// Create a peer that auto-answers all incoming calls and allows transfers.
    pub async fn with_auto_answer(config: Config) -> Result<Self> {
        Self::new(AutoAnswerHandler, config).await
    }
}

// ===== ClosureHandler — use a closure instead of a full trait impl =====

/// A [`CallHandler`] that delegates `on_incoming_call` to a closure.
///
/// Created by [`CallbackPeer::from_fn()`]. The closure receives a borrowed
/// [`IncomingCall`] for inspection and returns a [`CallHandlerDecision`].
/// The decision is applied automatically by the handler (accept/reject/etc.).
pub struct ClosureHandler {
    f: Box<dyn Fn(&IncomingCall) -> CallHandlerDecision + Send + Sync>,
}

#[async_trait]
impl CallHandler for ClosureHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        let decision = (self.f)(&call);
        match &decision {
            CallHandlerDecision::Accept => {
                let _ = call.accept().await;
            }
            CallHandlerDecision::AcceptWithSdp(sdp) => {
                let _ = call.accept_with_sdp(sdp.clone()).await;
            }
            CallHandlerDecision::Reject { status, reason } => {
                call.reject(*status, reason);
            }
            CallHandlerDecision::Redirect(target) => {
                call.redirect(target);
            }
            CallHandlerDecision::Defer(_) => {
                // Defer is not supported for closure handlers because a
                // captured IncomingCallGuard can't escape the &IncomingCall
                // closure signature. Use a trait impl for queue patterns.
                tracing::warn!("[ClosureHandler] Defer decision not supported; rejecting");
                call.reject(503, "Service Unavailable");
            }
        }
        decision
    }
}

impl CallbackPeer<ClosureHandler> {
    /// Create a peer with a closure for handling incoming calls.
    ///
    /// The closure receives a `&IncomingCall` (borrowed) and returns a
    /// [`CallHandlerDecision`]. The handler applies the decision automatically.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # async fn example() -> anyhow::Result<()> {
    /// use rvoip_session_core::{CallbackPeer, CallHandlerDecision, Config};
    ///
    /// let peer = CallbackPeer::from_fn(Config::default(), |call| {
    ///     println!("Call from {}", call.from);
    ///     CallHandlerDecision::Accept
    /// }).await?;
    ///
    /// peer.run().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn from_fn(
        config: Config,
        handler: impl Fn(&IncomingCall) -> CallHandlerDecision + Send + Sync + 'static,
    ) -> Result<Self> {
        Self::new(
            ClosureHandler {
                f: Box::new(handler),
            },
            config,
        )
        .await
    }
}

impl CallbackPeer<RejectAllHandler> {
    /// Create a peer that rejects all incoming calls with `486 Busy Here`.
    pub async fn with_reject_all(config: Config) -> Result<Self> {
        Self::new(RejectAllHandler::default(), config).await
    }

    /// Create a peer that rejects all calls with a custom status and reason.
    pub async fn with_reject(
        config: Config,
        status: u16,
        reason: impl Into<String>,
    ) -> Result<Self> {
        Self::new(RejectAllHandler::new(status, reason), config).await
    }
}
