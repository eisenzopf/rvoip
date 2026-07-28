//! Incoming call handling — accept, reject, redirect, or defer.
//!
//! Inbound SIP messages reach the application through one of four typed
//! wrappers that all implement
//! [`crate::api::headers::SipHeaderView`]:
//!
//! | Wrapper | What it wraps |
//! |---|---|
//! | [`IncomingCall`] | Inbound INVITE |
//! | [`IncomingRequest`] | In-dialog REFER / NOTIFY / INFO / OPTIONS / UPDATE / MESSAGE |
//! | [`IncomingResponse`] | Every inbound response (1xx / 2xx / 3xx / 4xx-6xx) |
//! | [`IncomingRegister`] | Inbound REGISTER on registrar surfaces |

#![deny(missing_docs)]

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, AtomicU8, Ordering},
    Arc, OnceLock, Weak,
};
use std::time::{Duration, Instant};

use rvoip_sip_core::types::headers::{HeaderAccess, HeaderName, TypedHeader};
use rvoip_sip_core::{Request, Response};

use crate::api::events::Event;
use crate::api::handle::{CallId, SessionHandle};
use crate::api::headers::view::{
    header_names_slice, header_slice, headers_named_slice, SipHeaderView,
};
use crate::api::lifecycle::{CallLifecycleSnapshot, CallTerminalInfo};
use crate::api::unified::UnifiedCoordinator;
use crate::errors::{Result, SessionError};
use crate::session_lifecycle::{OwnedOperation, OwnedOperationCompletion, SessionOperationKind};
use crate::session_registry::SessionRegistryHandle;
use crate::state_table::EventType;
use crate::types::CallState;

const INCOMING_GUARD_RESPONSE_COMPLETION_GRACE: Duration = Duration::from_secs(2);
const INCOMING_RESOLUTION_OPERATION_TIMEOUT: Duration = Duration::from_secs(10);

async fn wait_for_incoming_guard_cancellation(cancel: &mut tokio::sync::watch::Receiver<bool>) {
    loop {
        if *cancel.borrow_and_update() {
            return;
        }
        if cancel.changed().await.is_err() {
            return;
        }
    }
}

async fn rollback_owned_incoming_guard<T>(
    operation: OwnedOperation,
    value: T,
) -> OwnedOperationCompletion<T> {
    operation
        .rollback(value)
        .await
        .unwrap_or_else(|_| panic!("incoming-call guard exact rollback failed"))
}

async fn commit_owned_incoming_guard<T>(
    operation: OwnedOperation,
    value: T,
) -> OwnedOperationCompletion<T> {
    match operation.commit() {
        Ok(committed) => committed.complete(value),
        Err(failure) => rollback_owned_incoming_guard(failure.into_operation(), value).await,
    }
}

fn spawn_exact_incoming_reject(
    coordinator: Arc<UnifiedCoordinator>,
    lifecycle_handle: Option<SessionRegistryHandle>,
    status: u16,
    reason: String,
) {
    let Some(lifecycle_handle) = lifecycle_handle else {
        tracing::warn!(
            status,
            "incoming-call rejection suppressed because exact lifecycle authority is absent"
        );
        return;
    };
    let authority = Arc::clone(coordinator.helpers.state_machine.store.authority());
    let state_machine = Arc::clone(&coordinator.helpers.state_machine);
    let operation_key = lifecycle_handle.key().clone();
    let log_call_id = lifecycle_handle.session_id().clone();
    let scheduled = authority.spawn_owned_exact(
        &operation_key,
        SessionOperationKind::Signaling,
        INCOMING_RESOLUTION_OPERATION_TIMEOUT,
        move |operation| async move {
            let committed = match operation.commit() {
                Ok(committed) => committed,
                Err(failure) => {
                    return rollback_owned_incoming_guard(failure.into_operation(), ()).await;
                }
            };
            if let Err(error) = state_machine
                .process_event_exact(&lifecycle_handle, EventType::RejectCall { status, reason })
                .await
            {
                tracing::debug!(
                    session_id = %lifecycle_handle.session_id(),
                    %error,
                    "exact incoming-call rejection did not dispatch"
                );
            }
            committed.complete(())
        },
    );
    if let Err(error) = scheduled {
        tracing::debug!(
            session_id = %log_call_id,
            %error,
            "exact incoming-call rejection was not admitted"
        );
    }
}

/// An incoming SIP INVITE that must be handled.
///
/// Obtain one via
/// [`StreamPeer::wait_for_incoming`](crate::api::stream_peer::StreamPeer::wait_for_incoming)
/// or via the `on_incoming_call` method of
/// [`CallHandler`](crate::api::callback_peer::CallHandler).
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
/// | [`redirect_to()`] | 302 | Proxy / forward-to-voicemail |
/// | [`redirect_with_contacts()`] | 3xx | Multiple redirect choices |
/// | [`defer()`] | (hold) | Call center queue |
///
/// [`accept()`]: IncomingCall::accept
/// [`reject()`]: IncomingCall::reject
/// [`reject_busy()`]: IncomingCall::reject_busy
/// [`reject_decline()`]: IncomingCall::reject_decline
/// [`redirect_to()`]: IncomingCall::redirect_to
/// [`redirect_with_contacts()`]: IncomingCall::redirect_with_contacts
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
    ///
    /// **Deprecated:** prefer
    /// [`crate::api::headers::SipHeaderView`] inspection
    /// via [`Self::header`], [`Self::header_str`], or
    /// [`Self::raw_request`]. This field is populated from the parsed
    /// INVITE for back-compat and may carry only a curated subset of
    /// the wire headers. Will be removed in a future breaking release.
    pub headers: HashMap<String, String>,
    /// When the INVITE arrived.
    pub received_at: Instant,
    /// The parsed inbound INVITE request, when available. `None` only
    /// when the call was synthesized (tests, lean-mode feature flag).
    pub(crate) request: Option<Arc<Request>>,
    /// Transport context for auth policy decisions.
    pub(crate) transport: crate::auth::SipTransportSecurityContext,
    /// Internal — coordinator for performing accept/reject.
    pub(crate) coordinator: Arc<UnifiedCoordinator>,
    /// Causal authority for the exact registry lifetime that emitted this
    /// inbound INVITE. It is never reconstructed by delayed work.
    lifecycle_handle: Option<SessionRegistryHandle>,
    /// Whether this call has already been resolved (to catch double-resolve).
    resolved: bool,
}

impl std::fmt::Debug for IncomingCall {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IncomingCall")
            .field("sdp_present", &self.sdp.is_some())
            .field("sdp_bytes", &self.sdp.as_ref().map_or(0, String::len))
            .field("legacy_header_count", &self.headers.len())
            .field("raw_request_present", &self.request.is_some())
            .field(
                "raw_request_header_count",
                &self
                    .request
                    .as_ref()
                    .map_or(0, |request| request.headers.len()),
            )
            .field(
                "raw_request_body_bytes",
                &self
                    .request
                    .as_ref()
                    .map_or(0, |request| request.body().len()),
            )
            .field("resolved", &self.resolved)
            .finish()
    }
}

impl IncomingCall {
    #[cfg(test)]
    pub(crate) fn new(
        call_id: CallId,
        from: String,
        to: String,
        sdp: Option<String>,
        coordinator: Arc<UnifiedCoordinator>,
    ) -> Self {
        let lifecycle_handle = coordinator
            .helpers
            .state_machine
            .store
            .lifecycle_handle(&call_id);
        Self::new_captured(call_id, from, to, sdp, coordinator, lifecycle_handle)
    }

    pub(crate) fn new_captured(
        call_id: CallId,
        from: String,
        to: String,
        sdp: Option<String>,
        coordinator: Arc<UnifiedCoordinator>,
        lifecycle_handle: Option<SessionRegistryHandle>,
    ) -> Self {
        Self {
            call_id,
            from,
            to,
            sdp,
            headers: HashMap::new(),
            received_at: Instant::now(),
            request: None,
            transport: crate::auth::SipTransportSecurityContext::unknown(),
            coordinator,
            lifecycle_handle,
            resolved: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_exact(
        call_id: CallId,
        from: String,
        to: String,
        sdp: Option<String>,
        coordinator: Arc<UnifiedCoordinator>,
        lifecycle_handle: SessionRegistryHandle,
    ) -> Self {
        debug_assert_eq!(&call_id, lifecycle_handle.session_id());
        Self::new_captured(call_id, from, to, sdp, coordinator, Some(lifecycle_handle))
    }

    /// Construct an `IncomingCall` with a parsed inbound INVITE retained.
    /// Populates [`Self::headers`] from the request for compatibility with the
    /// legacy field while preserving the captured exact lifecycle.
    pub(crate) fn with_request_captured(
        call_id: CallId,
        from: String,
        to: String,
        sdp: Option<String>,
        coordinator: Arc<UnifiedCoordinator>,
        request: Arc<Request>,
        lifecycle_handle: Option<SessionRegistryHandle>,
    ) -> Self {
        let mut headers: HashMap<String, String> = HashMap::new();
        for hdr in &request.headers {
            let name = hdr.name();
            let key = legacy_header_key(&name);
            headers.entry(key).or_insert_with(|| hdr.to_string());
        }
        Self {
            call_id,
            from,
            to,
            sdp,
            headers,
            received_at: Instant::now(),
            request: Some(request),
            transport: crate::auth::SipTransportSecurityContext::unknown(),
            coordinator,
            lifecycle_handle,
            resolved: false,
        }
    }

    pub(crate) fn with_transport_context(
        mut self,
        transport: crate::auth::SipTransportSecurityContext,
    ) -> Self {
        self.transport = transport;
        self
    }

    /// Transport context used by auth policy decisions.
    pub fn transport_security_context(&self) -> &crate::auth::SipTransportSecurityContext {
        &self.transport
    }

    /// Underlying parsed [`Request`]. `None` when the call was
    /// synthesized (tests) or under a future lean-mode feature flag.
    pub fn raw_request(&self) -> Option<&Arc<Request>> {
        self.request.as_ref()
    }

    /// Authenticate this inbound INVITE with a UAS SIP auth service.
    ///
    /// Pass [`SipAuthService`](crate::SipAuthService) for Digest, Bearer,
    /// Basic, AKA, or multi-challenge validation. Pass
    /// [`SipDigestAuthService`](crate::SipDigestAuthService) for Digest-only
    /// compatibility.
    pub async fn authenticate_with<A>(
        &self,
        auth: &A,
    ) -> Result<<A as crate::auth::SipIncomingAuthenticator>::Decision>
    where
        A: crate::auth::SipIncomingAuthenticator + Sync,
    {
        let request = self.request.as_ref().ok_or_else(|| {
            SessionError::InvalidInput(
                "IncomingCall.authenticate_with() requires a parsed inbound INVITE".to_string(),
            )
        })?;
        let (authorization, source) = selected_authorization(request);
        let request_uri = request.uri().to_string();
        let body = if request.body().is_empty() {
            None
        } else {
            Some(request.body())
        };
        let transport = effective_transport_security_context(Some(request), &self.transport);
        auth.authenticate_incoming_with_transport_context(
            authorization.as_deref(),
            "INVITE",
            &request_uri,
            body,
            source,
            &transport,
        )
        .await
    }

    /// Zero-alloc header iteration — preferred over the boxed trait
    /// method on hot paths.
    pub fn headers_named_iter<'a>(
        &'a self,
        name: &'a HeaderName,
    ) -> impl Iterator<Item = &'a TypedHeader> + 'a {
        let slice: &[TypedHeader] = match &self.request {
            Some(r) => &r.headers[..],
            None => &[],
        };
        headers_named_slice(slice, name)
    }

    // ===== Resolution methods =====

    /// Accept the call and return a [`SessionHandle`] for controlling it.
    ///
    /// Completes SDP negotiation and sends 200 OK.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(incoming: rvoip_sip::IncomingCall) -> rvoip_sip::Result<()> {
    /// let call = incoming.accept().await?;
    /// # let _ = call;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn accept(self) -> Result<SessionHandle> {
        self.accept_builder().send().await
    }

