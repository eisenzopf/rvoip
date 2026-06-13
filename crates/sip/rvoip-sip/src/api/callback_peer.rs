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
//! running peer. Outbound calls flow through `control.invite(to)` and chain
//! `.with_extra_headers(...)` to attach caller-supplied typed headers to the
//! very first INVITE for PBX/SBC integrations that require non-standard or
//! vendor headers.
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
use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::api::dialog_package::{DialogInfo, DialogInfoDocument};
use crate::api::endpoint::SipAccount;
use crate::api::events::{
    Event, MediaSecurityState, SipTrace, SubscriptionState, TransferTargetEvidence,
};
use crate::api::handle::{CallId, SessionHandle};
use crate::api::incoming::{IncomingCall, IncomingCallGuard};
use crate::api::performance::PerformanceConfig;
use crate::api::unified::{
    Config, MediaMode, MediaSessionControllerConfig, RegistrationHandle, RtpSessionBufferConfig,
    RtpTransportBufferConfig, UnifiedCoordinator,
};
use crate::auth::SipClientAuth;
use crate::cleanup_diag::{self, CleanupStage};
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
type ReferReceivedHook = Arc<
    dyn Fn(SessionHandle, crate::api::incoming::IncomingRequest) -> CallbackFuture<Result<bool>>
        + Send
        + Sync,
