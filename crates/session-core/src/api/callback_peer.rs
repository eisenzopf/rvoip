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
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::api::events::Event;
use crate::api::handle::{CallId, SessionHandle};
use crate::api::incoming::{IncomingCall, IncomingCallGuard};
use crate::api::unified::{Config, Registration, RegistrationHandle, UnifiedCoordinator};
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
    /// Called for every event before the more specific callback hook.
    #[allow(unused_variables)]
    async fn on_event(&self, event: Event) {}

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

    /// Called when a call fails before normal BYE teardown.
    #[allow(unused_variables)]
    async fn on_call_failed(&self, call_id: CallId, status_code: u16, reason: String) {}

    /// Called when a ringing call is cancelled before answer.
    #[allow(unused_variables)]
    async fn on_call_cancelled(&self, call_id: CallId) {}

    /// Called when a DTMF digit is received on an active call.
    #[allow(unused_variables)]
    async fn on_dtmf(&self, handle: SessionHandle, digit: char) {}

    /// Called when a locally requested hold is accepted.
    #[allow(unused_variables)]
    async fn on_call_on_hold(&self, handle: SessionHandle) {}

    /// Called when a locally requested resume is accepted.
    #[allow(unused_variables)]
    async fn on_call_resumed(&self, handle: SessionHandle) {}

    /// Called when the remote peer places this call on hold.
    #[allow(unused_variables)]
    async fn on_remote_call_on_hold(&self, handle: SessionHandle) {}

    /// Called when the remote peer resumes this call.
    #[allow(unused_variables)]
    async fn on_remote_call_resumed(&self, handle: SessionHandle) {}

    /// Called when a REFER (transfer request) is received.
    ///
    /// Return `true` to allow the transfer (send 202 Accepted); `false` to reject.
    #[allow(unused_variables)]
    async fn on_transfer_request(&self, handle: SessionHandle, target: String) -> bool {
        false
    }

    /// Called when an outbound REFER is accepted by the peer.
    #[allow(unused_variables)]
    async fn on_transfer_accepted(&self, handle: SessionHandle, refer_to: String) {}

    /// Called when REFER progress NOTIFY is received.
    #[allow(unused_variables)]
    async fn on_transfer_progress(&self, handle: SessionHandle, status_code: u16, reason: String) {}

    /// Called when REFER completion is received.
    #[allow(unused_variables)]
    async fn on_transfer_completed(
        &self,
        old_call_id: CallId,
        new_call_id: CallId,
        target: String,
    ) {
    }

    /// Called when REFER failure is received.
    #[allow(unused_variables)]
    async fn on_transfer_failed(&self, handle: SessionHandle, status_code: u16, reason: String) {}

    /// Called for inbound NOTIFY requests.
    #[allow(unused_variables)]
    async fn on_notify(
        &self,
        handle: SessionHandle,
        event_package: String,
        subscription_state: Option<String>,
        content_type: Option<String>,
        body: Option<String>,
    ) {
    }

    /// Called when registration succeeds.
    #[allow(unused_variables)]
    async fn on_registration_success(&self, registrar: String, expires: u32, contact: String) {}

    /// Called when registration fails.
    #[allow(unused_variables)]
    async fn on_registration_failed(&self, registrar: String, status_code: u16, reason: String) {}

    /// Called when unregistration succeeds.
    #[allow(unused_variables)]
    async fn on_unregistration_success(&self, registrar: String) {}

    /// Called when unregistration fails.
    #[allow(unused_variables)]
    async fn on_unregistration_failed(&self, registrar: String, reason: String) {}

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

/// Cloneable command handle for a running [`CallbackPeer`].
#[derive(Clone)]
pub struct CallbackPeerControl {
    coordinator: Arc<UnifiedCoordinator>,
    local_uri: String,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
}

impl CallbackPeerControl {
    /// Initiate an outgoing call from this peer's configured local URI.
    pub async fn call(&self, target: &str) -> Result<SessionHandle> {
        let id = self.coordinator.make_call(&self.local_uri, target).await?;
        Ok(SessionHandle::new(id, self.coordinator.clone()))
    }