    /// Accept the call with a custom SDP answer.
    ///
    /// This is intended for B2BUA or gateway flows where the application has
    /// already obtained an answer body from another leg.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(incoming: rvoip_sip::IncomingCall, answer_sdp: String) -> rvoip_sip::Result<()> {
    /// let call = incoming.accept_with_sdp(answer_sdp).await?;
    /// # let _ = call;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn accept_with_sdp(self, sdp: String) -> Result<SessionHandle> {
        self.accept_builder().with_sdp(sdp).send().await
    }

    /// Send a reliable 183 Session Progress with early-media SDP (RFC 3262).
    ///
    /// Does NOT consume the call — you still need to call [`accept()`] or
    /// [`reject()`] afterward. Typical sequence:
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(incoming: rvoip_sip::IncomingCall) -> rvoip_sip::Result<()> {
    /// incoming.send_early_media(None).await?;
    /// tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    /// let session = incoming.accept().await?;
    /// # let _ = session;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// See [`PeerControl::send_early_media`] for the full semantics and the
    /// 100rel failure mode.
    ///
    /// [`accept()`]: IncomingCall::accept
    /// [`reject()`]: IncomingCall::reject
    /// [`PeerControl::send_early_media`]: crate::api::stream_peer::PeerControl::send_early_media
    pub async fn send_early_media(&self, sdp: Option<String>) -> Result<()> {
        let lifecycle_handle = self.lifecycle_handle.as_ref().ok_or_else(|| {
            SessionError::SessionNotFound(format!(
                "Incoming call {} has no exact lifecycle authority",
                self.call_id
            ))
        })?;
        self.coordinator
            .helpers
            .send_early_media_exact(lifecycle_handle, sdp)
            .await
    }

    /// Send a reliable 183 Session Progress and immediately swap the
    /// session's RTP transmitter to `source`. Lets the UAS play a ringback
    /// tone / "please hold" announcement during the `EarlyMedia` state.
    ///
    /// The source plays until the dialog transitions to `Active` (after
    /// `accept()`), at which point the state machine automatically swaps
    /// back to [`AudioSource::PassThrough`][crate::api::unified::AudioSource::PassThrough]
    /// so bidirectional audio flows without further action. Apps that want
    /// a *different* source during the active call (hold music, continued
    /// announcement) can call
    /// [`coordinator.set_audio_source`][crate::api::unified::UnifiedCoordinator::set_audio_source]
    /// again after observing `Event::CallEstablished`.
    ///
    /// See `docs/AUDIO_MODES.md` for the endpoint-vs-bridge comparison.
    ///
    /// Same 100rel precondition as [`send_early_media`][Self::send_early_media].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use rvoip_sip::{AudioSource, IncomingCall};
    /// # async fn demo(incoming: IncomingCall) -> rvoip_sip::Result<()> {
    /// incoming.send_early_media_with_source(
    ///     None,
    ///     AudioSource::Tone { frequency: 440.0, amplitude: 0.5 },
    /// ).await?;
    /// // Ringback plays; later call accept() to answer.
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_early_media_with_source(
        &self,
        sdp: Option<String>,
        source: crate::api::unified::AudioSource,
    ) -> Result<()> {
        let lifecycle_handle = self.lifecycle_handle.as_ref().ok_or_else(|| {
            SessionError::SessionNotFound(format!(
                "Incoming call {} has no exact lifecycle authority",
                self.call_id
            ))
        })?;
        self.coordinator
            .helpers
            .send_early_media_exact(lifecycle_handle, sdp)
            .await?;
        self.coordinator
            .set_audio_source_exact(lifecycle_handle, source)
            .await
    }

    /// Reject the call immediately with an explicit SIP status code and reason.
    ///
    /// This spawns the reject operation and returns immediately.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # fn example(incoming: rvoip_sip::IncomingCall) {
    /// incoming.reject(486, "Busy Here");
    /// # }
    /// ```
    pub fn reject(mut self, status: u16, reason: &str) {
        self.resolved = true;
        spawn_exact_incoming_reject(
            self.coordinator.clone(),
            self.lifecycle_handle.clone(),
            status,
            reason.to_string(),
        );
    }

    /// Reject with **486 Busy Here**.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # fn example(incoming: rvoip_sip::IncomingCall) {
    /// incoming.reject_busy();
    /// # }
    /// ```
    pub fn reject_busy(self) {
        self.reject(486, "Busy Here");
    }

    /// Reject with **603 Decline** (user explicitly declined).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # fn example(incoming: rvoip_sip::IncomingCall) {
    /// incoming.reject_decline();
    /// # }
    /// ```
    pub fn reject_decline(self) {
        self.reject(603, "Decline");
    }

    /// Redirect the caller to another URI with **302 Moved Temporarily**.
    ///
    /// This sends a SIP 3xx response with a `Contact` header. It is a
    /// rvoip-sip primitive; higher-level B2BUA/routing layers decide
    /// whether redirect is the right policy for a particular call.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(incoming: rvoip_sip::IncomingCall) -> rvoip_sip::Result<()> {
    /// incoming.redirect_to("sip:voicemail@example.com").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn redirect_to(self, target: impl Into<String>) -> Result<()> {
        self.redirect_with_contacts(302, [target.into()]).await
    }

    /// Redirect the caller with an explicit 3xx status and Contact list.
    ///
    /// `status` must be in `300..=399` and `contacts` must contain at least
    /// one SIP URI string.
    pub async fn redirect_with_contacts<I, S>(mut self, status: u16, contacts: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        if !(300..=399).contains(&status) {
            return Err(SessionError::InvalidInput(format!(
                "redirect status must be 3xx, got {status}"
            )));
        }
        let contacts = contacts.into_iter().map(Into::into).collect::<Vec<_>>();
        if contacts.is_empty() {
            return Err(SessionError::InvalidInput(
                "redirect requires at least one Contact URI".to_string(),
            ));
        }
        self.resolved = true;
        crate::api::respond::RedirectBuilder::new_captured(
            self.coordinator.clone(),
            self.call_id.clone(),
            self.lifecycle_handle.clone(),
        )
        .with_status(status)
        .with_contacts(contacts)
        .send()
        .await
    }

    /// Defer the accept/reject decision, keeping the call in `Ringing` state
    /// until the returned [`IncomingCallGuard`] is resolved or the `timeout`
    /// elapses.
    ///
    /// Use this for call queues: store the guard in a queue data structure and
    /// call `guard.accept()` when an agent becomes available.
    ///
    /// If the timeout elapses while the guard is unresolved, or the guard is
    /// dropped without being resolved, the call is rejected with **503 Service
    /// Unavailable**.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # fn example(incoming: rvoip_sip::IncomingCall) {
    /// let guard = incoming.defer(std::time::Duration::from_secs(30));
    /// // Store `guard` in a queue and later call guard.accept().await.
    /// # let _ = guard;
    /// # }
    /// ```
    pub fn defer(mut self, timeout: Duration) -> IncomingCallGuard {
        self.resolved = true; // prevent Drop from also rejecting
        if self.coordinator.fast_auto_accept_incoming_calls() {
            return IncomingCallGuard::resolved(
                self.call_id.clone(),
                self.coordinator.clone(),
                self.lifecycle_handle.clone(),
            );
        }
        IncomingCallGuard::new_captured(
            self.call_id.clone(),
            self.coordinator.clone(),
            timeout,
            self.lifecycle_handle.clone(),
        )
    }

    // ─────────────────────────────────────────────────────────────────
    // SIP_API_DESIGN_2 Phase D — builder entry points on inbound INVITE.
    // ─────────────────────────────────────────────────────────────────

    /// Begin an `AcceptBuilder` so additional response headers can be
    /// staged before sending 200 OK.
    pub fn accept_builder(mut self) -> crate::api::respond::AcceptBuilder {
        self.resolved = true;
        crate::api::respond::AcceptBuilder::new_captured(
            self.coordinator.clone(),
            self.call_id.clone(),
            self.lifecycle_handle.clone(),
        )
    }

    /// Begin a `RejectBuilder` for a custom 4xx/5xx/6xx response.
    pub fn reject_builder(mut self) -> crate::api::respond::RejectBuilder {
        self.resolved = true;
        crate::api::respond::RejectBuilder::new_captured(
            self.coordinator.clone(),
            self.call_id.clone(),
            self.lifecycle_handle.clone(),
        )
    }

    /// Begin a `RedirectBuilder` for a custom 3xx response.
    pub fn redirect_builder(mut self) -> crate::api::respond::RedirectBuilder {
        self.resolved = true;
        crate::api::respond::RedirectBuilder::new_captured(
            self.coordinator.clone(),
            self.call_id.clone(),
            self.lifecycle_handle.clone(),
        )
    }

    /// Begin a `ProvisionalBuilder` for a 1xx reliable provisional.
    pub fn send_provisional_builder(&self, code: u16) -> crate::api::respond::ProvisionalBuilder {
        crate::api::respond::ProvisionalBuilder::new_captured(
            self.coordinator.clone(),
            self.call_id.clone(),
            self.lifecycle_handle.clone(),
            code,
        )
    }

    /// Begin an `AuthChallengeBuilder` for a 401/407 response.
    pub fn challenge_builder(
        &self,
        scheme: crate::api::respond::AuthScheme,
    ) -> crate::api::respond::AuthChallengeBuilder {
        crate::api::respond::AuthChallengeBuilder::new_captured(
            self.coordinator.clone(),
            self.call_id.clone(),
            rvoip_sip_core::types::Method::Invite,
            scheme,
            self.lifecycle_handle.clone(),
        )
    }

    /// Begin a `GenericResponseBuilder` for an arbitrary 3xx-6xx
    /// status (e.g. 491 Request Pending with `Retry-After`).
    pub fn respond_builder(
        mut self,
        status: u16,
    ) -> Result<crate::api::respond::GenericResponseBuilder> {
        self.resolved = true;
        crate::api::respond::GenericResponseBuilder::new_captured(
            self.coordinator.clone(),
            self.call_id.clone(),
            self.lifecycle_handle.clone(),
            rvoip_sip_core::types::Method::Invite,
            status,
        )
    }
}

/// Return the historical lowercase map key from the canonical SIP wire name.
///
/// The legacy map is protocol data despite being deprecated, so its keys must
/// never be derived from `Debug`: diagnostic output is deliberately redacted
/// and is free to change independently of RFC header spelling.
fn legacy_header_key(name: &HeaderName) -> String {
    name.canonical_wire_name().as_str().to_ascii_lowercase()
}

impl SipHeaderView for IncomingCall {
    fn header(&self, name: &HeaderName) -> Option<&TypedHeader> {
        self.request
            .as_ref()
            .and_then(|r| header_slice(&r.headers[..], name))
    }

    fn headers_named<'a>(
        &'a self,
        name: &HeaderName,
    ) -> Box<dyn Iterator<Item = &'a TypedHeader> + 'a> {
        match &self.request {
            Some(r) => {
                let name = name.clone();
                Box::new(
                    r.headers.iter().filter(move |h| {
                        crate::api::headers::view::header_name_eq(&h.name(), &name)
                    }),
                )
            }
            None => Box::new(std::iter::empty()),
        }
    }

    fn headers<'a>(&'a self) -> Box<dyn Iterator<Item = &'a TypedHeader> + 'a> {
        match &self.request {
            Some(r) => Box::new(r.headers.iter()),
            None => Box::new(std::iter::empty()),
        }
    }

    fn header_names(&self) -> Vec<HeaderName> {
        match &self.request {
            Some(r) => header_names_slice(&r.headers[..]),
            None => Vec::new(),
        }
    }
}

impl Drop for IncomingCall {
    fn drop(&mut self) {
        // Safety net for panicking handlers only. Normal paths set
        // `resolved = true` via accept/reject/defer, OR rely on the
        // CallbackPeer dispatch to apply the CallHandlerDecision after
        // this IncomingCall is dropped — neither should trigger an
        // auto-reject here.
        //
        // `thread::panicking()` is true while the current thread is
        // unwinding from a panic (which is exactly when destructors run
        // during task panics under tokio). This lets us distinguish the
        // rare "handler crashed" path from ordinary drops.
        if self.resolved || !std::thread::panicking() {
            return;
        }
        let coordinator = self.coordinator.clone();
        let call_id = self.call_id.clone();
        // RFC 3261 §21.5.1: 500 is the correct code for a server-side
        // unexpected failure. Sending it terminates the UAC's INVITE
        // transaction cleanly instead of leaving it hanging until Timer C.
        tracing::warn!(
            "[IncomingCall] handler panicked for call {} — sending 500 Server Internal Error",
            call_id
        );
        spawn_exact_incoming_reject(
            coordinator,
            self.lifecycle_handle.clone(),
            500,
            "Server Internal Error".to_string(),
        );
    }
}

// ===== IncomingCallGuard =====

/// A deferred incoming call held in `Ringing` state.
///
/// Created by [`IncomingCall::defer()`]. Must be resolved by calling
/// [`accept()`] or [`reject()`] before the deadline. If the deadline elapses
/// while the guard is unresolved, or the guard is dropped unresolved, the call
/// is rejected with **503 Service Unavailable**.
///
/// [`accept()`]: IncomingCallGuard::accept
/// [`reject()`]: IncomingCallGuard::reject
pub struct IncomingCallGuard {
    call_id: CallId,
    coordinator: Arc<UnifiedCoordinator>,
    lifecycle_handle: Option<SessionRegistryHandle>,
    deadline: Instant,
    resolved: Arc<AtomicBool>,
}

