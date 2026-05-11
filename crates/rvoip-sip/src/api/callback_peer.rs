//! Trait-based reactive peer API for servers, IVR, and routing apps.
//!
//! Implement [`CallHandler`] and pass it to [`CallbackPeer::new()`]. Call
//! [`run()`][CallbackPeer::run] to start the event loop; it returns when the peer
//! is shut down.
//!
//! `CallbackPeer` is built for applications where the library should drive the
//! event loop and call user code when something happens: incoming calls,
//! established calls, DTMF, hold/resume, transfer, NOTIFY, registration, and
//! auth retry notifications. The peer also exposes [`CallbackPeerControl`] so
//! supervisors and handler-owned tasks can place outbound calls, register,
//! inspect registration metadata through the coordinator, and shut down the
//! running peer.
//!
//! # Use cases
//!
//! - **Proxy server**: `on_incoming_call` makes a fast routing decision and returns
//!   `Accept`, `Reject`, or `Redirect`.
//! - **IVR / call center**: `on_incoming_call` returns `Defer`, storing the
//!   [`IncomingCallGuard`] in a queue until an agent is available. Resolving
//!   the guard cancels the deferred-call watchdog.
//! - **B2BUA leg**: `on_call_established` bridges the accepted session to a second
//!   outgoing leg managed in the higher-layer b2bua crate.
//!
//! # Minimal server
//!
//! ```rust,no_run
//! use async_trait::async_trait;
//! use rvoip_sip::{
//!     CallHandler, CallHandlerDecision, CallbackPeer, Config, IncomingCall, Result,
//! };
//!
//! struct Server;
//!
//! #[async_trait]
//! impl CallHandler for Server {
//!     async fn on_incoming_call(&self, _call: IncomingCall) -> CallHandlerDecision {
//!         CallHandlerDecision::Accept
//!     }
//! }
//!
//! # async fn example() -> Result<()> {
//! let peer = CallbackPeer::new(Server, Config::default()).await?;
//! peer.run().await?;
//! # Ok(())
//! # }
//! ```

#![deny(missing_docs)]