    /// Initiate an outgoing call with explicit digest-auth credentials.
    pub async fn call_with_auth(
        &self,
        target: &str,
        credentials: crate::types::Credentials,
    ) -> Result<SessionHandle> {
        let id = self
            .coordinator
            .make_call_with_auth(&self.local_uri, target, credentials)
            .await?;
        Ok(SessionHandle::new(id, self.coordinator.clone()))
    }

    /// Register with a SIP server.
    pub async fn register_with(&self, reg: Registration) -> Result<RegistrationHandle> {
        self.coordinator.register_with(reg).await
    }

    /// Query whether a registration handle is currently registered.
    pub async fn is_registered(&self, handle: &RegistrationHandle) -> Result<bool> {
        self.coordinator.is_registered(handle).await
    }

    /// Unregister.
    pub async fn unregister(&self, handle: &RegistrationHandle) -> Result<()> {
        self.coordinator.unregister(handle).await
    }

    /// Hang up or cancel a call and wait until the state machine has accepted
    /// the request.
    pub async fn hangup(&self, handle: &SessionHandle) -> Result<()> {
        self.coordinator.hangup(handle.id()).await
    }

    /// Signal the owning [`CallbackPeer`] event loop to stop.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Access the underlying coordinator for advanced operations.
    pub fn coordinator(&self) -> &Arc<UnifiedCoordinator> {
        &self.coordinator
    }
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
    local_uri: String,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
    established_callbacks: Arc<tokio::sync::Mutex<HashSet<CallId>>>,
    terminal_callbacks: Arc<tokio::sync::Mutex<HashSet<CallId>>>,
    deferred_calls: Arc<tokio::sync::Mutex<HashMap<CallId, IncomingCallGuard>>>,
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
        let local_uri = config.local_uri.clone();
        let coordinator = UnifiedCoordinator::new(config).await?;
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        Ok(Self {
            handler: Arc::new(handler),
            coordinator,
            local_uri,
            shutdown_tx,
            shutdown_rx,
            established_callbacks: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
            terminal_callbacks: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
            deferred_calls: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        })
    }

    /// Access the underlying coordinator for initiating outgoing calls or
    /// performing advanced operations.
    pub fn coordinator(&self) -> &Arc<UnifiedCoordinator> {
        &self.coordinator
    }

    /// Return a cloneable control handle for calls, registration, and shutdown.
    pub fn control(&self) -> CallbackPeerControl {
        CallbackPeerControl {
            coordinator: self.coordinator.clone(),
            local_uri: self.local_uri.clone(),
            shutdown_tx: self.shutdown_tx.clone(),
        }
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
        let established_callbacks = self.established_callbacks.clone();
        let terminal_callbacks = self.terminal_callbacks.clone();
        let deferred_calls = self.deferred_calls.clone();

        handlers.spawn(async move {
            handler.on_event(event.clone()).await;

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
                    let decision = handler.on_incoming_call(incoming).await;
                    // These coordinator calls are idempotent — if the handler
                    // already resolved the call, the session has transitioned
                    // out of Ringing and the call becomes a no-op error we ignore.
                    match decision {
                        CallHandlerDecision::Accept => {
                            match coordinator.accept_call(&call_id).await {
                                Ok(()) => {
                                    let should_notify = {
                                        let mut callbacks = established_callbacks.lock().await;
                                        callbacks.insert(call_id.clone())
                                    };
                                    if should_notify {
                                        let handle =
                                            SessionHandle::new(call_id.clone(), coordinator.clone());
                                        handler.on_call_established(handle).await;
                                    }
                                }
                                Err(e) => {
                                    tracing::debug!(
                                        "Callback accept decision for {} was not applied: {}",
                                        call_id,
                                        e
                                    );
                                }
                            }
                        }
                        CallHandlerDecision::AcceptWithSdp(sdp) => {
                            match coordinator.accept_call_with_sdp(&call_id, sdp).await {
                                Ok(()) => {
                                    let should_notify = {
                                        let mut callbacks = established_callbacks.lock().await;
                                        callbacks.insert(call_id.clone())
                                    };
                                    if should_notify {
                                        let handle =
                                            SessionHandle::new(call_id.clone(), coordinator.clone());
                                        handler.on_call_established(handle).await;
                                    }
                                }
                                Err(e) => {
                                    tracing::debug!(
                                        "Callback accept-with-SDP decision for {} was not applied: {}",
                                        call_id,
                                        e
                                    );
                                }
                            }
                        }
                        CallHandlerDecision::Reject { status, reason } => {
                            let _ = coordinator.reject_call(&call_id, status, &reason).await;
                        }
                        CallHandlerDecision::Redirect(target) => {
                            let _ = coordinator.redirect_call(&call_id, 302, vec![target]).await;
                        }
                        CallHandlerDecision::Defer(guard) => {
                            deferred_calls.lock().await.insert(call_id, guard);
                        }
                    }
                }

                Event::CallAnswered { call_id, .. } => {
                    deferred_calls.lock().await.remove(&call_id);
                    let should_notify = {
                        let mut callbacks = established_callbacks.lock().await;
                        callbacks.insert(call_id.clone())
                    };
                    if should_notify {
                        let handle = SessionHandle::new(call_id, coordinator);
                        handler.on_call_established(handle).await;
                    }
                }

                Event::CallEnded { call_id, reason } => {
                    deferred_calls.lock().await.remove(&call_id);
                    established_callbacks.lock().await.remove(&call_id);
                    let should_notify = {
                        let mut callbacks = terminal_callbacks.lock().await;
                        callbacks.insert(call_id.clone())
                    };
                    if should_notify {
                        let end_reason = EndReason::from(reason);
                        handler.on_call_ended(call_id, end_reason).await;
                    }
                }

                Event::CallFailed {
                    call_id,
                    status_code,
                    reason,
                } => {
                    deferred_calls.lock().await.remove(&call_id);
                    established_callbacks.lock().await.remove(&call_id);
                    let should_notify = {
                        let mut callbacks = terminal_callbacks.lock().await;
                        callbacks.insert(call_id.clone())
                    };
                    if should_notify {
                        handler.on_call_failed(call_id, status_code, reason).await;
                    }
                }

                Event::CallCancelled { call_id } => {
                    deferred_calls.lock().await.remove(&call_id);
                    established_callbacks.lock().await.remove(&call_id);
                    let should_notify = {
                        let mut callbacks = terminal_callbacks.lock().await;
                        callbacks.insert(call_id.clone())
                    };
                    if should_notify {
                        handler.on_call_cancelled(call_id).await;
                    }
                }

                Event::CallOnHold { call_id } => {
                    let handle = SessionHandle::new(call_id, coordinator);
                    handler.on_call_on_hold(handle).await;
                }

                Event::CallResumed { call_id } => {
                    let handle = SessionHandle::new(call_id, coordinator);
                    handler.on_call_resumed(handle).await;
                }

                Event::RemoteCallOnHold { call_id } => {
                    let handle = SessionHandle::new(call_id, coordinator);
                    handler.on_remote_call_on_hold(handle).await;
                }

                Event::RemoteCallResumed { call_id } => {
                    let handle = SessionHandle::new(call_id, coordinator);
                    handler.on_remote_call_resumed(handle).await;
                }

                Event::DtmfReceived { call_id, digit } => {
                    let handle = SessionHandle::new(call_id, coordinator);
                    handler.on_dtmf(handle, digit).await;
                }

                Event::ReferReceived {
                    call_id, refer_to, ..
                } => {
                    let handle = SessionHandle::new(call_id, coordinator);
                    let accepted = handler.on_transfer_request(handle.clone(), refer_to).await;
                    let result = if accepted {
                        handle.accept_refer().await
                    } else {
                        handle.reject_refer(603, "Decline").await
                    };
                    if let Err(e) = result {
                        tracing::warn!("Failed to apply REFER handler decision: {}", e);
                    }
                }

                Event::TransferAccepted { call_id, refer_to } => {
                    let handle = SessionHandle::new(call_id, coordinator);
                    handler.on_transfer_accepted(handle, refer_to).await;
                }

                Event::TransferProgress {
                    call_id,
                    status_code,
                    reason,
                } => {
                    let handle = SessionHandle::new(call_id, coordinator);
                    handler
                        .on_transfer_progress(handle, status_code, reason)
                        .await;
                }

                Event::TransferCompleted {
                    old_call_id,
                    new_call_id,
                    target,
                } => {
                    handler
                        .on_transfer_completed(old_call_id, new_call_id, target)
                        .await;
                }

                Event::TransferFailed {
                    call_id,
                    status_code,
                    reason,
                } => {
                    let handle = SessionHandle::new(call_id, coordinator);
                    handler
                        .on_transfer_failed(handle, status_code, reason)
                        .await;
                }

                Event::NotifyReceived {
                    call_id,
                    event_package,
                    subscription_state,
                    content_type,
                    body,
                } => {
                    let handle = SessionHandle::new(call_id, coordinator);
                    handler
                        .on_notify(
                            handle,
                            event_package,
                            subscription_state,
                            content_type,
                            body,
                        )
                        .await;
                }

                Event::RegistrationSuccess {
                    registrar,
                    expires,
                    contact,
                } => {
                    handler
                        .on_registration_success(registrar, expires, contact)
                        .await;
                }

                Event::RegistrationFailed {
                    registrar,
                    status_code,
                    reason,
                } => {
                    handler
                        .on_registration_failed(registrar, status_code, reason)
                        .await;
                }

                Event::UnregistrationSuccess { registrar } => {
                    handler.on_unregistration_success(registrar).await;
                }

                Event::UnregistrationFailed { registrar, reason } => {
                    handler.on_unregistration_failed(registrar, reason).await;
                }

                Event::CallAuthRetrying {
                    call_id,
                    status_code,
                    realm,
                } => {
                    handler.on_auth_retrying(call_id, status_code, realm).await;
                }

                Event::SessionRefreshed { .. }
                | Event::SessionRefreshFailed { .. }
                | Event::CallMuted { .. }
                | Event::CallUnmuted { .. }
                | Event::MediaQualityChanged { .. }
                | Event::NetworkError { .. }
                | Event::AuthenticationRequired { .. } => {}
            }
        });
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
/// The peer applies that decision after the closure returns.
pub struct ClosureHandler {
    f: Box<dyn Fn(&IncomingCall) -> CallHandlerDecision + Send + Sync>,
}