impl IncomingCallGuard {
    #[cfg(test)]
    fn new(call_id: CallId, coordinator: Arc<UnifiedCoordinator>, timeout: Duration) -> Self {
        let lifecycle_handle = coordinator
            .helpers
            .state_machine
            .store
            .lifecycle_handle(&call_id);
        Self::new_captured(call_id, coordinator, timeout, lifecycle_handle)
    }

    fn new_captured(
        call_id: CallId,
        coordinator: Arc<UnifiedCoordinator>,
        timeout: Duration,
        lifecycle_handle: Option<SessionRegistryHandle>,
    ) -> Self {
        let deadline = Instant::now() + timeout;
        let resolved = Arc::new(AtomicBool::new(false));

        // The watchdog is retained by the exact session lifecycle rather than
        // a detached raw-ID task. Quiesce cancels it, drain joins it, and the
        // retained registry handle prevents a later admission with the same
        // application identifier from receiving this rejection.
        let state_machine = Arc::clone(&coordinator.helpers.state_machine);
        if let Some(lifecycle_handle) = lifecycle_handle.clone() {
            let authority = Arc::clone(state_machine.store.authority());
            let operation_key = lifecycle_handle.key().clone();
            let watchdog_resolved = Arc::clone(&resolved);
            let remaining = deadline.saturating_duration_since(Instant::now());
            let hard_timeout = remaining.saturating_add(INCOMING_GUARD_RESPONSE_COMPLETION_GRACE);
            let scheduled = authority.spawn_owned_exact(
                &operation_key,
                SessionOperationKind::Signaling,
                hard_timeout,
                move |operation| async move {
                    let Some(mut cancellation) = operation.cancellation() else {
                        return rollback_owned_incoming_guard(operation, ()).await;
                    };
                    tokio::select! {
                        _ = tokio::time::sleep(remaining) => {}
                        () = wait_for_incoming_guard_cancellation(&mut cancellation) => {
                            return rollback_owned_incoming_guard(operation, ()).await;
                        }
                    }

                    if watchdog_resolved.swap(true, Ordering::SeqCst) {
                        return rollback_owned_incoming_guard(operation, ()).await;
                    }

                    let dispatch = state_machine
                        .process_event_exact(
                            &lifecycle_handle,
                            EventType::RejectCall {
                                status: 503,
                                reason: "Service Unavailable".to_string(),
                            },
                        )
                        .await;
                    if let Err(error) = dispatch {
                        tracing::debug!(
                            session_id = %lifecycle_handle.session_id(),
                            %error,
                            "incoming-call guard timeout did not target a current exact lifetime"
                        );
                        return rollback_owned_incoming_guard(operation, ()).await;
                    }
                    commit_owned_incoming_guard(operation, ()).await
                },
            );
            if let Err(error) = scheduled {
                tracing::debug!(
                    session_id = %call_id,
                    %error,
                    "incoming-call guard watchdog was not admitted for exact lifecycle ownership"
                );
            }
        }

        Self {
            call_id,
            coordinator,
            lifecycle_handle,
            deadline,
            resolved,
        }
    }

    fn resolved(
        call_id: CallId,
        coordinator: Arc<UnifiedCoordinator>,
        lifecycle_handle: Option<SessionRegistryHandle>,
    ) -> Self {
        Self {
            call_id,
            coordinator,
            lifecycle_handle,
            deadline: Instant::now(),
            resolved: Arc::new(AtomicBool::new(true)),
        }
    }

    /// The call identifier for this deferred call (use as queue key).
    ///
    /// This accessor is trivial and can be used to index a queue or map.
    pub fn call_id(&self) -> &CallId {
        &self.call_id
    }

    /// Mark this deferred call as resolved because rvoip-sip already
    /// observed a terminal event for it. This prevents the drop safety net
    /// from sending a late rejection for a call that no longer exists.
    pub(crate) fn resolve_without_response(&self) {
        self.resolved.store(true, Ordering::SeqCst);
    }

    /// When the guard expires and the call is auto-rejected.
    ///
    /// This accessor is trivial and is useful for queue ordering.
    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Accept the call now. Returns a [`SessionHandle`].
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(guard: rvoip_sip::IncomingCallGuard) -> rvoip_sip::Result<()> {
    /// let call = guard.accept().await?;
    /// # let _ = call;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn accept(self) -> Result<SessionHandle> {
        let lifecycle_handle = self.lifecycle_handle.clone().ok_or_else(|| {
            SessionError::SessionNotFound(format!(
                "Deferred incoming call {} has no exact lifecycle authority",
                self.call_id
            ))
        })?;
        if self.coordinator.fast_auto_accept_incoming_calls() {
            self.resolved.store(true, Ordering::SeqCst);
            return Ok(SessionHandle::new_exact(
                self.call_id.clone(),
                self.coordinator.clone(),
                lifecycle_handle,
            ));
        }

        if Instant::now() >= self.deadline {
            return Err(SessionError::Timeout(
                "IncomingCallGuard deadline exceeded before accept".to_string(),
            ));
        }
        if self.resolved.swap(true, Ordering::SeqCst) {
            return Err(SessionError::InvalidTransition(format!(
                "IncomingCallGuard for {} is already resolved",
                self.call_id
            )));
        }
        self.coordinator
            .helpers
            .accept_call_exact(&lifecycle_handle)
            .await?;
        Ok(SessionHandle::new_exact(
            self.call_id.clone(),
            self.coordinator.clone(),
            lifecycle_handle,
        ))
    }

    /// Reject the call now with the given SIP status code and reason phrase.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # fn example(guard: rvoip_sip::IncomingCallGuard) {
    /// guard.reject(503, "Service Unavailable");
    /// # }
    /// ```
    pub fn reject(self, status: u16, reason: &str) {
        if self.resolved.swap(true, Ordering::SeqCst) {
            return;
        }
        if self.coordinator.fast_auto_accept_incoming_calls() {
            return;
        }
        spawn_exact_incoming_reject(
            self.coordinator.clone(),
            self.lifecycle_handle.clone(),
            status,
            reason.to_string(),
        );
    }

    /// Abandon the deferred call locally without sending a SIP response.
    ///
    /// This is an explicit escape hatch for tests or external policy engines
    /// that know the INVITE is already being resolved elsewhere. Normal
    /// applications should prefer [`accept`](Self::accept),
    /// [`reject`](Self::reject), or waiting for a real cancellation event.
    pub fn abandon(self) {
        self.resolved.store(true, Ordering::SeqCst);
    }

    /// Reject the call and wait for the matching terminal event.
    ///
    /// This is the deterministic variant for queues and tests that need to
    /// know when the rejection has been observed by rvoip-sip's event
    /// stream. The event subscription is opened before the reject is sent.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(guard: rvoip_sip::IncomingCallGuard) -> rvoip_sip::Result<()> {
    /// let terminal = guard
    ///     .reject_and_wait(503, "Service Unavailable", Some(std::time::Duration::from_secs(3)))
    ///     .await?;
    /// # let _ = terminal;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn reject_and_wait(
        self,
        status: u16,
        reason: &str,
        timeout: Option<Duration>,
    ) -> Result<Event> {
        if self.coordinator.fast_auto_accept_incoming_calls() {
            self.resolved.store(true, Ordering::SeqCst);
            return Ok(Event::CallAnswered {
                call_id: self.call_id.clone(),
                sdp: None,
            });
        }
        if self.resolved.swap(true, Ordering::SeqCst) {
            return Err(SessionError::InvalidTransition(format!(
                "IncomingCallGuard for {} is already resolved",
                self.call_id
            )));
        }

        let mut events = self.coordinator.events_for_session(&self.call_id).await?;
        let lifecycle_handle = self.lifecycle_handle.as_ref().ok_or_else(|| {
            SessionError::SessionNotFound(format!(
                "Deferred incoming call {} has no exact lifecycle authority",
                self.call_id
            ))
        })?;
        self.coordinator
            .helpers
            .reject_call_exact(lifecycle_handle, status, reason)
            .await?;

        let fut = async {
            loop {
                match events.next().await {
                    Some(
                        event @ (Event::CallFailed { .. }
                        | Event::CallEnded { .. }
                        | Event::CallCancelled { .. }),
                    ) => return Ok(event),
                    Some(_) => {}
                    None => {
                        return Err(SessionError::Other(
                            "Event channel closed while waiting for reject".to_string(),
                        ));
                    }
                }
            }
        };

        match timeout {
            Some(duration) => tokio::time::timeout(duration, fut)
                .await
                .map_err(|_| SessionError::Timeout("reject_and_wait timed out".to_string()))?,
            None => fut.await,
        }
    }

    /// Wait for the caller to cancel this deferred ringing call.
    ///
    /// This is useful for ring/cancel tests, queues, and callback-style
    /// applications that intentionally keep an INVITE in ringing state. The
    /// method resolves only on [`Event::CallCancelled`]. It returns an error
    /// if the call is answered, rejected, failed, normally ended, the guard
    /// deadline expires, or the event stream closes.
    ///
    /// A caller-supplied timeout only cancels this wait. It does not mark the
    /// guard resolved, reject the call, abandon the call, or suppress the
    /// drop-time safety rejection.
    pub async fn wait_for_cancelled(&self, timeout: Option<Duration>) -> Result<()> {
        if self.resolved.load(Ordering::SeqCst) {
            return Err(SessionError::InvalidTransition(format!(
                "IncomingCallGuard for {} is already resolved",
                self.call_id
            )));
        }

        let resolved = self.resolved.clone();
        if let Some(result) = cancellation_result_from_snapshot(
            &self.coordinator.lifecycle_snapshot(&self.call_id).await,
        ) {
            resolved.store(true, Ordering::SeqCst);
            return result;
        }

        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(SessionError::Timeout(
                "IncomingCallGuard deadline elapsed before cancellation".to_string(),
            ));
        }

        let wait_duration = timeout.map_or(remaining, |duration| duration.min(remaining));
        let mut events = self.coordinator.events_for_session(&self.call_id).await?;

        if let Some(result) = cancellation_result_from_snapshot(
            &self.coordinator.lifecycle_snapshot(&self.call_id).await,
        ) {
            resolved.store(true, Ordering::SeqCst);
            return result;
        }

        let fut = async {
            loop {
                match events.next().await {
                    Some(Event::CallCancelled { .. }) => {
                        resolved.store(true, Ordering::SeqCst);
                        return Ok(());
                    }
                    Some(Event::CallAnswered { .. }) => {
                        resolved.store(true, Ordering::SeqCst);
                        return Err(SessionError::Other(
                            "incoming call was answered before cancellation".to_string(),
                        ));
                    }
                    Some(Event::CallFailed {
                        status_code,
                        reason,
                        ..
                    }) => {
                        resolved.store(true, Ordering::SeqCst);
                        return Err(SessionError::Other(format!(
                            "incoming call failed before cancellation: {} {}",
                            status_code, reason
                        )));
                    }
                    Some(Event::CallEnded { reason, .. }) => {
                        resolved.store(true, Ordering::SeqCst);
                        return Err(SessionError::Other(format!(
                            "incoming call ended before cancellation: {}",
                            reason
                        )));
                    }
                    Some(_) => {}
                    None => {
                        return Err(SessionError::Other(
                            "Event channel closed while waiting for cancellation".to_string(),
                        ));
                    }
                }
            }
        };

        match tokio::time::timeout(wait_duration, fut).await {
            Ok(result) => result,
            Err(_) => {
                if Instant::now() >= self.deadline {
                    Err(SessionError::Timeout(
                        "IncomingCallGuard deadline elapsed before cancellation".to_string(),
                    ))
                } else {
                    Err(SessionError::Timeout(
                        "wait_for_cancelled timed out".to_string(),
                    ))
                }
            }
        }
    }
}