use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::api::dialog_package::{DialogInfo, DialogInfoDocument};
use crate::api::events::{
    Event, MediaSecurityState, SipTrace, SubscriptionState, TransferTargetEvidence,
};
use crate::api::handle::{CallId, SessionHandle};
use crate::api::incoming::{IncomingCall, IncomingCallGuard};
use crate::api::unified::{Config, Registration, RegistrationHandle, UnifiedCoordinator};
use crate::errors::{Result, SessionError};

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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(peer: rvoip_sip::CallbackPeer<rvoip_sip::AutoAnswerHandler>) {
    /// let stop = peer.shutdown_handle();
    /// stop.shutdown();
    /// # }
    /// ```
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
    Reject {
        /// SIP final response status code, usually in the 4xx, 5xx, or 6xx range.
        status: u16,
        /// SIP reason phrase to send with the final response.
        reason: String,
    },
    /// Redirect the caller to another URI by sending SIP 302 with a Contact header.
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

type CallbackFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
type EventHook = Arc<dyn Fn(Event) -> CallbackFuture<Result<()>> + Send + Sync>;
type IncomingHook = Arc<dyn Fn(IncomingCall) -> CallbackFuture<CallHandlerDecision> + Send + Sync>;
type EstablishedHook = Arc<dyn Fn(SessionHandle) -> CallbackFuture<Result<()>> + Send + Sync>;
type ProgressHook = Arc<
    dyn Fn(SessionHandle, u16, String, Option<String>) -> CallbackFuture<Result<()>> + Send + Sync,
>;
type DtmfHook = Arc<dyn Fn(SessionHandle, char) -> CallbackFuture<Result<()>> + Send + Sync>;
type EndedHook = Arc<dyn Fn(CallId, EndReason) -> CallbackFuture<Result<()>> + Send + Sync>;
type FailedHook = Arc<dyn Fn(CallId, u16, String) -> CallbackFuture<Result<()>> + Send + Sync>;
type CancelledHook = Arc<dyn Fn(CallId) -> CallbackFuture<Result<()>> + Send + Sync>;
type MediaSecurityHook =
    Arc<dyn Fn(SessionHandle, MediaSecurityState) -> CallbackFuture<Result<()>> + Send + Sync>;
type HoldHook = Arc<dyn Fn(SessionHandle) -> CallbackFuture<Result<()>> + Send + Sync>;
type TransferRequestHook =
    Arc<dyn Fn(SessionHandle, String) -> CallbackFuture<Result<bool>> + Send + Sync>;
type TransferAcceptedHook =
    Arc<dyn Fn(SessionHandle, String) -> CallbackFuture<Result<()>> + Send + Sync>;
type ReferProgressHook =
    Arc<dyn Fn(SessionHandle, u16, String) -> CallbackFuture<Result<()>> + Send + Sync>;
type ReferCompletedHook =
    Arc<dyn Fn(SessionHandle, String, u16, String) -> CallbackFuture<Result<()>> + Send + Sync>;
type TransferFailedHook =
    Arc<dyn Fn(SessionHandle, u16, String) -> CallbackFuture<Result<()>> + Send + Sync>;
type RegistrationSuccessHook =
    Arc<dyn Fn(String, u32, String) -> CallbackFuture<Result<()>> + Send + Sync>;
type RegistrationFailedHook =
    Arc<dyn Fn(String, u16, String) -> CallbackFuture<Result<()>> + Send + Sync>;
type UnregistrationSuccessHook = Arc<dyn Fn(String) -> CallbackFuture<Result<()>> + Send + Sync>;
type UnregistrationFailedHook =
    Arc<dyn Fn(String, String) -> CallbackFuture<Result<()>> + Send + Sync>;
type SipTraceHook = Arc<dyn Fn(SipTrace) -> CallbackFuture<Result<()>> + Send + Sync>;

/// Builder for closure-based [`CallbackPeer`] applications.
///
/// Use this when a full [`CallHandler`] implementation would be noisy but the
/// application still wants typed hooks for common lifecycle events.
pub struct CallbackPeerBuilder {
    config: Config,
    event: Option<EventHook>,
    incoming: Option<IncomingHook>,
    established: Option<EstablishedHook>,
    progress: Option<ProgressHook>,
    dtmf: Option<DtmfHook>,
    ended: Option<EndedHook>,
    failed: Option<FailedHook>,
    cancelled: Option<CancelledHook>,
    media_security: Option<MediaSecurityHook>,
    local_hold: Option<HoldHook>,
    local_resume: Option<HoldHook>,
    remote_hold: Option<HoldHook>,
    remote_resume: Option<HoldHook>,
    transfer_request: Option<TransferRequestHook>,
    transfer_accepted: Option<TransferAcceptedHook>,
    refer_progress: Option<ReferProgressHook>,
    refer_completed: Option<ReferCompletedHook>,
    transfer_failed: Option<TransferFailedHook>,
    registration_success: Option<RegistrationSuccessHook>,
    registration_failed: Option<RegistrationFailedHook>,
    unregistration_success: Option<UnregistrationSuccessHook>,
    unregistration_failed: Option<UnregistrationFailedHook>,
    sip_trace: Option<SipTraceHook>,
}

impl CallbackPeerBuilder {
    /// Create a closure builder for the supplied config.
    pub fn new(config: Config) -> Self {
        Self {
            config,
            event: None,
            incoming: None,
            established: None,
            progress: None,
            dtmf: None,
            ended: None,
            failed: None,
            cancelled: None,
            media_security: None,
            local_hold: None,
            local_resume: None,
            remote_hold: None,
            remote_resume: None,
            transfer_request: None,
            transfer_accepted: None,
            refer_progress: None,
            refer_completed: None,
            transfer_failed: None,
            registration_success: None,
            registration_failed: None,
            unregistration_success: None,
            unregistration_failed: None,
            sip_trace: None,
        }
    }

    /// Handle every public event before more specific typed hooks run.
    pub fn on_event<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(Event) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.event = Some(Arc::new(move |event| Box::pin(f(event))));
        self
    }

    /// Handle incoming calls.
    ///
    /// This hook is required because every inbound INVITE needs an explicit
    /// accept, reject, redirect, or defer decision.
    pub fn on_incoming<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(IncomingCall) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = CallHandlerDecision> + Send + 'static,
    {
        self.incoming = Some(Arc::new(move |call| Box::pin(f(call))));
        self
    }

    /// Handle established calls.
    pub fn on_established<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(SessionHandle) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.established = Some(Arc::new(move |handle| Box::pin(f(handle))));
        self
    }

    /// Handle provisional call progress such as 180 Ringing or 183 Session Progress.
    pub fn on_progress<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(SessionHandle, u16, String, Option<String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.progress = Some(Arc::new(move |handle, status, reason, sdp| {
            Box::pin(f(handle, status, reason, sdp))
        }));
        self
    }

    /// Handle inbound DTMF digits on active calls.
    pub fn on_dtmf<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(SessionHandle, char) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.dtmf = Some(Arc::new(move |handle, digit| Box::pin(f(handle, digit))));
        self
    }

    /// Handle failed calls.
    pub fn on_failed<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(CallId, u16, String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.failed = Some(Arc::new(move |call_id, status, reason| {
            Box::pin(f(call_id, status, reason))
        }));
        self
    }

    /// Handle cancelled ringing calls.
    pub fn on_cancelled<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(CallId) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.cancelled = Some(Arc::new(move |call_id| Box::pin(f(call_id))));
        self
    }

    /// Handle negotiated media security state.
    pub fn on_media_security<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(SessionHandle, MediaSecurityState) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.media_security = Some(Arc::new(move |handle, state| Box::pin(f(handle, state))));
        self
    }

    /// Handle local hold confirmation.
    pub fn on_local_hold<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(SessionHandle) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.local_hold = Some(Arc::new(move |handle| Box::pin(f(handle))));
        self
    }

    /// Handle local resume confirmation.
    pub fn on_local_resume<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(SessionHandle) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.local_resume = Some(Arc::new(move |handle| Box::pin(f(handle))));
        self
    }

    /// Handle remote hold.
    pub fn on_remote_hold<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(SessionHandle) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.remote_hold = Some(Arc::new(move |handle| Box::pin(f(handle))));
        self
    }

    /// Handle remote resume.
    pub fn on_remote_resume<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(SessionHandle) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.remote_resume = Some(Arc::new(move |handle| Box::pin(f(handle))));
        self
    }

    /// Handle call termination.
    pub fn on_ended<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(CallId, EndReason) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.ended = Some(Arc::new(move |call_id, reason| {
            Box::pin(f(call_id, reason))
        }));
        self
    }

    /// Decide whether to accept inbound REFER transfer requests.
    pub fn on_transfer_request<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(SessionHandle, String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<bool>> + Send + 'static,
    {
        self.transfer_request = Some(Arc::new(move |handle, target| Box::pin(f(handle, target))));
        self
    }

    /// Handle an accepted outbound REFER.
    pub fn on_transfer_accepted<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(SessionHandle, String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.transfer_accepted = Some(Arc::new(move |handle, refer_to| {
            Box::pin(f(handle, refer_to))
        }));
        self
    }

    /// Handle provisional REFER progress.
    pub fn on_refer_progress<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(SessionHandle, u16, String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.refer_progress = Some(Arc::new(move |handle, status, reason| {
            Box::pin(f(handle, status, reason))
        }));
        self
    }

    /// Handle successful terminal REFER completion.
    pub fn on_refer_completed<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(SessionHandle, String, u16, String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.refer_completed = Some(Arc::new(move |handle, target, status, reason| {
            Box::pin(f(handle, target, status, reason))
        }));
        self
    }

    /// Handle REFER failure.
    pub fn on_transfer_failed<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(SessionHandle, u16, String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.transfer_failed = Some(Arc::new(move |handle, status, reason| {
            Box::pin(f(handle, status, reason))
        }));
        self
    }

    /// Handle successful registration.
    pub fn on_registration_success<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(String, u32, String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.registration_success = Some(Arc::new(move |registrar, expires, contact| {
            Box::pin(f(registrar, expires, contact))
        }));
        self
    }

    /// Handle failed registration.
    pub fn on_registration_failed<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(String, u16, String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.registration_failed = Some(Arc::new(move |registrar, status, reason| {
            Box::pin(f(registrar, status, reason))
        }));
        self
    }

    /// Handle successful unregistration.
    pub fn on_unregistration_success<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.unregistration_success = Some(Arc::new(move |registrar| Box::pin(f(registrar))));
        self
    }

    /// Handle failed unregistration.
    pub fn on_unregistration_failed<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(String, String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.unregistration_failed = Some(Arc::new(move |registrar, reason| {
            Box::pin(f(registrar, reason))
        }));
        self
    }

    /// Handle SIP transport-boundary trace events when tracing is enabled.
    pub fn on_sip_trace<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(SipTrace) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.sip_trace = Some(Arc::new(move |trace| Box::pin(f(trace))));
        self
    }

    /// Build the [`CallbackPeer`].
    pub async fn build(self) -> Result<CallbackPeer<CallbackBuilderHandler>> {
        let incoming = self.incoming.ok_or_else(|| {
            SessionError::ConfigError(
                "CallbackPeer::builder requires an on_incoming hook".to_string(),
            )
        })?;
        CallbackPeer::new(
            CallbackBuilderHandler {
                event: self.event,
                incoming,
                established: self.established,
                progress: self.progress,
                dtmf: self.dtmf,
                ended: self.ended,
                failed: self.failed,
                cancelled: self.cancelled,
                media_security: self.media_security,
                local_hold: self.local_hold,
                local_resume: self.local_resume,
                remote_hold: self.remote_hold,
                remote_resume: self.remote_resume,
                transfer_request: self.transfer_request,
                transfer_accepted: self.transfer_accepted,
                refer_progress: self.refer_progress,
                refer_completed: self.refer_completed,
                transfer_failed: self.transfer_failed,
                registration_success: self.registration_success,
                registration_failed: self.registration_failed,
                unregistration_success: self.unregistration_success,
                unregistration_failed: self.unregistration_failed,
                sip_trace: self.sip_trace,
            },
            self.config,
        )
        .await
    }
}