>;
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
    refer_received: Option<ReferReceivedHook>,
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
            refer_received: None,
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

    /// Set peer-level UAC SIP auth used for outbound 401/407 retry.
    ///
    /// Use [`SipClientAuth::any`] when the peer may offer multiple schemes and
    /// the UAC should negotiate among Digest, Bearer, Basic, and AKA options.
    pub fn with_auth(mut self, auth: SipClientAuth) -> Self {
        self.config.auth = Some(auth);
        self
    }

    /// Set peer-level Digest credentials used for UAC outbound 401/407 retry.
    ///
    /// This is the Digest shorthand. Use [`Self::with_auth`] for Bearer,
    /// Basic, AKA, or multi-challenge negotiation.
    pub fn with_credentials(
        mut self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        self.config.credentials = Some(crate::types::Credentials::new(username, password));
        self
    }

    /// Set peer-level Bearer auth used for UAC outbound 401/407 retry.
    pub fn with_bearer_token(mut self, token: impl Into<String>) -> Self {
        self.config.auth = Some(SipClientAuth::bearer_token(token));
        self
    }

    /// Set peer-level Basic auth used for UAC outbound 401/407 retry.
    ///
    /// Basic remains cleartext-disabled unless the auth value explicitly opts
    /// in via [`SipClientAuth::allow_basic_over_cleartext`].
    pub fn with_basic_credentials(
        mut self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        self.config.auth = Some(SipClientAuth::basic(username, password));
        self
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

    /// Enable or disable automatic `180 Ringing` on inbound INVITEs.
    pub fn auto_180_ringing(mut self, enabled: bool) -> Self {
        self.config = self.config.with_auto_180_ringing(enabled);
        self
    }

    /// Enable or disable automatic `100 Trying` timer tasks on inbound INVITEs.
    pub fn auto_100_trying(mut self, enabled: bool) -> Self {
        self.config = self.config.with_auto_100_trying(enabled);
        self
    }

    /// Enable or disable immediate session-path accept for inbound INVITEs.
    pub fn fast_auto_accept_incoming_calls(mut self, enabled: bool) -> Self {
        self.config = self.config.with_fast_auto_accept_incoming_calls(enabled);
        self
    }

    /// Enable or disable real media-core RTP allocation.
    pub fn media_enabled(mut self, enabled: bool) -> Self {
        self.config = self.config.with_media_enabled(enabled);
        self
    }

    /// Skip media-core RTP allocation while still generating SDP.
    pub fn signaling_only_media(mut self, sdp_rtp_port: u16) -> Self {
        self.config = self
            .config
            .with_media_mode(MediaMode::SignalingOnly { sdp_rtp_port });
        self
    }

    /// Set the RTP media port range by start port and requested capacity.
    pub fn media_port_capacity(mut self, start: u16, capacity: usize) -> Self {
        self.config = self.config.with_media_port_capacity(start, capacity);
        self
    }

    /// Set the media-core session and RTP allocator capacity hint.
    pub fn media_session_capacity(mut self, capacity: usize) -> Self {
        self.config = self.config.with_media_session_capacity(capacity);
        self
    }

    /// Set RTP session queue sizing for SIP media calls.
    pub fn rtp_session_buffer_config(mut self, config: RtpSessionBufferConfig) -> Self {
        self.config = self.config.with_rtp_session_buffer_config(config);
        self
    }

    /// Set RTP transport event and receive buffer sizing for SIP media calls.
    pub fn rtp_transport_buffer_config(mut self, config: RtpTransportBufferConfig) -> Self {
        self.config = self.config.with_rtp_transport_buffer_config(config);
        self
    }

    /// Set media-core controller pool and capacity tuning for SIP media calls.
    pub fn media_session_controller_config(mut self, config: MediaSessionControllerConfig) -> Self {
        self.config = self.config.with_media_session_controller_config(config);
        self
    }

    /// Apply the high-CPS UDP auto-answer profile.
    pub fn high_cps_udp_auto_answer(mut self, capacity: usize) -> Self {
        self.config = self.config.with_high_cps_udp_auto_answer(capacity);
        self
    }

    /// Apply a YAML-backed performance recipe.
    pub fn performance_config(mut self, performance: PerformanceConfig) -> Result<Self> {
        self.config = self.config.try_with_performance_config(performance)?;
        Ok(self)
    }

    /// Apply the PBX media server performance recipe.
    pub fn pbx_media_server_performance(mut self, capacity: usize) -> Self {
        self.config = self.config.with_pbx_media_server_performance(capacity);
        self
    }

    /// Apply the signaling-only high-performance server recipe.
    pub fn signaling_only_server_high_performance(
        mut self,
        capacity: usize,
        sdp_rtp_port: u16,
    ) -> Self {
        self.config = self
            .config
            .with_signaling_only_server_high_performance(capacity, sdp_rtp_port);
        self
    }

    /// Set app-facing event buffer capacity.
    pub fn app_event_channel_capacity(mut self, capacity: usize) -> Self {
        self.config = self.config.with_app_event_channel_capacity(capacity);
        self
    }

    /// Set the UDP parse worker count.
    pub fn sip_udp_parse_workers(mut self, workers: usize) -> Self {
        self.config = self.config.with_sip_udp_parse_workers(workers);
        self
    }

    /// Set the per-worker UDP parse queue capacity.
    pub fn sip_udp_parse_queue_capacity(mut self, capacity: usize) -> Self {
        self.config = self.config.with_sip_udp_parse_queue_capacity(capacity);
        self
    }

    /// Set the per-transaction command channel capacity.
    pub fn sip_transaction_command_channel_capacity(mut self, capacity: usize) -> Self {
        self.config = self
            .config
            .with_sip_transaction_command_channel_capacity(capacity);
        self
    }

    /// Set the server-side inbound call admission limit.
    pub fn server_call_admission_limit(mut self, limit: usize) -> Self {
        self.config = self.config.with_server_call_admission_limit(limit);
        self
    }

    /// Set the soft threshold where server-side admission starts pacing.
    pub fn server_call_admission_soft_limit(mut self, limit: usize) -> Self {
        self.config = self.config.with_server_call_admission_soft_limit(limit);
        self
    }

    /// Set the delay in milliseconds while above the soft admission threshold.
    pub fn server_call_admission_pacing_delay_ms(mut self, delay_ms: u64) -> Self {
        self.config = self
            .config
            .with_server_call_admission_pacing_delay_ms(delay_ms);
        self
    }

    /// Set the `Retry-After` value used for server overload rejections.
    pub fn server_overload_retry_after_secs(mut self, seconds: u32) -> Self {
        self.config = self.config.with_server_overload_retry_after_secs(seconds);
        self
    }

    /// Enable or disable SIP UDP transport and duplicate-recovery diagnostics.
    pub fn sip_udp_diagnostics(mut self, enabled: bool) -> Self {
        self.config = self.config.with_sip_udp_diagnostics(enabled);
        self
    }

    /// Enable or disable media setup/teardown timing diagnostics.
    pub fn media_setup_diagnostics(mut self, enabled: bool) -> Self {
        self.config = self.config.with_media_setup_diagnostics(enabled);
        self
    }

    /// Enable or disable cleanup-stage timing diagnostics.
    pub fn cleanup_diagnostics(mut self, enabled: bool) -> Self {
        self.config = self.config.with_cleanup_diagnostics(enabled);
        self
    }

    /// Enable or disable per-operation cleanup diagnostic event logs.
    pub fn cleanup_diagnostic_events(mut self, enabled: bool) -> Self {
        self.config = self.config.with_cleanup_diagnostic_events(enabled);
        self
    }

    /// Set the RSS growth threshold used by perf soak release gates.
    #[cfg(feature = "perf-tests")]
    pub fn perf_max_rss_growth_mb_per_hr(mut self, limit: f64) -> Self {
        self.config = self.config.with_perf_max_rss_growth_mb_per_hr(limit);
        self
    }

    /// Enable or disable SRTP negotiation diagnostic log lines.
    pub fn srtp_diagnostics(mut self, enabled: bool) -> Self {
        self.config = self.config.with_srtp_diagnostics(enabled);
        self
    }

    /// Enable or disable RTP packet diagnostic log lines.
    pub fn rtp_diagnostics(mut self, enabled: bool) -> Self {
        self.config = self.config.with_rtp_diagnostics(enabled);
        self
    }

    /// Enable or disable SDP media diagnostic log lines.
    pub fn media_sdp_diagnostics(mut self, enabled: bool) -> Self {
        self.config = self.config.with_media_sdp_diagnostics(enabled);
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

    /// Decide whether to accept an inbound REFER (transfer request).
    ///
    /// SIP_API_DESIGN_2 Phase E. The closure receives a
    /// [`SessionHandle`] for the original call and an
    /// [`IncomingRequest`](crate::api::incoming::IncomingRequest) carrying
    /// every REFER header (Referred-By, Replaces, Target-Dialog, custom
    /// X-*). Return `Ok(true)` to send `202 Accepted` and have the
    /// transferee follow the Refer-To URI; return `Ok(false)` (or an
    /// `Err`) to send `603 Decline`.
    pub fn on_refer_received<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(SessionHandle, crate::api::incoming::IncomingRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<bool>> + Send + 'static,
    {
        self.refer_received = Some(Arc::new(move |handle, req| Box::pin(f(handle, req))));
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
                refer_received: self.refer_received,
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
    refer_received: Option<ReferReceivedHook>,
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

    async fn on_refer_received(&self, request: crate::api::incoming::IncomingRequest) {
        let Some(hook) = &self.refer_received else {
            return;
        };
        let Some(coord) = request.coordinator.clone() else {
            tracing::warn!(
                "[CallbackPeerBuilder] on_refer_received fired without a coordinator hook; \
                 dropping REFER for call {}",
                request.call_id
            );
            return;
        };
        let handle = SessionHandle::new(request.call_id.clone(), coord);
        let accepted = match hook(handle.clone(), request).await {
            Ok(b) => b,
            Err(err) => {
                tracing::warn!(
                    "[CallbackPeerBuilder] on_refer_received failed; rejecting REFER: {}",
                    err
                );
                false
            }
        };
        let result = if accepted {
            handle.accept_refer().await
        } else {
            handle.reject_refer(603, "Decline").await
        };
        if let Err(err) = result {
            tracing::warn!(
                "[CallbackPeerBuilder] applying REFER decision failed: {}",
                err
            );
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

    /// SIP_API_DESIGN_2 Phase E — typed inbound NOTIFY hook. Carries
    /// the full `IncomingRequest` so applications can inspect every
    /// header on the NOTIFY (Server, Allow-Events, custom routing
    /// hints) in addition to the legacy pre-decoded fields. Default
    /// impl is a no-op.
    #[allow(unused_variables)]
    async fn on_notify_received(&self, request: crate::api::incoming::IncomingRequest) {}

    /// SIP_API_DESIGN_2 Phase E — typed inbound REFER hook. Apps drive
    /// accept/reject via `req.accept_refer()` / `req.reject_refer(...)`
    /// inside this callback. Use for header-level access (Referred-By,
    /// Replaces, Target-Dialog, custom X-*).
    #[allow(unused_variables)]
    async fn on_refer_received(&self, request: crate::api::incoming::IncomingRequest) {}

    /// SIP_API_DESIGN_2 Phase E — typed inbound INFO hook. Today's
    /// stack drops INFO at the dialog layer; this hook surfaces it so
    /// SIP-INFO DTMF (`application/dtmf-relay`), fax flow control,
    /// and other application-layer signalling can be observed.
    #[allow(unused_variables)]
    async fn on_info_received(&self, request: crate::api::incoming::IncomingRequest) {}

    /// SIP_API_DESIGN_2 Phase E — typed inbound MESSAGE hook (RFC 3428).
    #[allow(unused_variables)]
    async fn on_message_received(&self, request: crate::api::incoming::IncomingRequest) {}

    /// SIP_API_DESIGN_2 Phase E — typed inbound OPTIONS hook
    /// (RFC 3261 §11). Both in-dialog and out-of-dialog OPTIONS reach
    /// here; `request.call_id` discriminates.
    #[allow(unused_variables)]
    async fn on_options_received(&self, request: crate::api::incoming::IncomingRequest) {}

    /// SIP_API_DESIGN_2 Phase E — typed inbound UPDATE hook
    /// (RFC 3311). Fires in addition to the legacy hold/resume state
    /// transitions; subscribe here for Session-Expires / X-*
    /// header inspection.
    #[allow(unused_variables)]
    async fn on_update_received(&self, request: crate::api::incoming::IncomingRequest) {}

    /// SIP_API_DESIGN_2 Phase D — typed inbound REGISTER hook
    /// (RFC 3261 §10). Registrar surfaces author the response via
    /// `register.accept_builder()` / `register.challenge_builder(..)` /
    /// `register.reject_builder(status)`; if the handler returns
    /// without authoring a response, the dialog stack falls back to
    /// the auto-response path.
    #[allow(unused_variables)]
    async fn on_register_received(&self, register: crate::api::incoming::IncomingRegister) {}

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
    /// `coord.invite(...).with_credentials(...)`; this hook does not alter
    /// flow. Useful for logging or surfacing auth activity in a UI.
    ///
    /// [`Config.credentials`]: crate::api::unified::Config::credentials
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
    /// Begin building an outbound REGISTER from this peer.
    ///
    /// Returns a [`RegisterBuilder`](crate::api::send::RegisterBuilder)
    /// — chain `.with_expires(..)`, `.with_from_uri(..)`,
    /// `.with_contact_uri(..)`, `.with_credentials(..)` etc. and finish
    /// with `.send().await`.
    pub fn register(
        &self,
        registrar: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> crate::api::send::RegisterBuilder {
        self.coordinator.register(registrar, username, password)
    }

    /// Begin building an outbound REGISTER from a shared SIP account.
    pub fn register_account(&self, account: &SipAccount) -> crate::api::send::RegisterBuilder {
        let mut builder = self
            .register(
                account.registrar.clone(),
                account.effective_auth_username().to_string(),
                account.password.clone(),
            )
            .with_expires(account.expires);
        if let Some(from_uri) = &account.from_uri {
            builder = builder.with_from_uri(from_uri.clone());
        }
        if let Some(contact_uri) = &account.contact_uri {
            builder = builder.with_contact_uri(contact_uri.clone());
        }
        builder
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

    /// Begin building an outbound INVITE from this peer's configured
    /// `local_uri`. Returns an
    /// [`OutboundCallBuilder`](crate::api::send::OutboundCallBuilder).
    pub fn invite(&self, target: impl Into<String>) -> crate::api::send::OutboundCallBuilder {
        self.coordinator
            .invite(Some(self.local_uri.clone()), target)
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
    terminal_callbacks: Arc<tokio::sync::Mutex<BoundedCallDedupe>>,
    deferred_calls: Arc<tokio::sync::Mutex<HashMap<CallId, IncomingCallGuard>>>,
}

const TERMINAL_CALLBACK_DEDUPE_CAPACITY: usize = 8192;

struct BoundedCallDedupe {
    set: HashSet<CallId>,
    order: VecDeque<CallId>,
    capacity: usize,
}

impl BoundedCallDedupe {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            set: HashSet::with_capacity(capacity),
            order: VecDeque::with_capacity(capacity),
            capacity: capacity.max(1),
        }
    }

    fn insert(&mut self, call_id: CallId) -> bool {
        if self.set.contains(&call_id) {
            return false;
        }

        self.set.insert(call_id.clone());
        self.order.push_back(call_id);

        while self.order.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.set.remove(&oldest);
            }
        }

        true
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.set.len()
    }
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
    /// Set `config.auth` for general UAC 401/407 retry across supported SIP
    /// auth schemes, or `config.credentials` as the Digest shorthand:
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_sip::Result<()> {
    /// use rvoip_sip::{CallbackPeer, Config, SipClientAuth};
    /// use rvoip_sip::api::handlers::AutoAnswerHandler;
    ///
    /// let config = Config {
    ///     auth: Some(SipClientAuth::digest("alice", "secret")),
    ///     ..Config::default()
    /// };
    /// let peer = CallbackPeer::new(AutoAnswerHandler, config).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// For per-call overrides, use `coordinator().invite(...).with_auth(...)`
    /// or the Digest shorthand `.with_credentials(...)` via
    /// [`coordinator()`](Self::coordinator).
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
            terminal_callbacks: Arc::new(tokio::sync::Mutex::new(
                BoundedCallDedupe::with_capacity(TERMINAL_CALLBACK_DEDUPE_CAPACITY),
            )),
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

    /// Begin building an outbound INVITE from this peer's configured
    /// `local_uri`. Equivalent to
    /// `peer.coordinator().invite(Some(local_uri), target)`.
    pub fn invite(&self, target: impl Into<String>) -> crate::api::send::OutboundCallBuilder {
        self.coordinator
            .invite(Some(self.local_uri.clone()), target)
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
    /// let call = control.invite("sip:bob@example.com").send().await?;
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

    /// Begin building an outbound REGISTER. Delegates to
    /// [`CallbackPeerControl::register`].
    pub fn register(
        &self,
        registrar: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> crate::api::send::RegisterBuilder {
        self.coordinator.register(registrar, username, password)
    }

    /// Begin building an outbound REGISTER from a shared SIP account.
    pub fn register_account(&self, account: &SipAccount) -> crate::api::send::RegisterBuilder {
        self.control().register_account(account)
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
            reap_ready_handlers(&mut handlers, "completed");
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
                    reap_ready_handlers(&mut handlers, "post-dispatch");
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

        // Shut down the coordinator and wait for dialog/transaction
        // transports to close before `run()` returns. Tests and services may
        // immediately restart a peer on the same port after this future
        // resolves.
        if let Err(e) = self
            .coordinator
            .shutdown_gracefully(Some(Duration::ZERO))
            .await
        {
            tracing::warn!("[CallbackPeer] Coordinator shutdown failed: {}", e);
        }
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
        let fast_auto_accept_incoming_calls = coordinator.fast_auto_accept_incoming_calls();

        handlers.spawn(async move {
            let dispatch_guard = cleanup_diag::stage_guard(
                callback_stage_for_event(&event),
                callback_label_for_event(&event),
            );
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
                    //
                    // SIP_API_DESIGN_2 Phase A: prefer the parsed
                    // `Arc<Request>` view when the bus enriched it.
                    let parsed = coordinator
                        .session_registry
                        .peek_pending_incoming_request()
                        .await;
                    let transport = coordinator
                        .session_registry
                        .peek_pending_incoming_transport()
                        .await;
                    let incoming = match parsed {
                        Some(req) => IncomingCall::with_request(
                            call_id.clone(),
                            from,
                            to,
                            sdp,
                            coordinator.clone(),
                            req,
                        ),
                        None => IncomingCall::new(
                            call_id.clone(),
                            from,
                            to,
                            sdp,
                            coordinator.clone(),
                        ),
                    }
                    .with_transport_context(
                        transport
                            .as_deref()
                            .cloned()
                            .unwrap_or_else(crate::auth::SipTransportSecurityContext::unknown),
                    );
                    let decision = handler.on_incoming_call(incoming).await;
                    // These coordinator calls are idempotent — if the handler
                    // already resolved the call, the session has transitioned
                    // out of Ringing and the call becomes a no-op error we ignore.
                    match decision {
                        CallHandlerDecision::Accept => {
                            if fast_auto_accept_incoming_calls {
                                tracing::debug!(
                                    "Callback accept decision for {} already handled by fast auto-accept",
                                    call_id
                                );
                                return;
                            }

                            let accept_guard = cleanup_diag::stage_guard(
                                CleanupStage::CallbackAcceptCall,
                                call_id.to_string(),
                            );
                            match coordinator.accept_call(&call_id).await {
                                Ok(()) => {
                                    accept_guard.finish_success();
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
                                    accept_guard.finish_failure();
                                }
                            }
                        }
                        CallHandlerDecision::AcceptWithSdp(sdp) => {
                            if fast_auto_accept_incoming_calls {
                                tracing::debug!(
                                    "Callback accept-with-SDP decision for {} already handled by fast auto-accept",
                                    call_id
                                );
                                return;
                            }

                            let accept_guard = cleanup_diag::stage_guard(
                                CleanupStage::CallbackAcceptCall,
                                call_id.to_string(),
                            );
                            match coordinator.accept_call_with_sdp(&call_id, sdp).await {
                                Ok(()) => {
                                    accept_guard.finish_success();
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
                                    accept_guard.finish_failure();
                                }
                            }
                        }
                        CallHandlerDecision::Reject { status, reason } => {
                            if fast_auto_accept_incoming_calls {
                                tracing::debug!(
                                    "Callback reject decision for {} ignored because fast auto-accept already answered the call",
                                    call_id
                                );
                                return;
                            }

                            let _ = coordinator
                                .reject(&call_id)
                                .with_status(status)
                                .with_reason(reason)
                                .send()
                                .await;
                        }
                        CallHandlerDecision::Redirect(target) => {
                            if fast_auto_accept_incoming_calls {
                                tracing::debug!(
                                    "Callback redirect decision for {} ignored because fast auto-accept already answered the call",
                                    call_id
                                );
                                return;
                            }

                            let _ = coordinator
                                .redirect(&call_id)
                                .with_status(302)
                                .with_contacts(vec![target])
                                .send()
                                .await;
                        }
                        CallHandlerDecision::Defer(guard) => {
                            if fast_auto_accept_incoming_calls {
                                guard.resolve_without_response();
                                tracing::debug!(
                                    "Callback defer decision for {} ignored because fast auto-accept already answered the call",
                                    call_id
                                );
                                return;
                            }

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
                    call_id: _, refer_to: _, request, ..
                } => {
                    // SIP_API_DESIGN_2 Phase E §9.5 — typed
                    // `on_refer_received(IncomingRequest)` hook only.
                    // Applications drive accept/reject via
                    // `req.accept_refer()` / `req.reject_refer(...)`
                    // inside the callback.
                    if let Some(mut req) = request {
                        req.set_coordinator(coordinator.clone());
                        handler.on_refer_received(req).await;
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
                    call_id: _,
                    event_package: _,
                    subscription_state: _,
                    content_type: _,
                    body: _,
                    request,
                } => {
                    // SIP_API_DESIGN_2 Phase E §9.5 — typed
                    // `on_notify_received(IncomingRequest)` hook only.
                    // Applications inspect headers/body via the
                    // `IncomingRequest` directly.
                    if let Some(mut req) = request {
                        req.set_coordinator(coordinator.clone());
                        handler.on_notify_received(req).await;
                    }
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
                | Event::AuthenticationRequired { .. }
                // SIP_API_DESIGN_2 Phase A: detailed-response events are
                // an additive surface alongside the legacy `CallProgress`
                // / `CallEnded` / `CallFailed` variants. The callback
                // surface routes the existing variants today; consumers
                // can also pattern-match on the detailed variant via
                // `handler.on_event(...)`.
                | Event::CallProgressDetailed(_)
                | Event::CallEstablishedDetailed(_)
                | Event::CallFailedDetailed(_) => {}
                // SIP_API_DESIGN_2 Phase E: typed mid-dialog inbound
                // events. Rehydrate the coordinator hook on the
                // IncomingRequest before forwarding to the handler so
                // any *_builder() it calls can dispatch.
                Event::InfoReceived { call_id: _, mut request } => {
                    request.set_coordinator(coordinator.clone());
                    handler.on_info_received(request).await;
                }
                Event::MessageReceived { call_id: _, mut request } => {
                    request.set_coordinator(coordinator.clone());
                    handler.on_message_received(request).await;
                }
                Event::OptionsReceived { call_id: _, mut request } => {
                    request.set_coordinator(coordinator.clone());
                    handler.on_options_received(request).await;
                }
                Event::UpdateReceived { call_id: _, mut request } => {
                    request.set_coordinator(coordinator.clone());
                    handler.on_update_received(request).await;
                }
                Event::IncomingRegister { mut register } => {
                    register.set_coordinator(coordinator.clone());
                    handler.on_register_received(register).await;
                }
            }
            dispatch_guard.finish_success();
        });
    }
}

fn reap_ready_handlers(handlers: &mut tokio::task::JoinSet<()>, context: &str) {
    while let Some(join_result) = handlers.try_join_next() {
        if let Err(e) = join_result {
            if !e.is_cancelled() {
                tracing::warn!("[CallbackPeer] Handler task panicked or errored ({context}): {e}");
            }
        }
    }
}

fn callback_stage_for_event(event: &Event) -> CleanupStage {
    match event {
        Event::IncomingCall { .. } => CleanupStage::CallbackIncomingDispatch,
        _ => CleanupStage::CallbackEventDispatch,
    }
}

fn callback_label_for_event(event: &Event) -> String {
    event
        .call_id()
        .map(|call_id| call_id.to_string())
        .unwrap_or_else(|| "-".to_string())
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

    #[test]
    fn callback_peer_builder_exposes_rtp_media_buffer_tuning() {
        let session_buffers = RtpSessionBufferConfig {
            sender_channel_capacity: 9,
            receiver_channel_capacity: 6,
            event_channel_capacity: 18,
        };
        let transport_buffers = RtpTransportBufferConfig {
            event_channel_capacity: 14,
            recv_buffer_size: 2048,
            rtcp_recv_buffer_size: 1024,
        };
        let mut media_config = MediaSessionControllerConfig::default();
        media_config.rtp_buffer_size = 960;
        media_config.rtp_buffer_initial_count = 5;
        media_config.rtp_buffer_max_count = 20;

        let builder = CallbackPeer::builder(Config::local("callback-builder", 15444))
            .media_session_controller_config(media_config)
            .rtp_session_buffer_config(session_buffers)
            .rtp_transport_buffer_config(transport_buffers);

        assert_eq!(builder.config.rtp_session_buffer_config, session_buffers);
        assert_eq!(
            builder.config.rtp_transport_buffer_config,
            transport_buffers
        );
        assert_eq!(
            builder
                .config
                .media_session_controller_config
                .rtp_buffer_size,
            960
        );
        assert_eq!(
            builder
                .config
                .media_session_controller_config
                .rtp_buffer_initial_count,
            5
        );
        assert_eq!(
            builder
                .config
                .media_session_controller_config
                .rtp_buffer_max_count,
            20
        );
    }

    #[test]
    fn bounded_terminal_callback_dedupe_evicts_oldest_entries() {
        let mut dedupe = BoundedCallDedupe::with_capacity(2);
        assert!(dedupe.insert(SessionId("call-1".to_string())));
        assert!(!dedupe.insert(SessionId("call-1".to_string())));
        assert!(dedupe.insert(SessionId("call-2".to_string())));
        assert_eq!(dedupe.len(), 2);
        assert!(dedupe.insert(SessionId("call-3".to_string())));
        assert_eq!(dedupe.len(), 2);
        assert!(dedupe.insert(SessionId("call-1".to_string())));
    }

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
                request: None,
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
                request: None,
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
            "transfer-accepted",
            "refer-notify:100",
            "refer-progress:180",
            "refer-completed:200",
            "transfer-failed:503",
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
                request: None,
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
        for expected in ["incoming", "established", "dtmf:7", "ended"] {
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

        let call_id = control
            .invite("sip:unreachable@127.0.0.1:15443")
            .send()
            .await
            .unwrap();
        assert!(!call_id.to_string().is_empty());

        control.shutdown();
        stop.shutdown();
        tokio::time::timeout(std::time::Duration::from_secs(2), run_task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}