fn cancellation_result_from_snapshot(snapshot: &CallLifecycleSnapshot) -> Option<Result<()>> {
    match snapshot.terminal.as_ref() {
        Some(CallTerminalInfo::Cancelled) => return Some(Ok(())),
        Some(CallTerminalInfo::Failed {
            status_code,
            reason,
        }) => {
            return Some(Err(SessionError::Other(format!(
                "incoming call failed before cancellation: {} {}",
                status_code, reason
            ))));
        }
        Some(CallTerminalInfo::Ended { reason }) => {
            return Some(Err(SessionError::Other(format!(
                "incoming call ended before cancellation: {}",
                reason
            ))));
        }
        None => {}
    }

    if snapshot.answered.is_some() {
        return Some(Err(SessionError::Other(
            "incoming call was answered before cancellation".to_string(),
        )));
    }

    match snapshot.state.as_ref()? {
        CallState::Cancelled => Some(Ok(())),
        CallState::Failed(reason) => Some(Err(SessionError::Other(format!(
            "incoming call failed before cancellation: {:?}",
            reason
        )))),
        CallState::Terminated => Some(Err(SessionError::Other(
            "incoming call ended before cancellation".to_string(),
        ))),
        CallState::Answering
        | CallState::AnsweringHangupPending
        | CallState::Active
        | CallState::HoldPending
        | CallState::OnHold
        | CallState::Resuming
        | CallState::Muted
        | CallState::Bridged
        | CallState::Transferring
        | CallState::TransferringCall
        | CallState::ConsultationCall => Some(Err(SessionError::Other(
            "incoming call was answered before cancellation".to_string(),
        ))),
        _ => None,
    }
}

impl Drop for IncomingCallGuard {
    fn drop(&mut self) {
        if !self.resolved.swap(true, Ordering::SeqCst) {
            spawn_exact_incoming_reject(
                self.coordinator.clone(),
                self.lifecycle_handle.clone(),
                503,
                "Service Unavailable".to_string(),
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// IncomingRequest / IncomingResponse / IncomingRegister
//
// These three wrappers complete the inbound surface. They are populated
// when rvoip-sip-dialog publishes an in-dialog request, an inbound response,
// or an inbound REGISTER. Today only `IncomingCall` carries a fully
// parsed `Arc<Request>`; the other three retain the same field shape so
// later phases can fill in their typed payloads without breaking the
// public API.
// ─────────────────────────────────────────────────────────────────────────

/// An in-dialog received SIP request (REFER / NOTIFY / INFO /
/// OPTIONS / UPDATE / MESSAGE).
///
/// Implements [`SipHeaderView`] for uniform header inspection.
#[derive(Clone)]
pub struct IncomingRequest {
    /// The session this request belongs to, when known.
    pub call_id: CallId,
    /// SIP From URI on the request.
    pub from: String,
    /// SIP To URI on the request.
    pub to: String,
    /// The SIP method (REFER, NOTIFY, …).
    pub method: rvoip_sip_core::types::Method,
    /// When the request arrived.
    pub received_at: Instant,
    /// The parsed inbound request, when available.
    pub(crate) request: Option<Arc<Request>>,
    /// Transport context for auth policy decisions.
    pub(crate) transport: crate::auth::SipTransportSecurityContext,
    /// Exact inbound server transaction used for an application-authored
    /// response. Legacy/synthetic events may not carry one.
    pub(crate) response_transaction: Option<rvoip_sip_dialog::transaction::TransactionKey>,
    /// Shared single-writer obligation for the exact final response.
    pub(crate) response_obligation: Option<Arc<ExactResponseObligation>>,
    /// Coordinator for sending responses. `None` for bus-constructed
    /// instances; the surface consumer (CallbackPeer / StreamPeer)
    /// repopulates this on dispatch so response builders can resolve
    /// the dialog/transaction.
    pub(crate) coordinator: Option<Arc<UnifiedCoordinator>>,
    /// Exact session generation carried by the causal event envelope. A
    /// missing sidecar stays missing so delayed requests never re-resolve a
    /// reused application Call-ID.
    lifecycle_handle: Option<SessionRegistryHandle>,
}

const EXACT_RESPONSE_AVAILABLE: u8 = 0;
const EXACT_RESPONSE_IN_FLIGHT: u8 = 1;
const EXACT_RESPONSE_COMPLETE: u8 = 2;

/// One cancellation-safe final-response authority retained by the
/// coordinator deadline registry.
///
/// INFO binds this owner to an exact session generation before registration;
/// REGISTER uses a standalone exact server-transaction scope.
pub(crate) struct ExactResponseObligation {
    state: AtomicU8,
    coordinator: OnceLock<Weak<UnifiedCoordinator>>,
    scope: OnceLock<ExactResponseScope>,
    call_id: Option<CallId>,
    transaction: rvoip_sip_dialog::transaction::TransactionKey,
    fallback_status: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ExactResponseScope {
    Session(SessionRegistryHandle),
    StandaloneTransaction,
}

impl ExactResponseObligation {
    pub(crate) fn new(
        call_id: CallId,
        transaction: rvoip_sip_dialog::transaction::TransactionKey,
    ) -> Self {
        Self {
            state: AtomicU8::new(EXACT_RESPONSE_AVAILABLE),
            coordinator: OnceLock::new(),
            scope: OnceLock::new(),
            call_id: Some(call_id),
            transaction,
            fallback_status: 501,
        }
    }

    pub(crate) fn new_standalone(
        transaction: rvoip_sip_dialog::transaction::TransactionKey,
        fallback_status: u16,
    ) -> Self {
        debug_assert!((200..=699).contains(&fallback_status));
        let scope = OnceLock::new();
        let _ = scope.set(ExactResponseScope::StandaloneTransaction);
        Self {
            state: AtomicU8::new(EXACT_RESPONSE_AVAILABLE),
            coordinator: OnceLock::new(),
            scope,
            call_id: None,
            transaction,
            fallback_status,
        }
    }

    pub(crate) fn attach_coordinator(&self, coordinator: &Arc<UnifiedCoordinator>) {
        let _ = self.coordinator.set(Arc::downgrade(coordinator));
    }

    pub(crate) fn bind_lifecycle_handle(&self, handle: &SessionRegistryHandle) -> bool {
        if self.call_id.as_ref() != Some(handle.session_id()) {
            return false;
        }
        match self.scope.set(ExactResponseScope::Session(handle.clone())) {
            Ok(()) => true,
            Err(_) => self.scope.get() == Some(&ExactResponseScope::Session(handle.clone())),
        }
    }

    pub(crate) fn lifecycle_handle(&self) -> Option<&SessionRegistryHandle> {
        match self.scope.get() {
            Some(ExactResponseScope::Session(handle)) => Some(handle),
            Some(ExactResponseScope::StandaloneTransaction) | None => None,
        }
    }

    pub(crate) fn has_owner_scope(&self) -> bool {
        self.scope.get().is_some()
    }

    pub(crate) fn transaction(&self) -> &rvoip_sip_dialog::transaction::TransactionKey {
        &self.transaction
    }

    pub(crate) fn fallback_status(&self) -> u16 {
        self.fallback_status
    }

    pub(crate) fn claim(self: &Arc<Self>) -> Result<ExactResponseClaim> {
        self.state
            .compare_exchange(
                EXACT_RESPONSE_AVAILABLE,
                EXACT_RESPONSE_IN_FLIGHT,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map(|_| ExactResponseClaim {
                obligation: Arc::clone(self),
                armed: true,
            })
            .map_err(|_| {
                SessionError::InvalidInput(
                    "exact inbound response has already been claimed".to_string(),
                )
            })
    }

    fn complete(&self) {
        self.state.store(EXACT_RESPONSE_COMPLETE, Ordering::Release);
        if let Some(coordinator) = self.coordinator.get().and_then(Weak::upgrade) {
            coordinator.complete_exact_response_obligation(self);
        }
    }

    fn release_after_failure(&self) {
        let _ = self.state.compare_exchange(
            EXACT_RESPONSE_IN_FLIGHT,
            EXACT_RESPONSE_AVAILABLE,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

/// Cancellation-safe ownership of one exact final-response attempt.
///
/// A successful write completes the obligation. An ordinary write error may
/// release it for an explicit retry. If the send future is cancelled or
/// panics, dropping the still-armed guard releases the claim. The
/// coordinator-owned deadline registry retains the obligation and authors the
/// stack-owned fallback without requiring a task spawn from `Drop`.
pub(crate) struct ExactResponseClaim {
    obligation: Arc<ExactResponseObligation>,
    armed: bool,
}

impl ExactResponseClaim {
    pub(crate) fn complete(mut self) {
        self.obligation.complete();
        self.armed = false;
    }

    pub(crate) fn release_after_failure(mut self) {
        self.obligation.release_after_failure();
        self.armed = false;
    }
}

impl Drop for ExactResponseClaim {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        self.obligation.release_after_failure();
    }
}

pub(crate) fn safe_incoming_method_debug_label(
    method: &rvoip_sip_core::types::Method,
) -> &'static str {
    use rvoip_sip_core::types::Method;

    match method {
        Method::Invite => "INVITE",
        Method::Ack => "ACK",
        Method::Bye => "BYE",
        Method::Cancel => "CANCEL",
        Method::Register => "REGISTER",
        Method::Options => "OPTIONS",
        Method::Subscribe => "SUBSCRIBE",
        Method::Notify => "NOTIFY",
        Method::Update => "UPDATE",
        Method::Refer => "REFER",
        Method::Info => "INFO",
        Method::Message => "MESSAGE",
        Method::Prack => "PRACK",
        Method::Publish => "PUBLISH",
        Method::Extension(_) => "extension",
    }
}

impl std::fmt::Debug for IncomingRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IncomingRequest")
            .field("method", &safe_incoming_method_debug_label(&self.method))
            .field("raw_request_present", &self.request.is_some())
            .field(
                "raw_request_header_count",
                &self
                    .request
                    .as_ref()
                    .map_or(0, |request| request.headers.len()),
            )
            .field(
                "raw_request_body_bytes",
                &self
                    .request
                    .as_ref()
                    .map_or(0, |request| request.body().len()),
            )
            .field("coordinator_present", &self.coordinator.is_some())
            .field(
                "response_transaction_present",
                &self.response_transaction.is_some(),
            )
            .finish()
    }
}

impl IncomingRequest {
    // ─────────────────────────────────────────────────────────────────
    // SIP_API_DESIGN_2 Phase D — response builder entry points.
    // ─────────────────────────────────────────────────────────────────

    /// Borrow a [`SessionHandle`] for the dialog this inbound request
    /// belongs to.
    ///
    /// Returns `Err(SessionError::InvalidInput)` when the event has not
    /// supplied both the coordinator hook and exact lifecycle authority.
    /// A missing generation is never recovered from the raw Call-ID because
    /// that identifier may already belong to a later session lifetime.
    ///
    /// Use this from `on_refer_received` / `on_notify_received` trait
    /// impls when you need to drive in-dialog actions on the session
    /// (e.g. `handle.accept_refer().await`).
    pub fn session_handle(&self) -> Result<crate::api::handle::SessionHandle> {
        let coord = self.coordinator.clone().ok_or_else(|| {
            SessionError::InvalidInput(
                "IncomingRequest.session_handle() requires a coordinator hook; \
                 the bus path has not yet rehydrated it"
                    .to_string(),
            )
        })?;
        let lifecycle_handle = self.lifecycle_handle.clone().ok_or_else(|| {
            SessionError::InvalidInput(
                "IncomingRequest.session_handle() requires exact lifecycle authority".to_string(),
            )
        })?;
        Ok(crate::api::handle::SessionHandle::new_exact(
            self.call_id.clone(),
            coord,
            lifecycle_handle,
        ))
    }

    /// Begin a `GenericResponseBuilder` for the inbound request.
    /// Returns `Err(SessionError::InvalidInput)` when this
    /// `IncomingRequest` lacks the coordinator hook or exact lifecycle
    /// authority carried by its causal event.
    pub fn respond_builder(
        &self,
        status: u16,
    ) -> Result<crate::api::respond::GenericResponseBuilder> {
        if self.response_transaction.is_some() {
            let transaction_id = self.exact_response_transaction()?;
            let coord = self.coordinator.clone().ok_or_else(|| {
                SessionError::InvalidInput(
                    "IncomingRequest.respond_builder() requires a coordinator hook; the bus \
                     path has not yet rehydrated it"
                        .to_string(),
                )
            })?;
            return crate::api::respond::GenericResponseBuilder::new_in_dialog(
                coord,
                self.call_id.clone(),
                self.method.clone(),
                transaction_id,
                status,
                self.exact_response_obligation()?,
            );
        }
        if matches!(
            self.method,
            rvoip_sip_core::Method::Info
                | rvoip_sip_core::Method::Notify
                | rvoip_sip_core::Method::Update
                | rvoip_sip_core::Method::Refer
        ) {
            return Err(SessionError::InvalidInput(
                "IncomingRequest.respond_builder() requires an exact inbound transaction for this in-dialog method"
                    .to_string(),
            ));
        }
        let coord = self.coordinator.clone().ok_or_else(|| {
            SessionError::InvalidInput(
                "IncomingRequest.respond_builder() requires a coordinator hook; the bus \
                 path has not yet rehydrated it"
                    .to_string(),
            )
        })?;
        let lifecycle_handle = self.lifecycle_handle.clone().ok_or_else(|| {
            SessionError::InvalidInput(
                "IncomingRequest.respond_builder() requires exact lifecycle authority".to_string(),
            )
        })?;
        crate::api::respond::GenericResponseBuilder::new_exact(
            coord,
            self.call_id.clone(),
            lifecycle_handle,
            self.method.clone(),
            status,
        )
    }

    /// Begin a final 2xx–6xx response for this exact inbound in-dialog
    /// transaction.
    ///
    /// This does not run INVITE accept/reject call-state transitions. It is
    /// therefore the response API for inbound INFO and similar application
    /// requests. Legacy events that lack exact transaction correlation return
    /// `InvalidInput` rather than guessing from dialog state.
    pub fn respond(&self, status: u16) -> Result<crate::api::respond::InDialogResponseBuilder> {
        let transaction_id = self.exact_response_transaction()?;
        let coord = self.coordinator.clone().ok_or_else(|| {
            SessionError::InvalidInput(
                "IncomingRequest.respond() requires a coordinator hook; the bus path has not yet rehydrated it"
                    .to_string(),
            )
        })?;
        crate::api::respond::InDialogResponseBuilder::new(
            coord,
            self.call_id.clone(),
            transaction_id,
            status,
            self.exact_response_obligation()?,
        )
    }

    fn exact_response_obligation(&self) -> Result<Arc<ExactResponseObligation>> {
        self.response_obligation.clone().ok_or_else(|| {
            SessionError::InvalidInput(
                "IncomingRequest response requires an exact response obligation".to_string(),
            )
        })
    }

    fn exact_response_transaction(&self) -> Result<rvoip_sip_dialog::transaction::TransactionKey> {
        let transaction = self.response_transaction.clone().ok_or_else(|| {
            SessionError::InvalidInput(
                "IncomingRequest response requires an exact inbound server transaction".to_string(),
            )
        })?;
        let wire_transaction = self
            .request
            .as_ref()
            .and_then(|request| {
                rvoip_sip_dialog::transaction::TransactionKey::from_request(request)
            })
            .ok_or_else(|| {
                SessionError::InvalidInput(
                    "IncomingRequest response requires an exact transaction on the wire request"
                        .to_string(),
                )
            })?;
        if transaction != wire_transaction || transaction.method() != &self.method {
            return Err(SessionError::InvalidInput(
                "IncomingRequest response transaction does not match the wire request".to_string(),
            ));
        }
        Ok(transaction)
    }

    /// Begin an `AuthChallengeBuilder` for 401/407 on the inbound request.
    pub fn challenge_builder(
        &self,
        scheme: crate::api::respond::AuthScheme,
    ) -> Result<crate::api::respond::AuthChallengeBuilder> {
        let coord = self.coordinator.clone().ok_or_else(|| {
            SessionError::InvalidInput(
                "IncomingRequest.challenge_builder() requires a coordinator hook".to_string(),
            )
        })?;
        let lifecycle_handle = self.lifecycle_handle.clone().ok_or_else(|| {
            SessionError::InvalidInput(
                "IncomingRequest.challenge_builder() requires exact lifecycle authority"
                    .to_string(),
            )
        })?;
        Ok(crate::api::respond::AuthChallengeBuilder::new_exact(
            coord,
            self.call_id.clone(),
            self.method.clone(),
            scheme,
            lifecycle_handle,
        ))
    }

    /// Authenticate this inbound request with a UAS SIP auth service.
    ///
    /// Pass [`SipAuthService`](crate::SipAuthService) for Digest, Bearer,
    /// Basic, AKA, or multi-challenge validation. Pass
    /// [`SipDigestAuthService`](crate::SipDigestAuthService) for Digest-only
    /// compatibility.
    pub async fn authenticate_with<A>(
        &self,
        auth: &A,
    ) -> Result<<A as crate::auth::SipIncomingAuthenticator>::Decision>
    where
        A: crate::auth::SipIncomingAuthenticator + Sync,
    {
        let request = self.request.as_ref().ok_or_else(|| {
            SessionError::InvalidInput(
                "IncomingRequest.authenticate_with() requires a parsed inbound request".to_string(),
            )
        })?;
        let (authorization, source) = selected_authorization(request);
        let request_uri = request.uri().to_string();
        let body = if request.body().is_empty() {
            None
        } else {
            Some(request.body())
        };
        let transport = effective_transport_security_context(Some(request), &self.transport);
        auth.authenticate_incoming_with_transport_context(
            authorization.as_deref(),
            self.method.as_str(),
            &request_uri,
            body,
            source,
            &transport,
        )
        .await
    }

    /// Rehydrate the coordinator hook so response builders can
    /// dispatch. Called by the surface consumer (CallbackPeer /
    /// StreamPeer) on every inbound event before exposing the
    /// `IncomingRequest` to application code.
    pub(crate) fn set_coordinator_captured(
        &mut self,
        coord: Arc<UnifiedCoordinator>,
        lifecycle_handle: Option<SessionRegistryHandle>,
    ) -> crate::api::unified::ExactResponseRegistration {
        if lifecycle_handle.is_some() {
            self.lifecycle_handle = lifecycle_handle;
        }
        if let Some(obligation) = &self.response_obligation {
            let Some(lifecycle_handle) = self.lifecycle_handle.as_ref() else {
                return crate::api::unified::ExactResponseRegistration::Collision;
            };
            if !obligation.bind_lifecycle_handle(lifecycle_handle) {
                return crate::api::unified::ExactResponseRegistration::Collision;
            }
            obligation.attach_coordinator(&coord);
            let registration = coord.register_exact_response_obligation(Arc::clone(obligation));
            self.coordinator = Some(coord);
            if !matches!(
                registration,
                crate::api::unified::ExactResponseRegistration::Registered
            ) {
                return registration;
            }
        } else {
            self.coordinator = Some(coord);
        }
        crate::api::unified::ExactResponseRegistration::Registered
    }

    pub(crate) fn set_response_transaction(
        &mut self,
        transaction: rvoip_sip_dialog::transaction::TransactionKey,
    ) {
        self.response_obligation = Some(Arc::new(ExactResponseObligation::new(
            self.call_id.clone(),
            transaction.clone(),
        )));
        self.response_transaction = Some(transaction);
    }

    pub(crate) fn clear_response_capability(&mut self) {
        self.response_transaction = None;
        self.response_obligation = None;
        self.coordinator = None;
        self.lifecycle_handle = None;
    }

    /// SIP_API_DESIGN_2 Phase E — construct an `IncomingRequest` from
    /// re-parsed bus bytes. Coordinator is `None` until the surface
    /// consumer (CallbackPeer / StreamPeer) rehydrates it via
    /// `set_coordinator` on dispatch.
    pub(crate) fn from_bus_request(
        call_id: CallId,
        from: String,
        to: String,
        method: rvoip_sip_core::types::Method,
        request: Arc<Request>,
    ) -> Self {
        Self {
            call_id,
            from,
            to,
            method,
            received_at: Instant::now(),
            request: Some(request),
            transport: crate::auth::SipTransportSecurityContext::unknown(),
            response_transaction: None,
            response_obligation: None,
            coordinator: None,
            lifecycle_handle: None,
        }
    }

    pub(crate) fn with_transport_context(
        mut self,
        transport: crate::auth::SipTransportSecurityContext,
    ) -> Self {
        self.transport = transport;
        self
    }

    /// Transport context used by auth policy decisions.
    pub fn transport_security_context(&self) -> &crate::auth::SipTransportSecurityContext {
        &self.transport
    }

    /// Underlying parsed [`Request`]. `None` when synthesized.
    pub fn raw_request(&self) -> Option<&Arc<Request>> {
        self.request.as_ref()
    }

    /// Zero-alloc header iteration — preferred over the boxed trait
    /// method on hot paths.
    pub fn headers_named_iter<'a>(
        &'a self,
        name: &'a HeaderName,
    ) -> impl Iterator<Item = &'a TypedHeader> + 'a {
        let slice: &[TypedHeader] = match &self.request {
            Some(r) => &r.headers[..],
            None => &[],
        };
        headers_named_slice(slice, name)
    }
}

impl SipHeaderView for IncomingRequest {
    fn header(&self, name: &HeaderName) -> Option<&TypedHeader> {
        self.request
            .as_ref()
            .and_then(|r| header_slice(&r.headers[..], name))
    }
    fn headers_named<'a>(
        &'a self,
        name: &HeaderName,
    ) -> Box<dyn Iterator<Item = &'a TypedHeader> + 'a> {
        match &self.request {
            Some(r) => {
                let name = name.clone();
                Box::new(
                    r.headers.iter().filter(move |h| {
                        crate::api::headers::view::header_name_eq(&h.name(), &name)
                    }),
                )
            }
            None => Box::new(std::iter::empty()),
        }
    }
    fn headers<'a>(&'a self) -> Box<dyn Iterator<Item = &'a TypedHeader> + 'a> {
        match &self.request {
            Some(r) => Box::new(r.headers.iter()),
            None => Box::new(std::iter::empty()),
        }
    }
    fn header_names(&self) -> Vec<HeaderName> {
        match &self.request {
            Some(r) => header_names_slice(&r.headers[..]),
            None => Vec::new(),
        }
    }
}

/// An inbound SIP response — 1xx provisional, 2xx success, 3xx
/// redirect, 4xx-6xx final. Carries the parsed [`Response`] when
/// available; used by B2BUA flows for downstream carry-through of
/// `Allow` / `Supported` / `Server` / `Session-Expires`, redirect
/// handling, and final-failure inspection (`Retry-After`, `Warning`,
/// `Reason`).
#[derive(Clone)]
pub struct IncomingResponse {
    /// The session the response belongs to.
    pub call_id: CallId,
    /// SIP status code (100, 180, 200, 302, 404, …).
    pub status_code: u16,
    /// Reason phrase from the status line.
    pub reason_phrase: String,
    /// SDP body on the response, if any.
    pub sdp: Option<String>,
    /// When the response arrived.
    pub received_at: Instant,
    /// The parsed inbound response, when available.
    pub(crate) response: Option<Arc<Response>>,
}

impl std::fmt::Debug for IncomingResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IncomingResponse")
            .field("status_code", &self.status_code)
            .field("reason_phrase_bytes", &self.reason_phrase.len())
            .field("sdp_present", &self.sdp.is_some())
            .field("sdp_bytes", &self.sdp.as_ref().map_or(0, String::len))
            .field("raw_response_present", &self.response.is_some())
            .field(
                "raw_response_header_count",
                &self
                    .response
                    .as_ref()
                    .map_or(0, |response| response.headers.len()),
            )
            .field(
                "raw_response_body_bytes",
                &self
                    .response
                    .as_ref()
                    .map_or(0, |response| response.body().len()),
            )
            .finish()
    }
}

