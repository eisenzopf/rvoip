//! Built-in [`CallHandler`] implementations for common patterns.
//!
//! Use these with [`CallbackPeer`] to handle incoming calls without writing
//! a custom trait implementation.
//!
//! | Handler | Behavior |
//! |---------|----------|
//! | [`AutoAnswerHandler`] | Accepts all calls and allows all transfers |
//! | [`RejectAllHandler`] | Rejects all calls with configurable status/reason |
//! | [`RoutingHandler`] | Routes calls by URI pattern matching |
//! | [`QueueHandler`] | Defers calls into a channel for async processing |
//!
//! [`CallbackPeer`]: crate::api::callback_peer::CallbackPeer
//! [`CallHandler`]: crate::api::callback_peer::CallHandler

use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::api::callback_peer::{CallHandler, CallHandlerDecision};
use crate::api::handle::SessionHandle;
use crate::api::incoming::{IncomingCall, IncomingCallGuard};

// ===== AutoAnswerHandler =====

/// Accepts all incoming calls and allows all transfers.
///
/// # Example
///
/// ```rust,no_run
/// # async fn example() -> anyhow::Result<()> {
/// use rvoip_session_core_v3::{CallbackPeer, Config};
/// use rvoip_session_core_v3::api::handlers::AutoAnswerHandler;
///
/// let peer = CallbackPeer::new(AutoAnswerHandler, Config::default()).await?;
/// peer.run().await?;
/// # Ok(())
/// # }
/// ```
pub struct AutoAnswerHandler;

#[async_trait]
impl CallHandler for AutoAnswerHandler {
    async fn on_incoming_call(&self, _call: IncomingCall) -> CallHandlerDecision {
        // Dispatch applies the decision — no need to consume `call` manually.
        CallHandlerDecision::Accept
    }

    async fn on_transfer_request(&self, _handle: SessionHandle, _target: String) -> bool {
        true
    }
}

// ===== RejectAllHandler =====

/// Rejects all incoming calls with a configurable status code and reason.
///
/// Defaults to `486 Busy Here`.
///
/// # Example
///
/// ```rust,no_run
/// # async fn example() -> anyhow::Result<()> {
/// use rvoip_session_core_v3::{CallbackPeer, Config};
/// use rvoip_session_core_v3::api::handlers::RejectAllHandler;
///
/// let peer = CallbackPeer::new(RejectAllHandler::default(), Config::default()).await?;
/// peer.run().await?;
/// # Ok(())
/// # }
/// ```
pub struct RejectAllHandler {
    pub status: u16,
    pub reason: String,
}

impl Default for RejectAllHandler {
    fn default() -> Self {
        Self {
            status: 486,
            reason: "Busy Here".into(),
        }
    }
}

impl RejectAllHandler {
    /// Create a handler that rejects with a custom status code and reason.
    pub fn new(status: u16, reason: impl Into<String>) -> Self {
        Self {
            status,
            reason: reason.into(),
        }
    }
}

#[async_trait]
impl CallHandler for RejectAllHandler {
    async fn on_incoming_call(&self, _call: IncomingCall) -> CallHandlerDecision {
        CallHandlerDecision::Reject {
            status: self.status,
            reason: self.reason.clone(),
        }
    }
}

// ===== RoutingHandler =====

/// What to do when a routing rule matches.
#[derive(Debug, Clone)]
pub enum RoutingAction {
    /// Accept the call.
    Accept,
    /// Reject with a SIP status code and reason.
    Reject { status: u16, reason: String },
    /// Redirect the caller to another URI (sends 3xx).
    Redirect(String),
}

/// A single routing rule: if the `To` URI contains `pattern`, apply `action`.
#[derive(Debug, Clone)]
pub struct RoutingRule {
    pub pattern: String,
    pub action: RoutingAction,
}