#[async_trait]
impl CallHandler for ClosureHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        match (self.f)(&call) {
            CallHandlerDecision::Defer(_) => {
                // Defer is not supported for closure handlers because a
                // captured IncomingCallGuard can't escape the &IncomingCall
                // closure signature. Use a trait impl for queue patterns.
                tracing::warn!("[ClosureHandler] Defer decision not supported; rejecting");
                call.reject(503, "Service Unavailable");
                CallHandlerDecision::Reject {
                    status: 503,
                    reason: "Service Unavailable".to_string(),
                }
            }
            decision => decision,
        }
    }
}

impl CallbackPeer<ClosureHandler> {
    /// Create a peer with a closure for handling incoming calls.
    ///
    /// The closure receives a `&IncomingCall` (borrowed) and returns a
    /// [`CallHandlerDecision`]. The peer applies the decision automatically.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_table::types::SessionId;
    use std::sync::Mutex;
    use tokio::task::JoinSet;

    #[derive(Default)]
    struct RecordingHandler {
        events: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingHandler {
        fn push(&self, value: impl Into<String>) {
            self.events.lock().unwrap().push(value.into());
        }
    }

    #[async_trait]
    impl CallHandler for RecordingHandler {
        async fn on_event(&self, _event: Event) {
            self.push("event");
        }