impl IncomingResponse {
    /// Synthesize an `IncomingResponse` without a parsed body. Used by
    /// the legacy bus path until Phase A's bus enrichment fully lands.
    pub(crate) fn synthetic(
        call_id: CallId,
        status_code: u16,
        reason_phrase: String,
        sdp: Option<String>,
    ) -> Self {
        Self {
            call_id,
            status_code,
            reason_phrase,
            sdp,
            received_at: Instant::now(),
            response: None,
        }
    }

    /// Construct an `IncomingResponse` carrying a parsed response.
    pub(crate) fn with_response(
        call_id: CallId,
        status_code: u16,
        reason_phrase: String,
        sdp: Option<String>,
        response: Arc<Response>,
    ) -> Self {
        Self {
            call_id,
            status_code,
            reason_phrase,
            sdp,
            received_at: Instant::now(),
            response: Some(response),
        }
    }

    /// Underlying parsed [`Response`]. `None` until Phase A's bus
    /// enrichment is wired up for this code path.
    pub fn raw_response(&self) -> Option<&Arc<Response>> {
        self.response.as_ref()
    }

    /// Zero-alloc header iteration.
    pub fn headers_named_iter<'a>(
        &'a self,
        name: &'a HeaderName,
    ) -> impl Iterator<Item = &'a TypedHeader> + 'a {
        let slice: &[TypedHeader] = match &self.response {
            Some(r) => &r.headers[..],
            None => &[],
        };
        headers_named_slice(slice, name)
    }

    /// True when this is a 1xx response.
    pub fn is_provisional(&self) -> bool {
        (100..200).contains(&self.status_code)
    }

    /// True when this carries an RFC 3262 reliable provisional marker
    /// (Require: 100rel + RSeq). Inspects the parsed response when
    /// available; falls back to `false` when synthesized.
    pub fn is_reliable_provisional(&self) -> bool {
        if !self.is_provisional() {
            return false;
        }
        let Some(resp) = &self.response else {
            return false;
        };
        let mut has_require_100rel = false;
        let mut has_rseq = false;
        for h in &resp.headers {
            match h {
                TypedHeader::Require(req)
                    if req
                        .option_tags
                        .iter()
                        .any(|s| s.eq_ignore_ascii_case("100rel")) =>
                {
                    has_require_100rel = true;
                }
                TypedHeader::RSeq(_) => has_rseq = true,
                _ => {}
            }
        }
        has_require_100rel && has_rseq
    }
}