/// Internal [`CallHandler`] adapter used by [`CallbackPeerBuilder`].
#[doc(hidden)]
pub struct CallbackBuilderHandler {
    event: Option<EventHook>,
    incoming: IncomingHook,
    established: Option<EstablishedHook>,
    progress: Option<ProgressHook>,
    dtmf: Option<DtmfHook>,
    ended: Option<EndedHook>,
    failed: Option<FailedHook>,
    cancelled: Option<CancelledHook>,
    media_security: Option<MediaSecurityHook>,
    local_hold: Option<HoldHook>,
    local_resume: Option<HoldHook>,
    remote_hold: Option<HoldHook>,
    remote_resume: Option<HoldHook>,
    transfer_request: Option<TransferRequestHook>,
    transfer_accepted: Option<TransferAcceptedHook>,
    refer_progress: Option<ReferProgressHook>,
    refer_completed: Option<ReferCompletedHook>,
    transfer_failed: Option<TransferFailedHook>,
    registration_success: Option<RegistrationSuccessHook>,
    registration_failed: Option<RegistrationFailedHook>,
    unregistration_success: Option<UnregistrationSuccessHook>,
    unregistration_failed: Option<UnregistrationFailedHook>,
    sip_trace: Option<SipTraceHook>,
}

#[async_trait]
impl CallHandler for CallbackBuilderHandler {
    async fn on_event(&self, event: Event) {
        if let Some(hook) = &self.event {
            if let Err(err) = hook(event).await {
                tracing::warn!("[CallbackPeerBuilder] on_event failed: {}", err);
            }
        }
    }

    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        (self.incoming)(call).await
    }

    async fn on_call_established(&self, handle: SessionHandle) {
        if let Some(hook) = &self.established {
            if let Err(err) = hook(handle).await {
                tracing::warn!("[CallbackPeerBuilder] on_established failed: {}", err);
            }
        }
    }

    async fn on_call_progress(
        &self,
        handle: SessionHandle,
        status_code: u16,
        reason: String,
        sdp: Option<String>,
    ) {
        if let Some(hook) = &self.progress {
            if let Err(err) = hook(handle, status_code, reason, sdp).await {
                tracing::warn!("[CallbackPeerBuilder] on_progress failed: {}", err);
            }
        }
    }

    async fn on_call_ended(&self, call_id: CallId, reason: EndReason) {
        if let Some(hook) = &self.ended {
            if let Err(err) = hook(call_id, reason).await {
                tracing::warn!("[CallbackPeerBuilder] on_ended failed: {}", err);
            }
        }
    }

    async fn on_call_failed(&self, call_id: CallId, status_code: u16, reason: String) {
        if let Some(hook) = &self.failed {
            if let Err(err) = hook(call_id, status_code, reason).await {
                tracing::warn!("[CallbackPeerBuilder] on_failed failed: {}", err);
            }
        }
    }

    async fn on_call_cancelled(&self, call_id: CallId) {
        if let Some(hook) = &self.cancelled {
            if let Err(err) = hook(call_id).await {
                tracing::warn!("[CallbackPeerBuilder] on_cancelled failed: {}", err);
            }
        }
    }

    async fn on_dtmf(&self, handle: SessionHandle, digit: char) {
        if let Some(hook) = &self.dtmf {
            if let Err(err) = hook(handle, digit).await {
                tracing::warn!("[CallbackPeerBuilder] on_dtmf failed: {}", err);
            }
        }
    }

    async fn on_media_security_negotiated(&self, handle: SessionHandle, state: MediaSecurityState) {
        if let Some(hook) = &self.media_security {
            if let Err(err) = hook(handle, state).await {
                tracing::warn!("[CallbackPeerBuilder] on_media_security failed: {}", err);
            }
        }
    }

    async fn on_call_on_hold(&self, handle: SessionHandle) {
        if let Some(hook) = &self.local_hold {
            if let Err(err) = hook(handle).await {
                tracing::warn!("[CallbackPeerBuilder] on_local_hold failed: {}", err);
            }
        }
    }

    async fn on_call_resumed(&self, handle: SessionHandle) {
        if let Some(hook) = &self.local_resume {
            if let Err(err) = hook(handle).await {
                tracing::warn!("[CallbackPeerBuilder] on_local_resume failed: {}", err);
            }
        }
    }

    async fn on_remote_call_on_hold(&self, handle: SessionHandle) {
        if let Some(hook) = &self.remote_hold {
            if let Err(err) = hook(handle).await {
                tracing::warn!("[CallbackPeerBuilder] on_remote_hold failed: {}", err);
            }
        }
    }

    async fn on_remote_call_resumed(&self, handle: SessionHandle) {
        if let Some(hook) = &self.remote_resume {
            if let Err(err) = hook(handle).await {
                tracing::warn!("[CallbackPeerBuilder] on_remote_resume failed: {}", err);
            }
        }
    }

    async fn on_transfer_request(&self, handle: SessionHandle, target: String) -> bool {
        let Some(hook) = &self.transfer_request else {
            return false;
        };
        match hook(handle, target).await {
            Ok(accepted) => accepted,
            Err(err) => {
                tracing::warn!(
                    "[CallbackPeerBuilder] on_transfer_request failed; rejecting transfer: {}",
                    err
                );
                false
            }
        }
    }

    async fn on_transfer_accepted(&self, handle: SessionHandle, refer_to: String) {
        if let Some(hook) = &self.transfer_accepted {
            if let Err(err) = hook(handle, refer_to).await {
                tracing::warn!("[CallbackPeerBuilder] on_transfer_accepted failed: {}", err);
            }
        }
    }

    async fn on_refer_progress(&self, handle: SessionHandle, status_code: u16, reason: String) {
        if let Some(hook) = &self.refer_progress {
            if let Err(err) = hook(handle, status_code, reason).await {
                tracing::warn!("[CallbackPeerBuilder] on_refer_progress failed: {}", err);
            }
        }
    }

    async fn on_refer_completed(
        &self,
        handle: SessionHandle,
        target: String,
        status_code: u16,
        reason: String,
    ) {
        if let Some(hook) = &self.refer_completed {
            if let Err(err) = hook(handle, target, status_code, reason).await {
                tracing::warn!("[CallbackPeerBuilder] on_refer_completed failed: {}", err);
            }
        }
    }

    async fn on_transfer_failed(&self, handle: SessionHandle, status_code: u16, reason: String) {
        if let Some(hook) = &self.transfer_failed {
            if let Err(err) = hook(handle, status_code, reason).await {
                tracing::warn!("[CallbackPeerBuilder] on_transfer_failed failed: {}", err);
            }
        }
    }

    async fn on_registration_success(&self, registrar: String, expires: u32, contact: String) {
        if let Some(hook) = &self.registration_success {
            if let Err(err) = hook(registrar, expires, contact).await {
                tracing::warn!(
                    "[CallbackPeerBuilder] on_registration_success failed: {}",
                    err
                );
            }
        }
    }

    async fn on_registration_failed(&self, registrar: String, status_code: u16, reason: String) {
        if let Some(hook) = &self.registration_failed {
            if let Err(err) = hook(registrar, status_code, reason).await {
                tracing::warn!(
                    "[CallbackPeerBuilder] on_registration_failed failed: {}",
                    err
                );
            }
        }
    }

    async fn on_unregistration_success(&self, registrar: String) {
        if let Some(hook) = &self.unregistration_success {
            if let Err(err) = hook(registrar).await {
                tracing::warn!(
                    "[CallbackPeerBuilder] on_unregistration_success failed: {}",
                    err
                );
            }
        }
    }

    async fn on_unregistration_failed(&self, registrar: String, reason: String) {
        if let Some(hook) = &self.unregistration_failed {
            if let Err(err) = hook(registrar, reason).await {
                tracing::warn!(
                    "[CallbackPeerBuilder] on_unregistration_failed failed: {}",
                    err
                );
            }
        }
    }

    async fn on_sip_trace(&self, trace: SipTrace) {
        if let Some(hook) = &self.sip_trace {
            if let Err(err) = hook(trace).await {
                tracing::warn!("[CallbackPeerBuilder] on_sip_trace failed: {}", err);
            }
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
/// use rvoip_sip::api::callback_peer::{CallHandler, CallHandlerDecision};
/// use rvoip_sip::{SessionHandle, CallId, IncomingCall};
/// use rvoip_sip::api::callback_peer::EndReason;
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

    /// Called when an outgoing call receives a provisional 1xx response.
    ///
    /// This surfaces SIP progress such as `180 Ringing` and
    /// `183 Session Progress` without requiring callback applications to
    /// inspect the catch-all [`on_event`](Self::on_event) hook.
    #[allow(unused_variables)]
    async fn on_call_progress(
        &self,
        handle: SessionHandle,
        status_code: u16,
        reason: String,
        sdp: Option<String>,
    ) {
    }

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

    /// Called when SRTP media security has been negotiated and contexts are installed.
    ///
    /// The state is typed and intentionally omits key material.
    #[allow(unused_variables)]
    async fn on_media_security_negotiated(&self, handle: SessionHandle, state: MediaSecurityState) {
    }

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

    /// Called when raw REFER NOTIFY status is received.
    #[allow(unused_variables)]
    async fn on_refer_notify(
        &self,
        handle: SessionHandle,
        status_code: u16,
        reason: String,
        subscription_state: Option<SubscriptionState>,
        body: Option<String>,
    ) {
    }

    /// Called when provisional REFER progress NOTIFY is received.
    #[allow(unused_variables)]
    async fn on_refer_progress(&self, handle: SessionHandle, status_code: u16, reason: String) {}

    /// Called when successful terminal REFER completion is received.
    #[allow(unused_variables)]
    async fn on_refer_completed(
        &self,
        handle: SessionHandle,
        target: String,
        status_code: u16,
        reason: String,
    ) {
    }

    /// Called when REFER failure is received.
    #[allow(unused_variables)]
    async fn on_transfer_failed(&self, handle: SessionHandle, status_code: u16, reason: String) {}

    /// Called when rvoip-sip has target-answer evidence for a transfer.
    #[allow(unused_variables)]
    async fn on_transfer_target_answered(
        &self,
        handle: SessionHandle,
        target_uri: String,
        evidence: TransferTargetEvidence,
    ) {
    }

    /// Called when RFC 4235 observes a candidate replacement dialog.
    #[allow(unused_variables)]
    async fn on_transfer_replacement_dialog_observed(
        &self,
        handle: SessionHandle,
        dialog: DialogInfo,
    ) {
    }

    /// Called when RFC 4235 or local evidence observes replacement dialog teardown.
    #[allow(unused_variables)]
    async fn on_transfer_replacement_dialog_terminated(
        &self,
        handle: SessionHandle,
        dialog: DialogInfo,
        reason: Option<String>,
    ) {
    }

    /// Called for every valid RFC 4235 dialog-package NOTIFY.
    #[allow(unused_variables)]
    async fn on_dialog_package_notify(
        &self,
        subscription_id: CallId,
        entity: Option<String>,
        version: Option<u32>,
        dialogs: Vec<DialogInfo>,
        document: DialogInfoDocument,
    ) {
    }

    /// Called for each parsed RFC 4235 dialog entry transition.
    #[allow(unused_variables)]
    async fn on_dialog_state_changed(&self, subscription_id: CallId, dialog: DialogInfo) {}

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

    /// Called for each SIP trace event when tracing is enabled.
    #[allow(unused_variables)]
    async fn on_sip_trace(&self, trace: SipTrace) {}

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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(control: rvoip_sip::CallbackPeerControl) -> rvoip_sip::Result<()> {
    /// let call = control.call("sip:bob@example.com").await?;
    /// # let _ = call;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn call(&self, target: &str) -> Result<SessionHandle> {
        let id = self.coordinator.make_call(&self.local_uri, target).await?;
        Ok(SessionHandle::new(id, self.coordinator.clone()))
    }

    /// Initiate an outgoing call with explicit digest-auth credentials.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(control: rvoip_sip::CallbackPeerControl) -> rvoip_sip::Result<()> {
    /// let call = control.call_with_auth(
    ///     "sip:bob@example.com",
    ///     rvoip_sip::types::Credentials::new("alice", "secret"),
    /// ).await?;
    /// # let _ = call;
    /// # Ok(())
    /// # }
    /// ```
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
    ///
    /// Successful registration uses the same lifecycle as
    /// [`UnifiedCoordinator::register_with`]: accepted expiry and refresh
    /// timing are stored, automatic refresh may be scheduled, and the handler
    /// receives `on_registration_success`.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(control: rvoip_sip::CallbackPeerControl) -> rvoip_sip::Result<()> {
    /// let handle = control.register_with(
    ///     rvoip_sip::Registration::new("sip:registrar.example.com", "alice", "secret")
    /// ).await?;
    /// # let _ = handle;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn register_with(&self, reg: Registration) -> Result<RegistrationHandle> {
        self.coordinator.register_with(reg).await
    }

    /// Query whether a registration handle is currently registered.
    ///
    /// This is a coarse boolean. Use
    /// [`coordinator`](Self::coordinator) and
    /// [`UnifiedCoordinator::registration_info`] for lifecycle metadata.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(control: rvoip_sip::CallbackPeerControl, handle: rvoip_sip::RegistrationHandle) -> rvoip_sip::Result<()> {
    /// let active = control.is_registered(&handle).await?;
    /// # let _ = active;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn is_registered(&self, handle: &RegistrationHandle) -> Result<bool> {
        self.coordinator.is_registered(handle).await
    }

    /// Unregister.
    ///
    /// Returns after the state machine accepts the unregister request. Use
    /// [`UnifiedCoordinator::unregister_and_wait`] through
    /// [`coordinator`](Self::coordinator) for deterministic registrar
    /// confirmation.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(control: rvoip_sip::CallbackPeerControl, handle: rvoip_sip::RegistrationHandle) -> rvoip_sip::Result<()> {
    /// control.unregister(&handle).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn unregister(&self, handle: &RegistrationHandle) -> Result<()> {
        self.coordinator.unregister(handle).await
    }

    /// Hang up or cancel a call and wait until rvoip-sip has accepted the
    /// request.
    ///
    /// This uses the same SIP teardown semantics as [`SessionHandle::hangup`]:
    /// BYE for established calls, CANCEL for ringing/early outbound calls, and
    /// delayed CANCEL intent before the first provisional response. Callback
    /// handlers observe terminal completion through `on_call_ended`,
    /// `on_call_failed`, or `on_call_cancelled`.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(control: rvoip_sip::CallbackPeerControl, call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// control.hangup(&call).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn hangup(&self, handle: &SessionHandle) -> Result<()> {
        self.coordinator.hangup(handle.id()).await
    }

    /// Signal the owning [`CallbackPeer`] event loop to stop.
    ///
    /// This is a stop signal for the event loop. For deterministic graceful
    /// unregister, call [`UnifiedCoordinator::shutdown_gracefully`] through
    /// [`coordinator`](Self::coordinator).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # fn example(control: rvoip_sip::CallbackPeerControl) {
    /// control.shutdown();
    /// # }
    /// ```
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Access the underlying coordinator for advanced operations.
    ///
    /// This accessor is intentionally trivial and returns the shared
    /// [`UnifiedCoordinator`] handle.
    pub fn coordinator(&self) -> &Arc<UnifiedCoordinator> {
        &self.coordinator
    }
}