        async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
            self.push("incoming");
            CallHandlerDecision::Defer(call.defer(std::time::Duration::from_millis(10)))
        }

        async fn on_call_established(&self, _handle: SessionHandle) {
            self.push("answered");
        }

        async fn on_call_ended(&self, _call_id: CallId, _reason: EndReason) {
            self.push("ended");
        }

        async fn on_call_failed(&self, _call_id: CallId, status_code: u16, _reason: String) {
            self.push(format!("failed:{status_code}"));
        }

        async fn on_call_cancelled(&self, _call_id: CallId) {
            self.push("cancelled");
        }

        async fn on_dtmf(&self, _handle: SessionHandle, digit: char) {
            self.push(format!("dtmf:{digit}"));
        }

        async fn on_call_on_hold(&self, _handle: SessionHandle) {
            self.push("hold");
        }

        async fn on_call_resumed(&self, _handle: SessionHandle) {
            self.push("resume");
        }

        async fn on_remote_call_on_hold(&self, _handle: SessionHandle) {
            self.push("remote-hold");
        }

        async fn on_remote_call_resumed(&self, _handle: SessionHandle) {
            self.push("remote-resume");
        }

        async fn on_transfer_request(&self, _handle: SessionHandle, _target: String) -> bool {
            self.push("refer");
            false
        }