impl SipHeaderView for IncomingResponse {
    fn header(&self, name: &HeaderName) -> Option<&TypedHeader> {
        self.response
            .as_ref()
            .and_then(|r| header_slice(&r.headers[..], name))
    }
    fn headers_named<'a>(
        &'a self,
        name: &HeaderName,
    ) -> Box<dyn Iterator<Item = &'a TypedHeader> + 'a> {
        match &self.response {
            Some(r) => {
                let name = name.clone();
                Box::new(
                    r.headers.iter().filter(move |h| {
                        crate::api::headers::view::header_name_eq(&h.name(), &name)
                    }),
                )
            }
            None => Box::new(std::iter::empty()),
        }
    }
    fn headers<'a>(&'a self) -> Box<dyn Iterator<Item = &'a TypedHeader> + 'a> {
        match &self.response {
            Some(r) => Box::new(r.headers.iter()),
            None => Box::new(std::iter::empty()),
        }
    }
    fn header_names(&self) -> Vec<HeaderName> {
        match &self.response {
            Some(r) => header_names_slice(&r.headers[..]),
            None => Vec::new(),
        }
    }
}

/// Inbound SIP REGISTER on registrar surfaces.
///
/// Wraps the parsed REGISTER request and exposes it through
/// [`SipHeaderView`] plus dedicated convenience accessors for AOR /
/// Contact / Expires that registrar code reaches for first.
#[derive(Clone)]
pub struct IncomingRegister {
    // Manual `Debug` impl below skips the `coordinator` field since
    // `UnifiedCoordinator` does not derive `Debug`.
    /// Transaction key for the inbound REGISTER (used for response
    /// authoring).
    pub transaction_id: String,
    /// `From` URI (the registering AOR).
    pub from_uri: String,
    /// `To` URI (the registrar / target AOR).
    pub to_uri: String,
    /// First `Contact` URI on the wire. Multi-Contact lookups go
    /// through [`SipHeaderView`].
    pub contact_uri: String,
    /// Requested `Expires` (header or Contact-param). 0 means unregister.
    pub expires: u32,
    /// Raw `Authorization:` header value if present.
    pub authorization: Option<String>,
    /// `Call-ID` from the request.
    pub call_id_header: String,
    /// When the REGISTER arrived.
    pub received_at: Instant,
    /// The parsed inbound REGISTER request, when available.
    pub(crate) request: Option<Arc<Request>>,
    /// Transport context for auth policy decisions.
    pub(crate) transport: crate::auth::SipTransportSecurityContext,
    /// Optional hook back into the coordinator so
    /// `RegisterResponseBuilder.send()` can answer the exact inbound
    /// REGISTER transaction. `None` when the wrapper was synthesized for
    /// tests or by an external registrar that authors responses directly.
    pub(crate) coordinator: Option<Arc<UnifiedCoordinator>>,
    /// Single-writer deadline owner for an application-authored REGISTER
    /// response. Only the private causal control copy retains it.
    pub(crate) response_obligation: Option<Arc<ExactResponseObligation>>,
    /// Whether an owning API receiver may attach response authority. Public
    /// observational projections clear this bit so cloning or redispatching
    /// an observation cannot recreate a second REGISTER response owner.
    response_capability: bool,
    /// Marks the stripped public copy emitted beside a private control
    /// delivery. The owning receiver suppresses only this copy; ordinary
    /// built-in-registrar observations remain visible to applications.
    control_observation: bool,
}

impl std::fmt::Debug for IncomingRegister {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IncomingRegister")
            .field("expires", &self.expires)
            .field("authorization_present", &self.authorization.is_some())
            .field(
                "authorization_bytes",
                &self.authorization.as_ref().map_or(0, String::len),
            )
            .field("raw_request_present", &self.request.is_some())
            .field(
                "raw_request_header_count",
                &self
                    .request
                    .as_ref()
                    .map_or(0, |request| request.headers.len()),
            )
            .field(
                "raw_request_body_bytes",
                &self
                    .request
                    .as_ref()
                    .map_or(0, |request| request.body().len()),
            )
            .field("coordinator_present", &self.coordinator.is_some())
            .finish()
    }
}

impl IncomingRegister {
    // ─────────────────────────────────────────────────────────────────
    // SIP_API_DESIGN_2 Phase D — registrar response builder entries.
    // ─────────────────────────────────────────────────────────────────

    /// Begin a `RegisterResponseBuilder` for the inbound REGISTER.
    pub fn accept_builder(&self) -> crate::api::respond::RegisterResponseBuilder {
        crate::api::respond::RegisterResponseBuilder::new(
            self.transaction_id.clone(),
            self.coordinator.clone(),
        )
        .with_response_obligation(self.response_obligation.clone())
    }

    /// Begin a 401 / 407 auth challenge for the inbound REGISTER.
    /// The matching dispatch uses the same exact transaction path as
    /// `accept_builder`, so a registrar can author both 200 OK and 401
    /// through one consistent surface.
    pub fn challenge_builder(
        &self,
        scheme: crate::api::respond::AuthScheme,
    ) -> crate::api::respond::RegisterResponseBuilder {
        crate::api::respond::RegisterResponseBuilder::new_challenge(
            self.transaction_id.clone(),
            self.coordinator.clone(),
            scheme,
        )
        .with_response_obligation(self.response_obligation.clone())
    }

    /// Authenticate this inbound REGISTER with a UAS SIP auth service.
    ///
    /// Pass [`SipAuthService`](crate::SipAuthService) for Digest, Bearer,
    /// Basic, AKA, or multi-challenge validation. Pass
    /// [`SipDigestAuthService`](crate::SipDigestAuthService) for Digest-only
    /// compatibility.
    pub async fn authenticate_with<A>(
        &self,
        auth: &A,
    ) -> Result<<A as crate::auth::SipIncomingAuthenticator>::Decision>
    where
        A: crate::auth::SipIncomingAuthenticator + Sync,
    {
        let request_uri = self
            .request
            .as_ref()
            .map(|request| request.uri().to_string())
            .unwrap_or_else(|| self.to_uri.clone());
        let body = self
            .request
            .as_ref()
            .and_then(|request| (!request.body().is_empty()).then(|| request.body()));
        let (authorization, source) = self
            .request
            .as_ref()
            .map(|request| selected_authorization(request))
            .unwrap_or_else(|| {
                (
                    self.authorization.clone(),
                    crate::auth::SipAuthSource::Origin,
                )
            });
        let transport =
            effective_transport_security_context(self.request.as_deref(), &self.transport);
        auth.authenticate_incoming_with_transport_context(
            authorization.as_deref(),
            "REGISTER",
            &request_uri,
            body,
            source,
            &transport,
        )
        .await
    }

    /// Begin a generic non-2xx REGISTER response (404, 423 Interval
    /// Too Brief with `Min-Expires`, 503 Service Unavailable, …).
    pub fn reject_builder(&self, status: u16) -> crate::api::respond::RegisterResponseBuilder {
        crate::api::respond::RegisterResponseBuilder::new_reject(
            self.transaction_id.clone(),
            self.coordinator.clone(),
            status,
        )
        .with_response_obligation(self.response_obligation.clone())
    }

    /// Synthesize without a parsed request. Used by the legacy bus
    /// path until enrichment fully lands.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn synthetic(
        transaction_id: String,
        from_uri: String,
        to_uri: String,
        contact_uri: String,
        expires: u32,
        authorization: Option<String>,
        call_id_header: String,
    ) -> Self {
        Self {
            transaction_id,
            from_uri,
            to_uri,
            contact_uri,
            expires,
            authorization,
            call_id_header,
            received_at: Instant::now(),
            request: None,
            transport: crate::auth::SipTransportSecurityContext::unknown(),
            coordinator: None,
            response_obligation: None,
            response_capability: true,
            control_observation: false,
        }
    }

    /// Construct from a parsed inbound REGISTER request.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn with_request(
        transaction_id: String,
        from_uri: String,
        to_uri: String,
        contact_uri: String,
        expires: u32,
        authorization: Option<String>,
        call_id_header: String,
        request: Arc<Request>,
    ) -> Self {
        Self {
            transaction_id,
            from_uri,
            to_uri,
            contact_uri,
            expires,
            authorization,
            call_id_header,
            received_at: Instant::now(),
            request: Some(request),
            transport: crate::auth::SipTransportSecurityContext::unknown(),
            coordinator: None,
            response_obligation: None,
            response_capability: true,
            control_observation: false,
        }
    }

    /// Same as [`with_request`] but threads the coordinator handle so the
    /// response builder can answer through rvoip-sip-dialog's exact server
    /// transaction.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn with_request_and_coordinator(
        transaction_id: String,
        from_uri: String,
        to_uri: String,
        contact_uri: String,
        expires: u32,
        authorization: Option<String>,
        call_id_header: String,
        request: Arc<Request>,
        coordinator: Arc<UnifiedCoordinator>,
    ) -> Self {
        Self {
            transaction_id,
            from_uri,
            to_uri,
            contact_uri,
            expires,
            authorization,
            call_id_header,
            received_at: Instant::now(),
            request: Some(request),
            transport: crate::auth::SipTransportSecurityContext::unknown(),
            coordinator: Some(coordinator),
            response_obligation: None,
            response_capability: true,
            control_observation: false,
        }
    }

    pub(crate) fn with_transport_context(
        mut self,
        transport: crate::auth::SipTransportSecurityContext,
    ) -> Self {
        self.transport = transport;
        self
    }

    /// Transport context used by auth policy decisions.
    pub fn transport_security_context(&self) -> &crate::auth::SipTransportSecurityContext {
        &self.transport
    }

    /// Underlying parsed [`Request`].
    pub fn raw_request(&self) -> Option<&Arc<Request>> {
        self.request.as_ref()
    }

    /// Zero-alloc header iteration.
    pub fn headers_named_iter<'a>(
        &'a self,
        name: &'a HeaderName,
    ) -> impl Iterator<Item = &'a TypedHeader> + 'a {
        let slice: &[TypedHeader] = match &self.request {
            Some(r) => &r.headers[..],
            None => &[],
        };
        headers_named_slice(slice, name)
    }

    /// Set the coordinator hook so `accept_builder()` /
    /// `challenge_builder()` / `reject_builder()` can dispatch through the
    /// exact inbound REGISTER transaction. Called immediately before the
    /// `IncomingRegister` reaches the application handler.
    pub fn set_coordinator(&mut self, coordinator: Arc<UnifiedCoordinator>) {
        if self.response_capability {
            self.coordinator = Some(coordinator);
        }
    }

    /// Install the standalone exact-transaction owner before the causal
    /// delivery is positively acknowledged. Its managed deadline authors 503
    /// if an accepted application handler never claims a response builder.
    pub(crate) fn install_response_obligation(
        &mut self,
        coordinator: Arc<UnifiedCoordinator>,
    ) -> Result<crate::api::unified::ExactResponseRegistration> {
        if !self.response_capability {
            return Err(SessionError::InvalidInput(
                "observational REGISTER cannot acquire response authority".to_string(),
            ));
        }
        let transaction = self
            .transaction_id
            .parse::<rvoip_sip_dialog::transaction::TransactionKey>()
            .map_err(|_| {
                SessionError::InvalidInput(
                    "IncomingRegister has an invalid exact transaction identifier".to_string(),
                )
            })?;
        if !transaction.is_server()
            || transaction.method() != &rvoip_sip_core::types::Method::Register
        {
            return Err(SessionError::InvalidInput(
                "IncomingRegister response authority requires a server REGISTER transaction"
                    .to_string(),
            ));
        }
        if let Some(request) = self.request.as_ref() {
            let wire_transaction = rvoip_sip_dialog::transaction::TransactionKey::from_request(
                request,
            )
            .ok_or_else(|| {
                SessionError::InvalidInput(
                    "IncomingRegister wire request has no exact transaction".to_string(),
                )
            })?;
            if wire_transaction != transaction {
                return Err(SessionError::InvalidInput(
                    "IncomingRegister event transaction does not match the wire request"
                        .to_string(),
                ));
            }
        }

        let obligation = Arc::new(ExactResponseObligation::new_standalone(transaction, 503));
        obligation.attach_coordinator(&coordinator);
        let registration = coordinator.register_exact_response_obligation(Arc::clone(&obligation));
        if matches!(
            registration,
            crate::api::unified::ExactResponseRegistration::Registered
        ) {
            self.coordinator = Some(coordinator);
            self.response_obligation = Some(obligation);
        }
        Ok(registration)
    }

    /// Permanently strip response authority from an observational copy.
    pub(crate) fn clear_response_capability(&mut self) {
        self.coordinator = None;
        self.response_obligation = None;
        self.response_capability = false;
    }

    /// Identify the stripped observation paired with a private control event.
    pub(crate) fn mark_control_observation(&mut self) {
        self.clear_response_capability();
        self.control_observation = true;
    }

    #[cfg(test)]
    pub(crate) fn is_control_observation(&self) -> bool {
        self.control_observation
    }
}

