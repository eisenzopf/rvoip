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
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use rvoip_sip_core::types::headers::{HeaderName, TypedHeader};
use rvoip_sip_core::{Request, Response};

use crate::api::events::Event;
use crate::api::handle::{CallId, SessionHandle};
use crate::api::headers::view::{
    header_names_slice, header_slice, headers_named_slice, SipHeaderView,
};
use crate::api::lifecycle::{CallLifecycleSnapshot, CallTerminalInfo};
use crate::api::unified::UnifiedCoordinator;
use crate::errors::{Result, SessionError};
use crate::types::CallState;

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
            request: None,
            coordinator,
            resolved: false,
        }
    }

    /// Construct an `IncomingCall` with a parsed inbound INVITE
    /// retained. Populates [`Self::headers`] from the request for
    /// back-compat with the legacy field.
    pub(crate) fn with_request(
        call_id: CallId,
        from: String,
        to: String,
        sdp: Option<String>,
        coordinator: Arc<UnifiedCoordinator>,
        request: Arc<Request>,
    ) -> Self {
        let mut headers: HashMap<String, String> = HashMap::new();
        for hdr in &request.headers {
            let name = hdr.name();
            let key = match &name {
                HeaderName::Other(s) => s.to_ascii_lowercase(),
                other => format!("{:?}", other).to_ascii_lowercase(),
            };
            // Keep first-seen wire value for the legacy HashMap.
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
            coordinator,
            resolved: false,
        }
    }

    /// Underlying parsed [`Request`]. `None` when the call was
    /// synthesized (tests) or under a future lean-mode feature flag.
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
        self.coordinator.send_early_media(&self.call_id, sdp).await
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
        self.coordinator
            .send_early_media(&self.call_id, sdp)
            .await?;
        self.coordinator
            .set_audio_source(&self.call_id, source)
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
        let coordinator = self.coordinator.clone();
        let call_id = self.call_id.clone();
        let reason = reason.to_string();
        tokio::spawn(async move {
            if let Err(e) = coordinator
                .reject(&call_id)
                .with_status(status)
                .with_reason(reason)
                .send()
                .await
            {
                tracing::warn!("[IncomingCall] reject failed for {}: {}", call_id, e);
            }
        });
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
        self.coordinator
            .redirect(&self.call_id)
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
            return IncomingCallGuard::resolved(self.call_id.clone(), self.coordinator.clone());
        }
        IncomingCallGuard::new(self.call_id.clone(), self.coordinator.clone(), timeout)
    }

    // ─────────────────────────────────────────────────────────────────
    // SIP_API_DESIGN_2 Phase D — builder entry points on inbound INVITE.
    // ─────────────────────────────────────────────────────────────────

    /// Begin an `AcceptBuilder` so additional response headers can be
    /// staged before sending 200 OK.
    pub fn accept_builder(mut self) -> crate::api::respond::AcceptBuilder {
        self.resolved = true;
        crate::api::respond::AcceptBuilder::new(self.coordinator.clone(), self.call_id.clone())
    }

    /// Begin a `RejectBuilder` for a custom 4xx/5xx/6xx response.
    pub fn reject_builder(mut self) -> crate::api::respond::RejectBuilder {
        self.resolved = true;
        crate::api::respond::RejectBuilder::new(self.coordinator.clone(), self.call_id.clone())
    }

    /// Begin a `RedirectBuilder` for a custom 3xx response.
    pub fn redirect_builder(mut self) -> crate::api::respond::RedirectBuilder {
        self.resolved = true;
        crate::api::respond::RedirectBuilder::new(self.coordinator.clone(), self.call_id.clone())
    }

    /// Begin a `ProvisionalBuilder` for a 1xx reliable provisional.
    pub fn send_provisional_builder(&self, code: u16) -> crate::api::respond::ProvisionalBuilder {
        crate::api::respond::ProvisionalBuilder::new(
            self.coordinator.clone(),
            self.call_id.clone(),
            code,
        )
    }

    /// Begin an `AuthChallengeBuilder` for a 401/407 response.
    pub fn challenge_builder(
        &self,
        scheme: crate::api::respond::AuthScheme,
    ) -> crate::api::respond::AuthChallengeBuilder {
        crate::api::respond::AuthChallengeBuilder::new(
            self.coordinator.clone(),
            self.call_id.clone(),
            rvoip_sip_core::types::Method::Invite,
            scheme,
        )
    }

    /// Begin a `GenericResponseBuilder` for an arbitrary 3xx-6xx
    /// status (e.g. 491 Request Pending with `Retry-After`).
    pub fn respond_builder(
        mut self,
        status: u16,
    ) -> Result<crate::api::respond::GenericResponseBuilder> {
        self.resolved = true;
        crate::api::respond::GenericResponseBuilder::new(
            self.coordinator.clone(),
            self.call_id.clone(),
            rvoip_sip_core::types::Method::Invite,
            status,
        )
    }
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
        tokio::spawn(async move {
            if let Err(e) = coordinator
                .reject(&call_id)
                .with_status(500)
                .with_reason("Server Internal Error")
                .send()
                .await
            {
                tracing::error!(
                    "[IncomingCall] panic-path reject failed for {}: {}",
                    call_id,
                    e
                );
            }
        });
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
    deadline: Instant,
    resolved: Arc<AtomicBool>,
}