// ===== CallbackPeer =====

/// A SIP peer driven by a [`CallHandler`] implementation.
///
/// `CallbackPeer` subscribes to the coordinator event stream, dispatches each
/// event to the relevant handler hook, and tracks in-flight hook tasks during
/// shutdown. Incoming-call decisions can either be returned as
/// [`CallHandlerDecision`] values or performed directly on the supplied
/// [`IncomingCall`].
///
/// The peer consumes itself in [`run`](Self::run). Obtain
/// [`shutdown_handle`](Self::shutdown_handle) or [`control`](Self::control)
/// before calling `run` if another task must stop it or place outbound calls.
///
/// # Example
///
/// ```rust,no_run
/// # async fn example() -> anyhow::Result<()> {
/// use rvoip_sip::api::callback_peer::{CallbackPeer, CallHandler, CallHandlerDecision};
/// use rvoip_sip::{IncomingCall, Config};
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

impl CallbackPeer<CallbackBuilderHandler> {
    /// Create a closure-based [`CallbackPeerBuilder`].
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_sip::Result<()> {
    /// use rvoip_sip::{CallHandlerDecision, CallbackPeer, Config};
    ///
    /// let peer = CallbackPeer::builder(Config::default())
    ///     .on_incoming(|_call| async move { CallHandlerDecision::Accept })
    ///     .build()
    ///     .await?;
    /// # let _ = peer;
    /// # Ok(())
    /// # }
    /// ```
    pub fn builder(config: Config) -> CallbackPeerBuilder {
        CallbackPeerBuilder::new(config)
    }
}

impl<H: CallHandler> CallbackPeer<H> {
    /// Create a new `CallbackPeer`.
    ///
    /// Set `config.credentials` to enable automatic RFC 3261 §22.2 INVITE
    /// digest-auth retry on 401/407 challenges from the server:
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_sip::Result<()> {
    /// use rvoip_sip::{CallbackPeer, Config, types::Credentials};
    /// use rvoip_sip::api::handlers::AutoAnswerHandler;
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

    /// Access the underlying coordinator for advanced operations.
    ///
    /// Prefer [`control`](Self::control) for normal outbound calls and
    /// registration from outside the event loop. Use the coordinator directly
    /// when you need lower-level methods such as media bridging,
    /// per-session event receivers, or custom transfer orchestration.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(peer: rvoip_sip::CallbackPeer<rvoip_sip::AutoAnswerHandler>) {
    /// let coordinator = peer.coordinator().clone();
    /// let mut events = coordinator.events().await.unwrap();
    /// # let _ = events.next().await;
    /// # }
    /// ```
    pub fn coordinator(&self) -> &Arc<UnifiedCoordinator> {
        &self.coordinator
    }

    /// Return a cloneable control handle for calls, registration, and shutdown.
    ///
    /// This is the ergonomic way to place outbound calls or unregister while
    /// the peer is running in another task.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(peer: rvoip_sip::CallbackPeer<rvoip_sip::AutoAnswerHandler>) -> rvoip_sip::Result<()> {
    /// let control = peer.control();
    /// tokio::spawn(async move { peer.run().await });
    /// let call = control.call("sip:bob@example.com").await?;
    /// # let _ = call;
    /// # Ok(())
    /// # }
    /// ```
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
    // calls. These are thin wrappers over the coordinator; use
    // `coordinator()` for metadata and deterministic wait helpers.

    /// Register with a SIP server using a [`Registration`] builder.
    ///
    /// Registration success/failure is surfaced through the corresponding
    /// [`CallHandler`] hooks and through the coordinator event stream.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(peer: rvoip_sip::CallbackPeer<rvoip_sip::AutoAnswerHandler>) -> rvoip_sip::Result<()> {
    /// let handle = peer.register_with(
    ///     rvoip_sip::Registration::new("sip:registrar.example.com", "alice", "secret")
    /// ).await?;
    /// # let _ = handle;
    /// # Ok(())
    /// # }
    /// ```
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
    /// not yet completed. Use [`coordinator`](Self::coordinator) and
    /// [`UnifiedCoordinator::registration_info`] for richer lifecycle details.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(peer: rvoip_sip::CallbackPeer<rvoip_sip::AutoAnswerHandler>, handle: rvoip_sip::RegistrationHandle) -> rvoip_sip::Result<()> {
    /// let active = peer.is_registered(&handle).await?;
    /// # let _ = active;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn is_registered(
        &self,
        handle: &crate::api::unified::RegistrationHandle,
    ) -> Result<bool> {
        self.coordinator.is_registered(handle).await
    }