impl SipHeaderView for IncomingRegister {
    fn header(&self, name: &HeaderName) -> Option<&TypedHeader> {
        self.request
            .as_ref()
            .and_then(|r| header_slice(&r.headers[..], name))
    }
    fn headers_named<'a>(
        &'a self,
        name: &HeaderName,
    ) -> Box<dyn Iterator<Item = &'a TypedHeader> + 'a> {
        match &self.request {
            Some(r) => {
                let name = name.clone();
                Box::new(
                    r.headers.iter().filter(move |h| {
                        crate::api::headers::view::header_name_eq(&h.name(), &name)
                    }),
                )
            }
            None => Box::new(std::iter::empty()),
        }
    }
    fn headers<'a>(&'a self) -> Box<dyn Iterator<Item = &'a TypedHeader> + 'a> {
        match &self.request {
            Some(r) => Box::new(r.headers.iter()),
            None => Box::new(std::iter::empty()),
        }
    }
    fn header_names(&self) -> Vec<HeaderName> {
        match &self.request {
            Some(r) => header_names_slice(&r.headers[..]),
            None => Vec::new(),
        }
    }
}

fn selected_authorization(request: &Request) -> (Option<String>, crate::auth::SipAuthSource) {
    if let Some(value) = request.raw_header_value(&HeaderName::Authorization) {
        return (Some(value), crate::auth::SipAuthSource::Origin);
    }
    if let Some(value) = request.raw_header_value(&HeaderName::ProxyAuthorization) {
        return (Some(value), crate::auth::SipAuthSource::Proxy);
    }
    (None, crate::auth::SipAuthSource::Origin)
}

fn request_transport_security_context(
    request: &Request,
) -> crate::auth::SipTransportSecurityContext {
    crate::auth::SipTransportSecurityContext::from_request_uri_hint(&request.uri().to_string())
}