impl IncomingCallGuard {
    fn new(call_id: CallId, coordinator: Arc<UnifiedCoordinator>, timeout: Duration) -> Self {
        let deadline = Instant::now() + timeout;
        let resolved = Arc::new(AtomicBool::new(false));

        // Spawn a watchdog that auto-rejects if the deadline passes
        let coordinator_clone = coordinator.clone();
        let call_id_clone = call_id.clone();
        let watchdog_resolved = resolved.clone();
        tokio::spawn(async move {
            let remaining = deadline.saturating_duration_since(Instant::now());
            tokio::time::sleep(remaining).await;
            if !watchdog_resolved.swap(true, Ordering::SeqCst) {
                // The coordinator will silently ignore this if the session is already gone
                let _ = coordinator_clone
                    .reject(&call_id_clone)
                    .with_status(503)
                    .with_reason("Service Unavailable")
                    .send()
                    .await;
            }
        });

        Self {
            call_id,
            coordinator,
            deadline,
            resolved,
        }
    }

    fn resolved(call_id: CallId, coordinator: Arc<UnifiedCoordinator>) -> Self {
        Self {
            call_id,
            coordinator,
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
        if self.coordinator.fast_auto_accept_incoming_calls() {
            self.resolved.store(true, Ordering::SeqCst);
            return Ok(SessionHandle::new(
                self.call_id.clone(),
                self.coordinator.clone(),
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
        self.coordinator.accept_call(&self.call_id).await?;
        Ok(SessionHandle::new(
            self.call_id.clone(),
            self.coordinator.clone(),
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
        let coordinator = self.coordinator.clone();
        let call_id = self.call_id.clone();
        let reason = reason.to_string();
        tokio::spawn(async move {
            let _ = coordinator
                .reject(&call_id)
                .with_status(status)
                .with_reason(reason)
                .send()
                .await;
        });
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
        self.coordinator
            .reject(&self.call_id)
            .with_status(status)
            .with_reason(reason.to_string())
            .send()
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
                        ))
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
                        ))
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
            ))))
        }
        Some(CallTerminalInfo::Ended { reason }) => {
            return Some(Err(SessionError::Other(format!(
                "incoming call ended before cancellation: {}",
                reason
            ))))
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
            let coordinator = self.coordinator.clone();
            let call_id = self.call_id.clone();
            tokio::spawn(async move {
                let _ = coordinator
                    .reject(&call_id)
                    .with_status(503)
                    .with_reason("Service Unavailable")
                    .send()
                    .await;
            });
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// IncomingRequest / IncomingResponse / IncomingRegister
//
// These three wrappers complete the inbound surface. They are populated
// when dialog-core publishes an in-dialog request, an inbound response,
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
    /// Coordinator for sending responses. `None` for bus-constructed
    /// instances; the surface consumer (CallbackPeer / StreamPeer)
    /// repopulates this on dispatch so response builders can resolve
    /// the dialog/transaction.
    pub(crate) coordinator: Option<Arc<UnifiedCoordinator>>,
}