        async fn on_transfer_accepted(&self, _handle: SessionHandle, _refer_to: String) {
            self.push("transfer-accepted");
        }

        async fn on_transfer_progress(
            &self,
            _handle: SessionHandle,
            status_code: u16,
            _reason: String,
        ) {
            self.push(format!("transfer-progress:{status_code}"));
        }

        async fn on_transfer_completed(
            &self,
            _old_call_id: CallId,
            _new_call_id: CallId,
            _target: String,
        ) {
            self.push("transfer-completed");
        }

        async fn on_transfer_failed(
            &self,
            _handle: SessionHandle,
            status_code: u16,
            _reason: String,
        ) {
            self.push(format!("transfer-failed:{status_code}"));
        }

        async fn on_notify(
            &self,
            _handle: SessionHandle,
            event_package: String,
            _subscription_state: Option<String>,
            _content_type: Option<String>,
            _body: Option<String>,
        ) {
            self.push(format!("notify:{event_package}"));
        }

        async fn on_registration_success(
            &self,
            _registrar: String,
            _expires: u32,
            _contact: String,
        ) {
            self.push("registration-success");
        }

        async fn on_registration_failed(
            &self,
            _registrar: String,
            status_code: u16,
            _reason: String,
        ) {
            self.push(format!("registration-failed:{status_code}"));
        }

        async fn on_unregistration_success(&self, _registrar: String) {
            self.push("unregistration-success");
        }

        async fn on_unregistration_failed(&self, _registrar: String, _reason: String) {
            self.push("unregistration-failed");
        }
    }

    async fn drain(mut handlers: JoinSet<()>) {
        while let Some(result) = handlers.join_next().await {
            result.unwrap();
        }
    }

    #[tokio::test]
    async fn callback_dispatch_invokes_typed_hooks_for_public_events() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let handler = RecordingHandler {
            events: seen.clone(),
        };
        let peer = CallbackPeer::new(handler, Config::local("callback-test", 15440))
            .await
            .unwrap();
        let mut handlers = JoinSet::new();
        let call_id = SessionId::new();
        let failed_call_id = SessionId::new();
        let cancelled_call_id = SessionId::new();