fn effective_transport_security_context(
    request: Option<&Request>,
    transport: &crate::auth::SipTransportSecurityContext,
) -> crate::auth::SipTransportSecurityContext {
    if transport.secure
        || transport.transport.is_some()
        || transport.local_addr.is_some()
        || transport.remote_addr.is_some()
    {
        transport.clone()
    } else {
        request
            .map(request_transport_security_context)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_header_keys_use_canonical_wire_spelling_without_debug() {
        assert_eq!(legacy_header_key(&HeaderName::CallId), "call-id");
        assert_eq!(
            legacy_header_key(&HeaderName::Other("i".into())),
            "call-id",
            "compact standard names must expand before entering the legacy map"
        );
        assert_eq!(
            legacy_header_key(&HeaderName::Other("X-Vendor-Trace".into())),
            "x-vendor-trace"
        );

        let debug = format!("{:?}", HeaderName::CallId);
        assert_ne!(
            legacy_header_key(&HeaderName::CallId),
            debug.to_ascii_lowercase()
        );
    }
    use crate::api::unified::Config;
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

    async fn publish_synthetic(coordinator: &UnifiedCoordinator, event: Event) {
        coordinator
            .publish_app_event_for_test(event)
            .await
            .expect("publish synthetic event");
    }

    #[test]
    fn incoming_wrapper_debug_never_exposes_signaling_payloads() {
        const SECRET: &str = "incoming-wrapper-secret-canary";
        const METHOD: &str = "X-INCOMING-METHOD-SECRET-CANARY";
        let request = Request::new(
            rvoip_sip_core::types::Method::Refer,
            rvoip_sip_core::types::Uri::sip("incoming-wrapper-secret-canary.example"),
        )
        .with_header(TypedHeader::Other(
            HeaderName::Authorization,
            rvoip_sip_core::types::headers::HeaderValue::Raw(SECRET.as_bytes().to_vec()),
        ))
        .with_body(SECRET);
        let incoming_request = IncomingRequest::from_bus_request(
            crate::state_table::types::SessionId(SECRET.into()),
            SECRET.into(),
            SECRET.into(),
            rvoip_sip_core::types::Method::Extension(METHOD.into()),
            Arc::new(request),
        );
        let incoming_response = IncomingResponse::synthetic(
            crate::state_table::types::SessionId(SECRET.into()),
            401,
            SECRET.into(),
            Some(SECRET.into()),
        );
        let incoming_register = IncomingRegister::synthetic(
            SECRET.into(),
            SECRET.into(),
            SECRET.into(),
            SECRET.into(),
            300,
            Some(SECRET.into()),
            SECRET.into(),
        );

        for rendered in [
            format!("{incoming_request:?}"),
            format!("{incoming_response:?}"),
            format!("{incoming_register:?}"),
        ] {
            assert!(
                !rendered.contains(SECRET),
                "debug leaked secret: {rendered}"
            );
            assert!(
                !rendered.contains(METHOD),
                "debug leaked method: {rendered}"
            );
        }
    }

    #[test]
    fn register_observation_permanently_strips_response_authority() {
        let mut incoming = IncomingRegister::synthetic(
            "z9hG4bK-register-observation:REGISTER:server".into(),
            "sip:alice@example.test".into(),
            "sip:alice@example.test".into(),
            "sip:alice@192.0.2.10".into(),
            300,
            None,
            "register-observation@example.test".into(),
        );
        incoming.response_obligation = Some(Arc::new(ExactResponseObligation::new_standalone(
            rvoip_sip_dialog::transaction::TransactionKey::new(
                "z9hG4bK-register-observation".into(),
                rvoip_sip_core::Method::Register,
                true,
            ),
            503,
        )));

        incoming.mark_control_observation();
        assert!(incoming.response_obligation.is_none());
        assert!(!incoming.response_capability);
        assert!(incoming.is_control_observation());
    }

    #[test]
    fn incoming_response_rejects_wrong_branch_before_dispatch() {
        let raw = b"INFO sip:bob@example.test SIP/2.0\r\n\
                    Via: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-wire-branch\r\n\
                    From: <sip:alice@example.test>;tag=from-tag\r\n\
                    To: <sip:bob@example.test>;tag=to-tag\r\n\
                    Call-ID: response-correlation\r\n\
                    CSeq: 2 INFO\r\n\
                    Content-Length: 0\r\n\r\n";
        let request = match rvoip_sip_core::parse_message(raw).expect("parse INFO") {
            rvoip_sip_core::Message::Request(request) => request,
            other => panic!("expected request, got {other:?}"),
        };
        let mut incoming = IncomingRequest::from_bus_request(
            crate::state_table::types::SessionId("response-correlation".into()),
            "sip:alice@example.test".into(),
            "sip:bob@example.test".into(),
            rvoip_sip_core::types::Method::Info,
            Arc::new(request),
        );
        incoming.set_response_transaction(rvoip_sip_dialog::transaction::TransactionKey::new(
            "z9hG4bK-wrong-branch".into(),
            rvoip_sip_core::types::Method::Info,
            true,
        ));

        assert!(matches!(
            incoming.respond(200),
            Err(SessionError::InvalidInput(message))
                if message.contains("does not match the wire request")
        ));
    }

    #[test]
    fn cancelled_exact_response_claim_returns_to_retryable_state() {
        let obligation = Arc::new(ExactResponseObligation::new(
            CallId::new(),
            rvoip_sip_dialog::transaction::TransactionKey::new(
                "z9hG4bK-cancelled-exact-response".into(),
                rvoip_sip_core::types::Method::Info,
                true,
            ),
        ));

        let cancelled = obligation.claim().expect("first exact response claim");
        drop(cancelled);
        let retry = obligation
            .claim()
            .expect("cancelled exact response claim was released by RAII");
        retry.release_after_failure();
    }

    #[test]
    fn incoming_source_keeps_request_response_and_register_on_manual_debug() {
        let source = include_str!("incoming.rs");
        for declaration in [
            "pub struct IncomingRequest",
            "pub struct IncomingResponse",
            "pub struct IncomingRegister",
        ] {
            let declaration_offset = source
                .find(declaration)
                .unwrap_or_else(|| panic!("missing declaration {declaration}"));
            let prefix = &source[..declaration_offset];
            let derive_offset = prefix
                .rfind("#[derive(")
                .unwrap_or_else(|| panic!("missing derive for {declaration}"));
            assert!(
                !prefix[derive_offset..].contains("Debug"),
                "{declaration} regained derived Debug"
            );
        }
    }

    #[tokio::test]
    async fn incoming_request_auth_uses_transport_context_over_uri_hint() {
        let token = BASE64_STANDARD.encode("alice:secret");
        let raw = format!(
            "OPTIONS sip:bob@example.test SIP/2.0\r\n\
             Via: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-test\r\n\
             From: <sip:alice@example.test>;tag=from-tag\r\n\
             To: <sip:bob@example.test>\r\n\
             Call-ID: incoming-auth-context\r\n\
             CSeq: 1 OPTIONS\r\n\
             Authorization: Basic {token}\r\n\
             Content-Length: 0\r\n\r\n"
        );
        let request = match rvoip_sip_core::parse_message(raw.as_bytes()).expect("parse request") {
            rvoip_sip_core::Message::Request(request) => request,
            other => panic!("expected request, got {other:?}"),
        };
        let incoming = IncomingRequest::from_bus_request(
            crate::state_table::types::SessionId("incoming-auth".to_string()),
            "sip:alice@example.test".to_string(),
            "sip:bob@example.test".to_string(),
            rvoip_sip_core::types::Method::Options,
            Arc::new(request),
        )
        .with_transport_context(
            crate::auth::SipTransportSecurityContext::from_transport_name("WSS"),
        );

        let mut service = crate::auth::SipAuthService::new().with_basic_realm("legacy");
        service.add_basic_user("alice", "secret");

        let decision = incoming
            .authenticate_with(&service)
            .await
            .expect("incoming auth");

        assert!(matches!(
            decision,
            crate::auth::SipAuthDecision::Authorized(crate::auth::AuthIdentity {
                scheme: crate::auth::SipAuthScheme::Basic,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn deferred_guard_reject_marks_shared_resolution_before_watchdog() {
        let coordinator = UnifiedCoordinator::new(Config::local("guard-test", 35990))
            .await
            .expect("coordinator starts");
        let guard = IncomingCallGuard::new(CallId::new(), coordinator, Duration::from_millis(25));
        let resolved = guard.resolved.clone();

        guard.reject(486, "Busy Here");
        tokio::time::sleep(Duration::from_millis(50)).await;

        assert!(
            resolved.load(Ordering::SeqCst),
            "explicit reject should resolve the guard before the watchdog fires"
        );
    }

    #[tokio::test]
    async fn deferred_guard_accept_attempt_marks_shared_resolution_before_watchdog() {
        let coordinator = UnifiedCoordinator::new(Config::local("guard-test", 35991))
            .await
            .expect("coordinator starts");
        let guard = IncomingCallGuard::new(CallId::new(), coordinator, Duration::from_millis(25));
        let resolved = guard.resolved.clone();

        let result = guard.accept().await;
        assert!(
            result.is_err(),
            "fake guard has no backing session, so accept should surface that error"
        );

        tokio::time::sleep(Duration::from_millis(50)).await;

        assert!(
            resolved.load(Ordering::SeqCst),
            "accept should resolve the guard before the watchdog can auto-reject"
        );
    }

    #[tokio::test]
    async fn deferred_guard_watchdog_is_cancelled_and_cannot_cross_raw_id_reuse() {
        let coordinator = UnifiedCoordinator::new(Config::local("guard-test", 35989))
            .await
            .expect("coordinator starts");
        let call_id = CallId::from("deferred-guard-reuse");
        coordinator
            .helpers
            .create_session(
                call_id.clone(),
                "sip:callee@example.test".to_string(),
                "sip:caller@example.test".to_string(),
                crate::state_table::types::Role::UAS,
            )
            .await
            .expect("create original exact incoming lifetime");
        let store = Arc::clone(&coordinator.helpers.state_machine.store);
        let original = store
            .lifecycle_handle(&call_id)
            .expect("original exact registry handle");
        store
            .update_session_exact_with(&original, None, |session| {
                session.call_state = CallState::Ringing;
            })
            .expect("mark original call ringing");

        let guard = IncomingCallGuard::new(
            call_id.clone(),
            Arc::clone(&coordinator),
            Duration::from_millis(200),
        );
        let resolved = Arc::clone(&guard.resolved);

        store
            .remove_session_exact(&original)
            .await
            .expect("teardown cancels and joins the exact watchdog");
        assert!(
            !resolved.load(Ordering::SeqCst),
            "lifecycle cancellation must not resolve the application guard"
        );
        assert!(
            store.authority().elapse_reuse_horizon_for_test(&call_id),
            "expire the production anti-reuse fence through the test clock seam"
        );

        coordinator
            .helpers
            .create_session(
                call_id.clone(),
                "sip:replacement@example.test".to_string(),
                "sip:caller@example.test".to_string(),
                crate::state_table::types::Role::UAS,
            )
            .await
            .expect("reuse the raw identifier for a later exact lifetime");
        let replacement = store
            .lifecycle_handle(&call_id)
            .expect("replacement exact registry handle");
        assert_ne!(original, replacement);
        store
            .update_session_exact_with(&replacement, None, |session| {
                session.call_state = CallState::Ringing;
            })
            .expect("mark replacement call ringing");

        tokio::time::sleep(Duration::from_millis(250)).await;
        assert!(
            !resolved.load(Ordering::SeqCst),
            "the cancelled watchdog must stay inert after its old deadline"
        );
        assert_eq!(
            store
                .get_session_exact(&replacement)
                .await
                .expect("replacement remains live")
                .call_state,
            CallState::Ringing,
            "the old watchdog must not reject the reused identifier"
        );

        guard.abandon();
        coordinator.shutdown();
    }

    #[tokio::test]
    async fn retired_incoming_objects_cannot_resolve_reused_generation() {
        let coordinator = UnifiedCoordinator::new(Config::local("incoming-reuse-test", 35988))
            .await
            .expect("coordinator starts");
        let call_id = CallId::from("incoming-object-reuse");
        coordinator
            .helpers
            .create_session(
                call_id.clone(),
                "sip:callee@example.test".to_string(),
                "sip:caller@example.test".to_string(),
                crate::state_table::types::Role::UAS,
            )
            .await
            .expect("create generation A");
        let store = Arc::clone(&coordinator.helpers.state_machine.store);
        let generation_a = store
            .lifecycle_handle(&call_id)
            .expect("generation A exact handle");
        store
            .update_session_exact_with(&generation_a, None, |session| {
                session.call_state = CallState::Ringing;
            })
            .expect("mark generation A ringing");

        let make_incoming = || {
            IncomingCall::new_exact(
                call_id.clone(),
                "sip:caller@example.test".to_string(),
                "sip:callee@example.test".to_string(),
                None,
                Arc::clone(&coordinator),
                generation_a.clone(),
            )
        };
        let accept_a = make_incoming();
        let redirect_a = make_incoming();
        let reject_a = make_incoming();
        let panic_drop_a = make_incoming();
        let accept_guard_a = make_incoming().defer(Duration::from_secs(5));
        let reject_guard_a = make_incoming().defer(Duration::from_secs(5));

        store
            .remove_session_exact(&generation_a)
            .await
            .expect("retire generation A and cancel its deferred watchdogs");
        assert!(
            store.authority().elapse_reuse_horizon_for_test(&call_id),
            "expire anti-reuse horizon"
        );
        coordinator
            .helpers
            .create_session(
                call_id.clone(),
                "sip:replacement@example.test".to_string(),
                "sip:caller@example.test".to_string(),
                crate::state_table::types::Role::UAS,
            )
            .await
            .expect("create generation B");
        let generation_b = store
            .lifecycle_handle(&call_id)
            .expect("generation B exact handle");
        assert_ne!(generation_a, generation_b);
        store
            .update_session_exact_with(&generation_b, None, |session| {
                session.call_state = CallState::Ringing;
            })
            .expect("mark generation B ringing");

        assert!(accept_a.accept().await.is_err());
        assert!(redirect_a
            .redirect_to("sip:elsewhere@example.test")
            .await
            .is_err());
        assert!(accept_guard_a.accept().await.is_err());
        reject_a.reject(486, "Busy Here");
        reject_guard_a.reject(503, "Service Unavailable");
        let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _owned = panic_drop_a;
            panic!("exercise IncomingCall panic-drop safety net");
        }));
        assert!(panic_result.is_err());

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            store
                .get_session_exact(&generation_b)
                .await
                .expect("generation B remains live")
                .call_state,
            CallState::Ringing,
            "generation-A accept/reject/redirect/guard/drop work must not mutate generation B"
        );

        coordinator.shutdown();
    }

    #[tokio::test]
    async fn retired_incoming_request_cannot_resolve_or_reject_reused_generation() {
        let coordinator =
            UnifiedCoordinator::new(Config::local("incoming-request-reuse-test", 35987))
                .await
                .expect("coordinator starts");
        let call_id = CallId::from("incoming-request-reuse");
        coordinator
            .helpers
            .create_session(
                call_id.clone(),
                "sip:callee@example.test".to_string(),
                "sip:caller@example.test".to_string(),
                crate::state_table::types::Role::UAS,
            )
            .await
            .expect("create generation A");
        let store = Arc::clone(&coordinator.helpers.state_machine.store);
        let generation_a = store
            .lifecycle_handle(&call_id)
            .expect("generation A exact handle");
        store
            .update_session_exact_with(&generation_a, None, |session| {
                session.call_state = CallState::Ringing;
            })
            .expect("mark generation A ringing");

        let request = Request::new(
            rvoip_sip_core::types::Method::Options,
            rvoip_sip_core::types::Uri::sip("callee.example.test"),
        );
        let mut incoming = IncomingRequest::from_bus_request(
            call_id.clone(),
            "sip:caller@example.test".to_string(),
            "sip:callee@example.test".to_string(),
            rvoip_sip_core::types::Method::Options,
            Arc::new(request),
        );
        assert!(matches!(
            incoming.set_coordinator_captured(Arc::clone(&coordinator), Some(generation_a.clone())),
            crate::api::unified::ExactResponseRegistration::Registered
        ));

        store
            .remove_session_exact(&generation_a)
            .await
            .expect("retire generation A");
        assert!(
            store.authority().elapse_reuse_horizon_for_test(&call_id),
            "expire anti-reuse horizon"
        );
        coordinator
            .helpers
            .create_session(
                call_id.clone(),
                "sip:replacement@example.test".to_string(),
                "sip:caller@example.test".to_string(),
                crate::state_table::types::Role::UAS,
            )
            .await
            .expect("create generation B");
        let generation_b = store
            .lifecycle_handle(&call_id)
            .expect("generation B exact handle");
        assert_ne!(generation_a, generation_b);
        store
            .update_session_exact_with(&generation_b, None, |session| {
                session.call_state = CallState::Ringing;
            })
            .expect("mark generation B ringing");

        assert!(
            incoming
                .session_handle()
                .expect("request retains generation A authority")
                .state()
                .await
                .is_err(),
            "the request-derived session handle must not re-resolve generation B"
        );
        assert!(
            incoming
                .respond_builder(486)
                .expect("exact generic response builder")
                .send()
                .await
                .is_err(),
            "a delayed generic response must fail against retired generation A"
        );
        assert!(
            incoming
                .challenge_builder(crate::api::respond::AuthScheme::Digest)
                .expect("exact challenge builder")
                .with_realm("example.test")
                .with_nonce("generation-a-nonce")
                .send()
                .await
                .is_err(),
            "a delayed auth challenge must fail against retired generation A"
        );
        assert_eq!(
            store
                .get_session_exact(&generation_b)
                .await
                .expect("generation B remains live")
                .call_state,
            CallState::Ringing,
            "generation-A request objects must not mutate generation B"
        );

        coordinator.shutdown();
    }

    #[tokio::test]
    async fn deferred_guard_wait_for_cancelled_observes_call_cancelled() {
        let coordinator = UnifiedCoordinator::new(Config::local("guard-test", 35992))
            .await
            .expect("coordinator starts");
        let call_id = CallId::new();
        let guard =
            IncomingCallGuard::new(call_id.clone(), coordinator.clone(), Duration::from_secs(5));
        let resolved = guard.resolved.clone();

        let waiter = tokio::spawn({
            let guard = guard;
            async move { guard.wait_for_cancelled(Some(Duration::from_secs(2))).await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;

        publish_synthetic(&coordinator, Event::CallCancelled { call_id }).await;

        waiter.await.unwrap().unwrap();
        assert!(resolved.load(Ordering::SeqCst));
        coordinator.shutdown();
    }

    #[tokio::test]
    async fn deferred_guard_wait_for_cancelled_errors_on_answer() {
        let coordinator = UnifiedCoordinator::new(Config::local("guard-test", 35993))
            .await
            .expect("coordinator starts");
        let call_id = CallId::new();
        let guard =
            IncomingCallGuard::new(call_id.clone(), coordinator.clone(), Duration::from_secs(5));
        let resolved = guard.resolved.clone();

        let waiter = tokio::spawn({
            let guard = guard;
            async move { guard.wait_for_cancelled(Some(Duration::from_secs(2))).await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;

        publish_synthetic(&coordinator, Event::CallAnswered { call_id, sdp: None }).await;

        let err = waiter.await.unwrap().unwrap_err();
        assert!(matches!(
            err,
            SessionError::Other(ref detail) if detail.contains("answered before cancellation")
        ));
        assert!(resolved.load(Ordering::SeqCst));
        coordinator.shutdown();
    }

    #[tokio::test]
    async fn deferred_guard_wait_for_cancelled_timeout_does_not_resolve_guard() {
        let coordinator = UnifiedCoordinator::new(Config::local("guard-test", 35994))
            .await
            .expect("coordinator starts");
        let guard =
            IncomingCallGuard::new(CallId::new(), coordinator.clone(), Duration::from_secs(5));
        let resolved = guard.resolved.clone();

        let err = guard
            .wait_for_cancelled(Some(Duration::from_millis(25)))
            .await
            .unwrap_err();

        assert!(
            matches!(err, SessionError::Timeout(_)),
            "caller timeout should surface as a timeout"
        );
        assert!(
            !resolved.load(Ordering::SeqCst),
            "caller timeout must not resolve or mutate the guard"
        );

        guard.abandon();
        assert!(
            resolved.load(Ordering::SeqCst),
            "abandon is the explicit local policy decision"
        );
        coordinator.shutdown();
    }

    #[tokio::test]
    async fn deferred_guard_abandon_resolves_without_sip_response() {
        let coordinator = UnifiedCoordinator::new(Config::local("guard-test", 35995))
            .await
            .expect("coordinator starts");
        let guard =
            IncomingCallGuard::new(CallId::new(), coordinator.clone(), Duration::from_secs(5));
        let resolved = guard.resolved.clone();

        guard.abandon();

        assert!(resolved.load(Ordering::SeqCst));
        coordinator.shutdown();
    }

    #[tokio::test]
    async fn redirect_with_contacts_rejects_non_3xx_status() {
        let coordinator = UnifiedCoordinator::new(Config::local("redirect-test", 35996))
            .await
            .expect("coordinator starts");
        let incoming = IncomingCall::new(
            CallId::new(),
            "sip:a@example.test".into(),
            "sip:b@example.test".into(),
            None,
            coordinator.clone(),
        );

        let err = incoming
            .redirect_with_contacts(486, ["sip:voicemail@example.test"])
            .await
            .unwrap_err();
        assert!(matches!(err, SessionError::InvalidInput(_)));
        coordinator.shutdown();
    }

    #[tokio::test]
    async fn redirect_with_contacts_rejects_empty_contacts() {
        let coordinator = UnifiedCoordinator::new(Config::local("redirect-test", 35997))
            .await
            .expect("coordinator starts");
        let incoming = IncomingCall::new(
            CallId::new(),
            "sip:a@example.test".into(),
            "sip:b@example.test".into(),
            None,
            coordinator.clone(),
        );

        let contacts: Vec<String> = Vec::new();
        let err = incoming
            .redirect_with_contacts(302, contacts)
            .await
            .unwrap_err();
        assert!(matches!(err, SessionError::InvalidInput(_)));
        coordinator.shutdown();
    }
}
