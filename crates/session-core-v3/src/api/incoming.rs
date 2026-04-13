//! Incoming call handling — accept, reject, redirect, or defer.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::api::handle::{CallId, SessionHandle};
use crate::api::unified::UnifiedCoordinator;
use crate::errors::{Result, SessionError};

/// An incoming SIP INVITE that must be handled.
///
/// Obtain one via [`StreamPeer::wait_for_incoming()`] or via the `on_incoming_call`
/// method of [`CallHandler`].
///
/// The call remains in `Ringing` state until you call one of the resolution
/// methods. Dropping without resolving rejects the call with **486 Busy Here**.
///
/// # Resolution options
///
/// | Method | SIP response | Use for |
/// |--------|-------------|---------|
/// | [`accept()`] | 200 OK | Softphones, immediate answer |
/// | [`reject()`] | 4xx/5xx | Busy, unauthorized, etc. |
/// | [`reject_busy()`] | 486 | Convenience wrapper |
/// | [`reject_decline()`] | 603 | User declined |
/// | [`redirect()`] | 3xx | Proxy / forward-to-voicemail |
/// | [`defer()`] | (hold) | Call center queue |
///
/// [`accept()`]: IncomingCall::accept
/// [`reject()`]: IncomingCall::reject
/// [`reject_busy()`]: IncomingCall::reject_busy
/// [`reject_decline()`]: IncomingCall::reject_decline
/// [`redirect()`]: IncomingCall::redirect
/// [`defer()`]: IncomingCall::defer
pub struct IncomingCall {
    /// The session / call identifier.
    pub call_id: CallId,
    /// SIP From URI (caller).
    pub from: String,
    /// SIP To URI (called party).
    pub to: String,
    /// Remote SDP offer, if present.
    pub sdp: Option<String>,
    /// Additional SIP headers (lower-cased names).
    pub headers: HashMap<String, String>,
    /// When the INVITE arrived.
    pub received_at: Instant,
    /// Internal — coordinator for performing accept/reject.
    pub(crate) coordinator: Arc<UnifiedCoordinator>,
    /// Whether this call has already been resolved (to catch double-resolve).
    resolved: bool,
}

impl IncomingCall {
    pub(crate) fn new(
        call_id: CallId,
        from: String,
        to: String,
        sdp: Option<String>,
        coordinator: Arc<UnifiedCoordinator>,
    ) -> Self {
        Self {
            call_id,
            from,
            to,
            sdp,
            headers: HashMap::new(),
            received_at: Instant::now(),
            coordinator,
            resolved: false,
        }
    }

    // ===== Resolution methods =====

    /// Accept the call and return a [`SessionHandle`] for controlling it.
    ///
    /// Completes SDP negotiation and sends 200 OK.
    pub async fn accept(mut self) -> Result<SessionHandle> {
        self.resolved = true;
        self.coordinator.accept_call(&self.call_id).await?;
        Ok(SessionHandle::new(self.call_id.clone(), self.coordinator.clone()))
    }

    /// Accept the call with a custom SDP answer.
    pub async fn accept_with_sdp(mut self, _sdp: String) -> Result<SessionHandle> {
        // TODO: pass custom SDP through the state machine
        self.resolved = true;
        self.coordinator.accept_call(&self.call_id).await?;
        Ok(SessionHandle::new(self.call_id.clone(), self.coordinator.clone()))
    }

    /// Reject the call immediately with an explicit SIP status code and reason.
    pub fn reject(mut self, _status: u16, reason: &str) {
        self.resolved = true;
        let coordinator = self.coordinator.clone();
        let call_id = self.call_id.clone();
        let reason = reason.to_string();
        tokio::spawn(async move {
            if let Err(e) = coordinator.reject_call(&call_id, &reason).await {
                tracing::warn!("[IncomingCall] reject failed for {}: {}", call_id, e);
            }
        });
    }

    /// Reject with **486 Busy Here**.
    pub fn reject_busy(self) {
        self.reject(486, "Busy Here");
    }

    /// Reject with **603 Decline** (user explicitly declined).
    pub fn reject_decline(self) {
        self.reject(603, "Decline");
    }