        let events = vec![
            Event::IncomingCall {
                call_id: call_id.clone(),
                from: "sip:a@example.test".into(),
                to: "sip:b@example.test".into(),
                sdp: None,
            },
            Event::CallAnswered {
                call_id: call_id.clone(),
                sdp: None,
            },
            Event::CallAnswered {
                call_id: call_id.clone(),
                sdp: None,
            },
            Event::CallEnded {
                call_id: call_id.clone(),
                reason: "normal".into(),
            },
            Event::CallFailed {
                call_id: failed_call_id.clone(),
                status_code: 486,
                reason: "Busy Here".into(),
            },
            Event::CallCancelled {
                call_id: cancelled_call_id.clone(),
            },
            Event::CallCancelled {
                call_id: cancelled_call_id.clone(),
            },
            Event::CallOnHold {
                call_id: call_id.clone(),
            },
            Event::CallResumed {
                call_id: call_id.clone(),
            },
            Event::RemoteCallOnHold {
                call_id: call_id.clone(),
            },
            Event::RemoteCallResumed {
                call_id: call_id.clone(),
            },
            Event::DtmfReceived {
                call_id: call_id.clone(),
                digit: '5',
            },
            Event::ReferReceived {
                call_id: call_id.clone(),
                refer_to: "sip:c@example.test".into(),
                referred_by: None,
                replaces: None,
                transaction_id: "tx-1".into(),
                transfer_type: "blind".into(),
            },
            Event::TransferAccepted {
                call_id: call_id.clone(),
                refer_to: "sip:c@example.test".into(),
            },
            Event::TransferProgress {
                call_id: call_id.clone(),
                status_code: 180,
                reason: "Ringing".into(),
            },
            Event::TransferCompleted {
                old_call_id: call_id.clone(),
                new_call_id: SessionId::new(),
                target: "sip:c@example.test".into(),
            },
            Event::TransferFailed {
                call_id: call_id.clone(),
                status_code: 503,
                reason: "Service Unavailable".into(),
            },
            Event::NotifyReceived {
                call_id: call_id.clone(),
                event_package: "refer".into(),
                subscription_state: Some("active".into()),
                content_type: Some("message/sipfrag".into()),
                body: Some("SIP/2.0 100 Trying".into()),
            },
            Event::RegistrationSuccess {
                registrar: "sip:registrar.example.test".into(),
                expires: 300,
                contact: "sip:callback-test@example.test".into(),
            },
            Event::RegistrationFailed {
                registrar: "sip:registrar.example.test".into(),
                status_code: 403,
                reason: "Forbidden".into(),
            },
            Event::UnregistrationSuccess {
                registrar: "sip:registrar.example.test".into(),
            },
            Event::UnregistrationFailed {
                registrar: "sip:registrar.example.test".into(),
                reason: "timeout".into(),
            },
        ];

        for event in events {
            peer.dispatch(event, &mut handlers).await;
        }
        drain(handlers).await;

        let seen = seen.lock().unwrap().clone();
        for expected in [
            "incoming",
            "answered",
            "ended",
            "failed:486",
            "cancelled",
            "hold",
            "resume",
            "remote-hold",
            "remote-resume",
            "dtmf:5",
            "refer",
            "transfer-accepted",
            "transfer-progress:180",
            "transfer-completed",
            "transfer-failed:503",
            "notify:refer",
            "registration-success",
            "registration-failed:403",
            "unregistration-success",
            "unregistration-failed",
        ] {
            assert!(
                seen.iter().any(|value| value == expected),
                "missing {expected}"
            );
        }
        assert_eq!(
            seen.iter()
                .filter(|value| value.as_str() == "event")
                .count(),
            22
        );
        assert_eq!(
            seen.iter()
                .filter(|value| value.as_str() == "answered")
                .count(),
            1
        );
        assert_eq!(
            seen.iter()
                .filter(|value| value.as_str() == "cancelled")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn callback_control_can_initiate_calls_while_peer_can_be_moved_to_run() {
        struct NoopHandler;
        #[async_trait]
        impl CallHandler for NoopHandler {
            async fn on_incoming_call(&self, _call: IncomingCall) -> CallHandlerDecision {
                CallHandlerDecision::Reject {
                    status: 486,
                    reason: "Busy Here".into(),
                }
            }
        }

        let peer = CallbackPeer::new(NoopHandler, Config::local("callback-control", 15442))
            .await
            .unwrap();
        let control = peer.control();
        let stop = peer.shutdown_handle();
        let run_task = tokio::spawn(async move { peer.run().await });

        let handle = control
            .call("sip:unreachable@127.0.0.1:15443")
            .await
            .unwrap();
        assert!(!handle.id().to_string().is_empty());

        control.shutdown();
        stop.shutdown();
        tokio::time::timeout(std::time::Duration::from_secs(2), run_task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}