impl std::fmt::Debug for IncomingRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IncomingRequest")
            .field("call_id", &self.call_id)
            .field("from", &self.from)
            .field("to", &self.to)
            .field("method", &self.method)
            .field("has_request", &self.request.is_some())
            .field("has_coordinator", &self.coordinator.is_some())
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
    /// Returns `Err(SessionError::InvalidInput)` when the bus path has
    /// not yet rehydrated the coordinator hook (only possible while an
    /// `IncomingRequest` is in flight between the event bus and the
    /// surface consumer).
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
        Ok(crate::api::handle::SessionHandle::new(
            self.call_id.clone(),
            coord,
        ))
    }

    /// Begin a `GenericResponseBuilder` for the inbound request.
    /// Returns `Err(SessionError::InvalidInput)` when this
    /// `IncomingRequest` was constructed without a coordinator hook
    /// (bus path before the surface consumer rehydrates it).
    pub fn respond_builder(
        &self,
        status: u16,
    ) -> Result<crate::api::respond::GenericResponseBuilder> {
        let coord = self.coordinator.clone().ok_or_else(|| {
            SessionError::InvalidInput(
                "IncomingRequest.respond_builder() requires a coordinator hook; the bus \
                 path has not yet rehydrated it"
                    .to_string(),
            )
        })?;
        crate::api::respond::GenericResponseBuilder::new(
            coord,
            self.call_id.clone(),
            self.method.clone(),
            status,
        )
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
        Ok(crate::api::respond::AuthChallengeBuilder::new(
            coord,
            self.call_id.clone(),
            self.method.clone(),
            scheme,
        ))
    }

    /// Rehydrate the coordinator hook so response builders can
    /// dispatch. Called by the surface consumer (CallbackPeer /
    /// StreamPeer) on every inbound event before exposing the
    /// `IncomingRequest` to application code.
    pub(crate) fn set_coordinator(&mut self, coord: Arc<UnifiedCoordinator>) {
        self.coordinator = Some(coord);
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
            coordinator: None,
        }
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
#[derive(Clone, Debug)]
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
                TypedHeader::Require(req) => {
                    if req
                        .option_tags
                        .iter()
                        .any(|s| s.eq_ignore_ascii_case("100rel"))
                    {
                        has_require_100rel = true;
                    }
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
    /// SIP_API_DESIGN_2 Phase D — optional hook back into the
    /// coordinator so `RegisterResponseBuilder.send()` can publish a
    /// `SessionToDialogEvent::SendRegisterResponse` to dialog-core.
    /// `None` when the wrapper was synthesized for tests or by the
    /// legacy registrar crate that authors responses directly.
    pub(crate) coordinator: Option<Arc<UnifiedCoordinator>>,
}

impl std::fmt::Debug for IncomingRegister {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IncomingRegister")
            .field("transaction_id", &self.transaction_id)
            .field("from_uri", &self.from_uri)
            .field("to_uri", &self.to_uri)
            .field("contact_uri", &self.contact_uri)
            .field("expires", &self.expires)
            .field("authorization_present", &self.authorization.is_some())
            .field("call_id_header", &self.call_id_header)
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
    }

    /// Begin a 401 / 407 auth challenge for the inbound REGISTER.
    /// The matching dispatch happens via the same
    /// `SessionToDialogEvent::SendRegisterResponse` channel that
    /// `accept_builder` uses, so a registrar can author both 200 OK
    /// and 401 through one consistent surface.
    pub fn challenge_builder(
        &self,
        scheme: crate::api::respond::AuthScheme,
    ) -> crate::api::respond::RegisterResponseBuilder {
        crate::api::respond::RegisterResponseBuilder::new_challenge(
            self.transaction_id.clone(),
            self.coordinator.clone(),
            scheme,
        )
    }

    /// Begin a generic non-2xx REGISTER response (404, 423 Interval
    /// Too Brief with `Min-Expires`, 503 Service Unavailable, …).
    pub fn reject_builder(&self, status: u16) -> crate::api::respond::RegisterResponseBuilder {
        crate::api::respond::RegisterResponseBuilder::new_reject(
            self.transaction_id.clone(),
            self.coordinator.clone(),
            status,
        )
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
            coordinator: None,
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
            coordinator: None,
        }
    }

    /// Same as [`with_request`] but threads the coordinator handle so
    /// the response builder can publish a
    /// `SessionToDialogEvent::SendRegisterResponse` back to
    /// dialog-core.
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
            coordinator: Some(coordinator),
        }
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

    /// SIP_API_DESIGN_2 Phase D — set the coordinator hook so
    /// `accept_builder()` / `challenge_builder()` / `reject_builder()`
    /// can dispatch a `SessionToDialogEvent::SendRegisterResponse`.
    /// Called by the dispatch path right before handing the
    /// `IncomingRegister` to the application handler.
    pub fn set_coordinator(&mut self, coordinator: Arc<UnifiedCoordinator>) {
        self.coordinator = Some(coordinator);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::unified::Config;

    async fn publish_synthetic(coordinator: &UnifiedCoordinator, event: Event) {
        coordinator
            .publish_app_event_for_test(event)
            .await
            .expect("publish synthetic event");
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
        assert!(err.to_string().contains("answered before cancellation"));
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