    /// Redirect the caller to another URI (sends a 3xx response).
    ///
    /// Note: redirect support requires dialog-core to send a 3xx response; this
    /// currently falls back to a rejection and logs a warning.
    pub fn redirect(self, target: &str) {
        // TODO: implement 3xx support in dialog_adapter
        tracing::warn!(
            "[IncomingCall] redirect to {} not yet fully supported; rejecting with 302",
            target
        );
        self.reject(302, &format!("Moved Temporarily to {}", target));
    }

    /// Defer the accept/reject decision, keeping the call in `Ringing` state
    /// until the returned [`IncomingCallGuard`] is resolved or the `timeout` elapses.
    ///
    /// Use this for call queues: store the guard in a queue data structure and
    /// call `guard.accept()` when an agent becomes available.
    ///
    /// If the guard is dropped without being resolved, the call is rejected with
    /// **503 Service Unavailable**.
    pub fn defer(mut self, timeout: Duration) -> IncomingCallGuard {
        self.resolved = true; // prevent Drop from also rejecting
        IncomingCallGuard::new(self.call_id.clone(), self.coordinator.clone(), timeout)
    }
}

impl Drop for IncomingCall {
    fn drop(&mut self) {
        if !self.resolved {
            let coordinator = self.coordinator.clone();
            let call_id = self.call_id.clone();
            // Auto-reject so the remote party isn't left ringing forever
            tokio::spawn(async move {
                let _ = coordinator.reject_call(&call_id, "Busy Here").await;
            });
        }
    }
}

// ===== IncomingCallGuard =====

/// A deferred incoming call held in `Ringing` state.
///
/// Created by [`IncomingCall::defer()`]. Must be resolved by calling
/// [`accept()`] or [`reject()`] before the deadline, otherwise the call is
/// rejected with **503 Service Unavailable** when the guard is dropped.
///
/// [`accept()`]: IncomingCallGuard::accept
/// [`reject()`]: IncomingCallGuard::reject
pub struct IncomingCallGuard {
    call_id: CallId,
    coordinator: Arc<UnifiedCoordinator>,
    deadline: Instant,
    resolved: bool,
}

impl IncomingCallGuard {
    fn new(call_id: CallId, coordinator: Arc<UnifiedCoordinator>, timeout: Duration) -> Self {
        let deadline = Instant::now() + timeout;

        // Spawn a watchdog that auto-rejects if the deadline passes
        let coordinator_clone = coordinator.clone();
        let call_id_clone = call_id.clone();
        tokio::spawn(async move {
            let remaining = deadline.saturating_duration_since(Instant::now());
            tokio::time::sleep(remaining).await;
            // The coordinator will silently ignore this if the session is already gone
            let _ = coordinator_clone
                .reject_call(&call_id_clone, "Service Unavailable")
                .await;
        });

        Self {
            call_id,
            coordinator,
            deadline,
            resolved: false,
        }
    }

    /// The call identifier for this deferred call (use as queue key).
    pub fn call_id(&self) -> &CallId {
        &self.call_id
    }

    /// When the guard expires and the call is auto-rejected.
    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Accept the call now. Returns a [`SessionHandle`].
    pub async fn accept(mut self) -> Result<SessionHandle> {
        if Instant::now() >= self.deadline {
            return Err(SessionError::Timeout(
                "IncomingCallGuard deadline exceeded before accept".to_string(),
            ));
        }
        self.resolved = true;
        self.coordinator.accept_call(&self.call_id).await?;
        Ok(SessionHandle::new(self.call_id.clone(), self.coordinator.clone()))
    }

    /// Reject the call now.
    pub fn reject(mut self, _status: u16, reason: &str) {
        self.resolved = true;
        let coordinator = self.coordinator.clone();
        let call_id = self.call_id.clone();
        let reason = reason.to_string();
        tokio::spawn(async move {
            let _ = coordinator.reject_call(&call_id, &reason).await;
        });
    }
}

impl Drop for IncomingCallGuard {
    fn drop(&mut self) {
        if !self.resolved {
            let coordinator = self.coordinator.clone();
            let call_id = self.call_id.clone();
            tokio::spawn(async move {
                let _ = coordinator
                    .reject_call(&call_id, "Service Unavailable")
                    .await;
            });
        }
    }
}