    /// Unregister (sends REGISTER with `Expires: 0`).
    ///
    /// Returns after the state machine accepts the request. Use
    /// [`UnifiedCoordinator::unregister_and_wait`] through
    /// [`coordinator`](Self::coordinator) when the caller needs to wait for
    /// the registrar response.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(peer: rvoip_sip::CallbackPeer<rvoip_sip::AutoAnswerHandler>, handle: rvoip_sip::RegistrationHandle) -> rvoip_sip::Result<()> {
    /// peer.unregister(&handle).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn unregister(&self, handle: &crate::api::unified::RegistrationHandle) -> Result<()> {
        self.coordinator.unregister(handle).await
    }

    /// Signal shutdown. The `run()` future will return after the current event
    /// is processed.
    ///
    /// This does not wait for unregister. For deterministic unregister before
    /// stopping, call [`UnifiedCoordinator::shutdown_gracefully`] through
    /// [`coordinator`](Self::coordinator).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # fn example(peer: rvoip_sip::CallbackPeer<rvoip_sip::AutoAnswerHandler>) {
    /// peer.shutdown();
    /// # }
    /// ```
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Return a handle that can signal shutdown from another task.
    ///
    /// Obtain this **before** calling [`run()`], which consumes `self`.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn demo() -> rvoip_sip::Result<()> {
    /// # use rvoip_sip::*;
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
    /// After draining handlers, `run()` signals coordinator shutdown. That
    /// shutdown path performs best-effort unregister when configured. Callers
    /// that need deterministic unregister completion should invoke
    /// [`UnifiedCoordinator::shutdown_gracefully`] before or instead of
    /// signalling this event loop.
    ///
    /// Deferred incoming calls that are still unresolved when shutdown begins
    /// are marked resolved before the coordinator is stopped. This prevents the
    /// deferred guard's drop safety net from attempting a late rejection over a
    /// closing transport. Applications that need to reject queued calls should
    /// resolve them before calling shutdown.
    ///
    /// [`shutdown()`]: Self::shutdown
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(peer: rvoip_sip::CallbackPeer<rvoip_sip::AutoAnswerHandler>) -> rvoip_sip::Result<()> {
    /// let stop = peer.shutdown_handle();
    /// tokio::spawn(async move { peer.run().await });
    /// stop.shutdown();
    /// # Ok(())
    /// # }
    /// ```
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

        {
            let mut deferred = self.deferred_calls.lock().await;
            for guard in deferred.values() {
                guard.resolve_without_response();
            }
            deferred.clear();
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
                    if let Some(guard) = deferred_calls.lock().await.remove(&call_id) {
                        guard.resolve_without_response();
                    }
                    let should_notify = {
                        let mut callbacks = established_callbacks.lock().await;
                        callbacks.insert(call_id.clone())
                    };
                    if should_notify {
                        let handle = SessionHandle::new(call_id, coordinator);
                        handler.on_call_established(handle).await;
                    }
                }

                Event::CallProgress {
                    call_id,
                    status_code,
                    reason,
                    sdp,
                } => {
                    let handle = SessionHandle::new(call_id, coordinator);
                    handler
                        .on_call_progress(handle, status_code, reason, sdp)
                        .await;
                }

                Event::CallEnded { call_id, reason } => {
                    if let Some(guard) = deferred_calls.lock().await.remove(&call_id) {
                        guard.resolve_without_response();
                    }
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
                    if let Some(guard) = deferred_calls.lock().await.remove(&call_id) {
                        guard.resolve_without_response();
                    }
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
                    if let Some(guard) = deferred_calls.lock().await.remove(&call_id) {
                        guard.resolve_without_response();
                    }
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

                Event::MediaSecurityNegotiated {
                    call_id,
                    keying,
                    suite,
                    profile,
                    contexts_installed,
                } => {
                    let handle = SessionHandle::new(call_id, coordinator);
                    handler
                        .on_media_security_negotiated(
                            handle,
                            MediaSecurityState {
                                keying,
                                suite,
                                profile,
                                contexts_installed,
                            },
                        )
                        .await;
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

                Event::ReferNotify {
                    call_id,
                    status_code,
                    reason,
                    subscription_state,
                    body,
                } => {
                    let handle = SessionHandle::new(call_id, coordinator);
                    handler
                        .on_refer_notify(handle, status_code, reason, subscription_state, body)
                        .await;
                }

                Event::ReferProgress {
                    call_id,
                    status_code,
                    reason,
                } => {
                    let handle = SessionHandle::new(call_id, coordinator);
                    handler.on_refer_progress(handle, status_code, reason).await;
                }

                Event::ReferCompleted {
                    call_id,
                    target,
                    status_code,
                    reason,
                } => {
                    let handle = SessionHandle::new(call_id, coordinator);
                    handler
                        .on_refer_completed(handle, target, status_code, reason)
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

                Event::TransferTargetAnswered {
                    transfer_call_id,
                    target_uri,
                    evidence,
                } => {
                    let handle = SessionHandle::new(transfer_call_id, coordinator);
                    handler
                        .on_transfer_target_answered(handle, target_uri, evidence)
                        .await;
                }

                Event::TransferReplacementDialogObserved {
                    transfer_call_id,
                    dialog,
                } => {
                    let handle = SessionHandle::new(transfer_call_id, coordinator);
                    handler
                        .on_transfer_replacement_dialog_observed(handle, dialog)
                        .await;
                }

                Event::TransferReplacementDialogTerminated {
                    transfer_call_id,
                    dialog,
                    reason,
                } => {
                    let handle = SessionHandle::new(transfer_call_id, coordinator);
                    handler
                        .on_transfer_replacement_dialog_terminated(handle, dialog, reason)
                        .await;
                }

                Event::DialogPackageNotify {
                    subscription_id,
                    entity,
                    version,
                    dialogs,
                    document,
                } => {
                    handler
                        .on_dialog_package_notify(subscription_id, entity, version, dialogs, document)
                        .await;
                }

                Event::DialogStateChanged {
                    subscription_id,
                    dialog,
                } => {
                    handler
                        .on_dialog_state_changed(subscription_id, dialog)
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

                Event::SipTrace(trace) => {
                    handler.on_sip_trace(trace).await;
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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_sip::Result<()> {
    /// let peer = rvoip_sip::CallbackPeer::with_auto_answer(
    ///     rvoip_sip::Config::default(),
    /// ).await?;
    /// let stop = peer.shutdown_handle();
    /// tokio::spawn(async move { peer.run().await });
    /// stop.shutdown();
    /// # Ok(())
    /// # }
    /// ```
    pub async fn with_auto_answer(config: Config) -> Result<Self> {
        Self::new(AutoAnswerHandler, config).await
    }
}

// ===== ClosureHandler — use a closure instead of a full trait impl =====

/// A [`CallHandler`] that delegates `on_incoming_call` to a closure.
///
/// Created by [`CallbackPeer::from_fn()`]. The closure receives a borrowed
/// [`IncomingCall`] for inspection and returns a [`CallHandlerDecision`].
/// The peer applies that decision after the closure returns. Use
/// [`CallbackPeer::builder`] for async closure hooks, DTMF, ended, transfer,
/// or defer flows that need to own the [`IncomingCall`].
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
    /// use rvoip_sip::{CallbackPeer, CallHandlerDecision, Config};
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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_sip::Result<()> {
    /// let peer = rvoip_sip::CallbackPeer::with_reject_all(
    ///     rvoip_sip::Config::default(),
    /// ).await?;
    /// let stop = peer.shutdown_handle();
    /// tokio::spawn(async move { peer.run().await });
    /// stop.shutdown();
    /// # Ok(())
    /// # }
    /// ```
    pub async fn with_reject_all(config: Config) -> Result<Self> {
        Self::new(RejectAllHandler::default(), config).await
    }

    /// Create a peer that rejects all calls with a custom status and reason.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_sip::Result<()> {
    /// let peer = rvoip_sip::CallbackPeer::with_reject(
    ///     rvoip_sip::Config::default(),
    ///     603,
    ///     "Decline",
    /// ).await?;
    /// # let _ = peer;
    /// # Ok(())
    /// # }
    /// ```
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
    use crate::api::events::{MediaSecurityKeying, MediaSecurityProfile};
    use crate::state_table::types::SessionId;
    use rvoip_sip_core::types::sdp::CryptoSuite;
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

        async fn on_call_progress(
            &self,
            _handle: SessionHandle,
            status_code: u16,
            _reason: String,
            _sdp: Option<String>,
        ) {
            self.push(format!("progress:{status_code}"));
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

        async fn on_media_security_negotiated(
            &self,
            _handle: SessionHandle,
            state: MediaSecurityState,
        ) {
            self.push(format!("media-security:{}", state.contexts_installed));
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

        async fn on_refer_notify(
            &self,
            _handle: SessionHandle,
            status_code: u16,
            _reason: String,
            _subscription_state: Option<SubscriptionState>,
            _body: Option<String>,
        ) {
            self.push(format!("refer-notify:{status_code}"));
        }

        async fn on_refer_progress(
            &self,
            _handle: SessionHandle,
            status_code: u16,
            _reason: String,
        ) {
            self.push(format!("refer-progress:{status_code}"));
        }

        async fn on_refer_completed(
            &self,
            _handle: SessionHandle,
            _target: String,
            status_code: u16,
            _reason: String,
        ) {
            self.push(format!("refer-completed:{status_code}"));
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
            Event::CallProgress {
                call_id: call_id.clone(),
                status_code: 180,
                reason: "Ringing".into(),
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
            Event::MediaSecurityNegotiated {
                call_id: call_id.clone(),
                keying: MediaSecurityKeying::Sdes,
                suite: CryptoSuite::AesCm128HmacSha1_80,
                profile: MediaSecurityProfile::RtpSavp,
                contexts_installed: true,
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
            Event::ReferNotify {
                call_id: call_id.clone(),
                status_code: 100,
                reason: "Trying".into(),
                subscription_state: Some(SubscriptionState::parse("active;expires=60")),
                body: Some("SIP/2.0 100 Trying".into()),
            },
            Event::ReferProgress {
                call_id: call_id.clone(),
                status_code: 180,
                reason: "Ringing".into(),
            },
            Event::ReferCompleted {
                call_id: call_id.clone(),
                target: "sip:c@example.test".into(),
                status_code: 200,
                reason: "OK".into(),
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
            "progress:180",
            "ended",
            "failed:486",
            "cancelled",
            "hold",
            "resume",
            "remote-hold",
            "remote-resume",
            "dtmf:5",
            "media-security:true",
            "refer",
            "transfer-accepted",
            "refer-notify:100",
            "refer-progress:180",
            "refer-completed:200",
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
            25
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
    async fn callback_builder_invokes_common_closure_hooks() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let peer = CallbackPeer::builder(Config::local("callback-builder", 15441))
            .on_incoming({
                let seen = seen.clone();
                move |_call| {
                    let seen = seen.clone();
                    async move {
                        seen.lock().unwrap().push("incoming".to_string());
                        CallHandlerDecision::Reject {
                            status: 486,
                            reason: "Busy Here".into(),
                        }
                    }
                }
            })
            .on_established({
                let seen = seen.clone();
                move |_handle| {
                    let seen = seen.clone();
                    async move {
                        seen.lock().unwrap().push("established".to_string());
                        Ok(())
                    }
                }
            })
            .on_dtmf({
                let seen = seen.clone();
                move |_handle, digit| {
                    let seen = seen.clone();
                    async move {
                        seen.lock().unwrap().push(format!("dtmf:{digit}"));
                        Ok(())
                    }
                }
            })
            .on_ended({
                let seen = seen.clone();
                move |_call_id, _reason| {
                    let seen = seen.clone();
                    async move {
                        seen.lock().unwrap().push("ended".to_string());
                        Ok(())
                    }
                }
            })
            .on_transfer_request({
                let seen = seen.clone();
                move |_handle, target| {
                    let seen = seen.clone();
                    async move {
                        seen.lock().unwrap().push(format!("refer:{target}"));
                        Ok(true)
                    }
                }
            })
            .build()
            .await
            .unwrap();

        let mut handlers = JoinSet::new();
        let call_id = SessionId::new();
        for event in [
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
            Event::DtmfReceived {
                call_id: call_id.clone(),
                digit: '7',
            },
            Event::ReferReceived {
                call_id: call_id.clone(),
                refer_to: "sip:c@example.test".into(),
                referred_by: None,
                replaces: None,
                transaction_id: "tx-builder".into(),
                transfer_type: "blind".into(),
            },
            Event::CallEnded {
                call_id,
                reason: "normal".into(),
            },
        ] {
            peer.dispatch(event, &mut handlers).await;
        }
        drain(handlers).await;

        let seen = seen.lock().unwrap().clone();
        for expected in [
            "incoming",
            "established",
            "dtmf:7",
            "refer:sip:c@example.test",
            "ended",
        ] {
            assert!(
                seen.iter().any(|value| value == expected),
                "missing {expected}; saw {seen:?}"
            );
        }
    }

    #[tokio::test]
    async fn callback_builder_requires_incoming_hook() {
        let result = CallbackPeer::builder(Config::local("callback-builder-missing", 15443))
            .build()
            .await;
        let err = match result {
            Ok(_) => panic!("builder without on_incoming should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("on_incoming"));
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
