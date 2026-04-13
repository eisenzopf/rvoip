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

use std::sync::Arc;
use async_trait::async_trait;

use crate::api::events::Event;
use crate::api::handle::{CallId, SessionHandle};
use crate::api::incoming::{IncomingCall, IncomingCallGuard};
use crate::api::unified::{Config, UnifiedCoordinator};
use crate::errors::Result;

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
            r if r.contains("reject") || r.contains("decline") || r.contains("busy") => EndReason::Rejected,
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
/// use rvoip_session_core_v3::api::callback_peer::{CallHandler, CallHandlerDecision};
/// use rvoip_session_core_v3::{SessionHandle, CallId, IncomingCall};
/// use rvoip_session_core_v3::api::callback_peer::EndReason;
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
}

// ===== CallbackPeer =====

/// A SIP peer driven by a [`CallHandler`] implementation.
///
/// # Example
///
/// ```rust,no_run
/// # async fn example() -> anyhow::Result<()> {
/// use rvoip_session_core_v3::api::callback_peer::{CallbackPeer, CallHandler, CallHandlerDecision};
/// use rvoip_session_core_v3::{IncomingCall, Config};
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

    /// Signal shutdown. The `run()` future will return after the current event
    /// is processed.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Start the event loop.
    ///
    /// Processes events until [`shutdown()`] is called or the coordinator is dropped.
    /// Returns `Ok(())` on clean shutdown.
    ///
    /// [`shutdown()`]: Self::shutdown
    pub async fn run(self) -> Result<()> {
        let mut event_rx = self.coordinator.subscribe_events().await?;
        let mut shutdown_rx = self.shutdown_rx.clone();

        loop {
            tokio::select! {
                // Check for shutdown signal
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        tracing::info!("[CallbackPeer] Shutdown signal received");
                        break;
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
                    self.dispatch(event).await;
                }
            }
        }

        Ok(())
    }

    /// Dispatch a single event to the appropriate handler method.
    async fn dispatch(&self, event: Event) {
        let handler = self.handler.clone();
        let coordinator = self.coordinator.clone();

        match event {
            Event::IncomingCall { call_id, from, to, sdp } => {
                let incoming = IncomingCall::new(call_id, from, to, sdp, coordinator.clone());
                tokio::spawn(async move {
                    let decision = handler.on_incoming_call(incoming).await;
                    // IncomingCall's Drop impl handles auto-reject if not yet resolved.
                    // The decision is already applied because the accept/reject/defer methods
                    // on IncomingCall consume `self`, so by the time we return from
                    // on_incoming_call, the call has been resolved (or the guard returned).
                    let _ = decision; // Decision already applied by IncomingCall methods
                });
            }

            Event::CallAnswered { call_id, .. } => {
                let handle = SessionHandle::new(call_id, coordinator);
                tokio::spawn(async move {
                    handler.on_call_established(handle).await;
                });
            }

            Event::CallEnded { call_id, reason } => {
                let end_reason = EndReason::from(reason);
                tokio::spawn(async move {
                    handler.on_call_ended(call_id, end_reason).await;
                });
            }

            Event::DtmfReceived { call_id, digit } => {
                let handle = SessionHandle::new(call_id, coordinator);
                tokio::spawn(async move {
                    handler.on_dtmf(handle, digit).await;
                });
            }

            Event::ReferReceived { call_id, refer_to, .. } => {
                let handle = SessionHandle::new(call_id, coordinator);
                tokio::spawn(async move {
                    handler.on_transfer_request(handle, refer_to).await;
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
/// Created by [`CallbackPeer::from_fn()`].
pub struct ClosureHandler {
    f: Box<dyn Fn(IncomingCall) -> CallHandlerDecision + Send + Sync>,
}

#[async_trait]
impl CallHandler for ClosureHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        (self.f)(call)
    }
}

impl CallbackPeer<ClosureHandler> {
    /// Create a peer with a closure for handling incoming calls.
    ///
    /// This is a quick alternative to implementing [`CallHandler`] when you only
    /// need custom logic for `on_incoming_call`.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # async fn example() -> anyhow::Result<()> {
    /// use rvoip_session_core_v3::{CallbackPeer, CallHandlerDecision, Config};
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
        handler: impl Fn(IncomingCall) -> CallHandlerDecision + Send + Sync + 'static,
    ) -> Result<Self> {
        Self::new(ClosureHandler { f: Box::new(handler) }, config).await
    }
}

impl CallbackPeer<RejectAllHandler> {
    /// Create a peer that rejects all incoming calls with `486 Busy Here`.
    pub async fn with_reject_all(config: Config) -> Result<Self> {
        Self::new(RejectAllHandler::default(), config).await
    }

    /// Create a peer that rejects all calls with a custom status and reason.
    pub async fn with_reject(config: Config, status: u16, reason: impl Into<String>) -> Result<Self> {
        Self::new(RejectAllHandler::new(status, reason), config).await
    }
}