/// Routes incoming calls based on URI pattern matching.
///
/// Rules are evaluated in order; the first match wins. If no rule matches,
/// `default_action` is applied (defaults to `Reject 404 Not Found`).
///
/// # Example
///
/// ```rust,no_run
/// # async fn example() -> anyhow::Result<()> {
/// use rvoip_session_core_v3::{CallbackPeer, Config};
/// use rvoip_session_core_v3::api::handlers::{RoutingHandler, RoutingAction};
///
/// let handler = RoutingHandler::new()
///     .with_rule("support@", RoutingAction::Accept)
///     .with_rule("spam@", RoutingAction::Reject {
///         status: 403,
///         reason: "Forbidden".into(),
///     })
///     .with_default(RoutingAction::Reject {
///         status: 404,
///         reason: "Not Found".into(),
///     });
///
/// let peer = CallbackPeer::new(handler, Config::default()).await?;
/// peer.run().await?;
/// # Ok(())
/// # }
/// ```
pub struct RoutingHandler {
    rules: Vec<RoutingRule>,
    default_action: RoutingAction,
}

impl RoutingHandler {
    /// Create a new routing handler with no rules and a default 404 reject.
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            default_action: RoutingAction::Reject {
                status: 404,
                reason: "Not Found".into(),
            },
        }
    }

    /// Add a routing rule. If the `To` URI contains `pattern`, apply `action`.
    pub fn with_rule(mut self, pattern: impl Into<String>, action: RoutingAction) -> Self {
        self.rules.push(RoutingRule {
            pattern: pattern.into(),
            action,
        });
        self
    }

    /// Set the default action for calls that match no rule.
    pub fn with_default(mut self, action: RoutingAction) -> Self {
        self.default_action = action;
        self
    }
}

impl Default for RoutingHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CallHandler for RoutingHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        let action = self
            .rules
            .iter()
            .find(|r| call.to.contains(&r.pattern))
            .map(|r| &r.action)
            .unwrap_or(&self.default_action);

        match action {
            RoutingAction::Accept => CallHandlerDecision::Accept,
            RoutingAction::Reject { status, reason } => CallHandlerDecision::Reject {
                status: *status,
                reason: reason.clone(),
            },
            RoutingAction::Redirect(target) => CallHandlerDecision::Redirect(target.clone()),
        }
    }
}

// ===== QueueHandler =====

/// Defers incoming calls and sends their guards to a channel for async processing.
///
/// Pair with a consumer task that calls [`IncomingCallGuard::accept()`] or
/// [`IncomingCallGuard::reject()`] when ready. If the channel is full, the
/// call is rejected with `503 Service Unavailable`.
///
/// # Example
///
/// ```rust,no_run
/// # async fn example() -> anyhow::Result<()> {
/// use rvoip_session_core_v3::{CallbackPeer, Config};
/// use rvoip_session_core_v3::api::handlers::QueueHandler;
/// use std::time::Duration;
///
/// let (handler, mut rx) = QueueHandler::new(100, Duration::from_secs(30));
///
/// // Consumer task — resolve queued calls
/// tokio::spawn(async move {
///     while let Some(guard) = rx.recv().await {
///         // Accept each queued call
///         let _ = guard.accept().await;
///     }
/// });
///
/// let peer = CallbackPeer::new(handler, Config::default()).await?;
/// peer.run().await?;
/// # Ok(())
/// # }
/// ```
pub struct QueueHandler {
    tx: mpsc::Sender<IncomingCallGuard>,
    defer_timeout: Duration,
}

impl QueueHandler {
    /// Create a queue handler and its receiving end.
    ///
    /// `buffer` controls the channel capacity.
    /// `defer_timeout` sets how long calls ring before auto-rejecting if not resolved.
    pub fn new(buffer: usize, defer_timeout: Duration) -> (Self, mpsc::Receiver<IncomingCallGuard>) {
        let (tx, rx) = mpsc::channel(buffer);
        (Self { tx, defer_timeout }, rx)
    }
}

#[async_trait]
impl CallHandler for QueueHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        let guard = call.defer(self.defer_timeout);
        match self.tx.send(guard).await {
            Ok(()) => {
                // Guard is in the queue; consumer will resolve it.
                // IncomingCall is already consumed by defer(), so the returned
                // decision is ignored by CallbackPeer::dispatch.
                CallHandlerDecision::Accept
            }
            Err(send_err) => {
                // Channel closed or full — reject the call.
                let guard = send_err.0;
                guard.reject(503, "Service Unavailable");
                CallHandlerDecision::Accept
            }
        }
    }
}
