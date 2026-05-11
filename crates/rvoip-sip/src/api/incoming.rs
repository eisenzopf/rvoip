//! Incoming call handling — accept, reject, redirect, or defer.

#![deny(missing_docs)]

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use crate::api::events::Event;
use crate::api::handle::{CallId, SessionHandle};
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
    pub async fn accept(mut self) -> Result<SessionHandle> {
        self.resolved = true;
        self.coordinator.accept_call(&self.call_id).await?;
        Ok(SessionHandle::new(
            self.call_id.clone(),
            self.coordinator.clone(),
        ))
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
    pub async fn accept_with_sdp(mut self, sdp: String) -> Result<SessionHandle> {
        self.resolved = true;
        self.coordinator
            .accept_call_with_sdp(&self.call_id, sdp)
            .await?;
        Ok(SessionHandle::new(
            self.call_id.clone(),
            self.coordinator.clone(),
        ))
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
            if let Err(e) = coordinator.reject_call(&call_id, status, &reason).await {
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
    /// session-core primitive; higher-level B2BUA/routing layers decide
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
            .redirect_call(&self.call_id, status, contacts)
            .await
    }

    /// Redirect the caller to another URI with **302 Moved Temporarily**.
    ///
    /// This legacy fire-and-forget method is kept for compatibility. Prefer
    /// [`redirect_to`](Self::redirect_to) when the caller needs a result.
    #[deprecated(note = "Use redirect_to(...).await instead")]
    pub fn redirect(self, target: &str) {
        let coordinator = self.coordinator.clone();
        let call_id = self.call_id.clone();
        let target = target.to_string();
        let mut this = self;
        this.resolved = true;
        tokio::spawn(async move {
            if let Err(e) = coordinator.redirect_call(&call_id, 302, vec![target]).await {
                tracing::warn!("[IncomingCall] redirect failed for {}: {}", call_id, e);
            }
        });
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
        IncomingCallGuard::new(self.call_id.clone(), self.coordinator.clone(), timeout)
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
                .reject_call(&call_id, 500, "Server Internal Error")
                .await
            {
                tracing::error!(
                    "[IncomingCall] panic-path reject_call failed for {}: {}",
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
                    .reject_call(&call_id_clone, 503, "Service Unavailable")
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

    /// The call identifier for this deferred call (use as queue key).
    ///
    /// This accessor is trivial and can be used to index a queue or map.
    pub fn call_id(&self) -> &CallId {
        &self.call_id
    }

    /// Mark this deferred call as resolved because session-core already
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
        let coordinator = self.coordinator.clone();
        let call_id = self.call_id.clone();
        let reason = reason.to_string();
        tokio::spawn(async move {
            let _ = coordinator.reject_call(&call_id, status, &reason).await;
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
    /// know when the rejection has been observed by session-core's event
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
        if self.resolved.swap(true, Ordering::SeqCst) {
            return Err(SessionError::InvalidTransition(format!(
                "IncomingCallGuard for {} is already resolved",
                self.call_id
            )));
        }

        let mut events = self.coordinator.events_for_session(&self.call_id).await?;
        self.coordinator
            .reject_call(&self.call_id, status, reason)
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
                    .reject_call(&call_id, 503, "Service Unavailable")
                    .await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::SessionApiCrossCrateEvent;
    use crate::api::unified::Config;

    async fn publish_synthetic(event: Event) {
        let wrapped = SessionApiCrossCrateEvent::new(event);
        let coord = rvoip_infra_common::events::global_coordinator()
            .await
            .clone();
        coord
            .publish(wrapped)
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

        publish_synthetic(Event::CallCancelled { call_id }).await;

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

        publish_synthetic(Event::CallAnswered { call_id, sdp: None }).await;

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
