//! Sequential peer API for clients, scripts, softphones, and tests.
//!
//! [`StreamPeer`] wraps a [`UnifiedCoordinator`]
//! with two ergonomic pieces:
//!
//! - [`PeerControl`] for commands such as `call`, `accept`, registration, and
//!   early media.
//! - [`EventReceiver`] for typed application events.
//!
//! The unsplit [`StreamPeer`] exposes common `wait_for_*` helpers that consume
//! events until the requested state is observed. This is the simplest API for
//! application tests and single-peer clients. Split the peer when one task
//! should receive events while other tasks issue commands.
//!
//! Registration uses the same lifecycle as the lower-level coordinator:
//! successful REGISTER responses store accepted expiry and refresh timing, and
//! `StreamPeer::shutdown` performs best-effort graceful unregister by default.
//!
//! For reactive server code (proxies, IVR engines) use [`CallbackPeer`] instead.
//!
//! [`CallbackPeer`]: crate::api::callback_peer::CallbackPeer

#![deny(missing_docs)]

use std::net::IpAddr;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::adapters::SessionApiCrossCrateEvent;
use crate::api::events::{Event, MediaSecurityState};
use crate::api::handle::{CallId, SessionHandle};
use crate::api::incoming::IncomingCall;
use crate::api::unified::{Config, UnifiedCoordinator};
use crate::errors::{Result, SessionError};

// Re-export Config so callers can import it from this module
pub use crate::api::unified::Config as PeerConfig;

// ===== EventReceiver =====

/// A receiver for session API events.
///
/// Obtained via [`StreamPeer::next_event()`], [`SessionHandle::events()`],
/// [`UnifiedCoordinator::events`](crate::UnifiedCoordinator::events), or
/// [`PeerControl::subscribe_events()`]. Each `EventReceiver` is independent,
/// so slow consumers do not affect others.
///
/// Events flow through the [`GlobalEventCoordinator`]'s `"session_to_app"` channel,
/// which uses a lock-free broadcast internally.
///
/// Filter helpers such as [`next_transfer`](Self::next_transfer) consume and
/// discard non-matching events. Create a separate receiver when another task
/// also needs to observe those events.
///
/// # Consuming events
///
/// ```rust,no_run
/// # async fn example(mut events: rvoip_session_core::EventReceiver) {
/// while let Some(event) = events.next().await {
///     if event.is_transfer_event() {
///         println!("transfer event: {:?}", event);
///     }
/// }
/// # }
/// ```
///
/// [`GlobalEventCoordinator`]: rvoip_infra_common::events::coordinator::GlobalEventCoordinator
pub struct EventReceiver {
    rx: mpsc::Receiver<Arc<dyn rvoip_infra_common::events::cross_crate::CrossCrateEvent>>,
    filter: Option<CallId>,
}

impl EventReceiver {
    pub(crate) fn new(
        rx: mpsc::Receiver<Arc<dyn rvoip_infra_common::events::cross_crate::CrossCrateEvent>>,
    ) -> Self {
        Self { rx, filter: None }
    }

    /// Create a receiver pre-filtered to a specific session.
    pub(crate) fn filtered(
        rx: mpsc::Receiver<Arc<dyn rvoip_infra_common::events::cross_crate::CrossCrateEvent>>,
        call_id: CallId,
    ) -> Self {
        Self {
            rx,
            filter: Some(call_id),
        }
    }

    /// Wait for the next event (optionally filtered to one session).
    ///
    /// Returns `None` when the coordinator shuts down.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(mut events: rvoip_session_core::EventReceiver) {
    /// while let Some(event) = events.next().await {
    ///     println!("session event: {event:?}");
    /// }
    /// # }
    /// ```
    pub async fn next(&mut self) -> Option<Event> {
        loop {
            let raw = self.rx.recv().await?;
            // Downcast from Arc<dyn CrossCrateEvent> to our concrete wrapper
            let session_event = raw.as_any().downcast_ref::<SessionApiCrossCrateEvent>()?;
            let event = session_event.event.clone();
            // Apply per-session filter if set
            if let Some(ref filter) = self.filter {
                if event.call_id() != Some(filter) {
                    continue;
                }
            }
            return Some(event);
        }
    }

    /// Non-blocking: return the next event if one is immediately available.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # fn example(mut events: rvoip_session_core::EventReceiver) {
    /// if let Some(event) = events.try_next() {
    ///     println!("ready event: {event:?}");
    /// }
    /// # }
    /// ```
    pub fn try_next(&mut self) -> Option<Event> {
        loop {
            let raw = self.rx.try_recv().ok()?;
            let session_event = raw.as_any().downcast_ref::<SessionApiCrossCrateEvent>()?;
            let event = session_event.event.clone();
            if let Some(ref filter) = self.filter {
                if event.call_id() != Some(filter) {
                    continue;
                }
            }
            return Some(event);
        }
    }

    // ===== Filtered-wait helpers =====
    //
    // Each method loops over `self.next()` and returns only matching events.
    // Non-matching events are consumed and discarded — the same behaviour as
    // the existing `wait_for_*` methods on `StreamPeer`.

    /// Wait for the next incoming call event, skipping all others.
    ///
    /// Returns `(call_id, from, to, sdp)` or `None` on channel close.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(mut events: rvoip_session_core::EventReceiver) {
    /// if let Some((call_id, from, to, sdp)) = events.next_incoming().await {
    ///     println!("incoming {call_id} from {from} to {to}; sdp={}", sdp.is_some());
    /// }
    /// # }
    /// ```
    pub async fn next_incoming(&mut self) -> Option<(CallId, String, String, Option<String>)> {
        loop {
            match self.next().await? {
                Event::IncomingCall {
                    call_id,
                    from,
                    to,
                    sdp,
                } => {
                    return Some((call_id, from, to, sdp));
                }
                _ => continue,
            }
        }
    }

    /// Wait for the next DTMF digit on any call, skipping all others.
    ///
    /// Returns `(call_id, digit)` or `None` on channel close.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(mut events: rvoip_session_core::EventReceiver) {
    /// if let Some((call_id, digit)) = events.next_dtmf().await {
    ///     println!("{call_id} sent DTMF {digit}");
    /// }
    /// # }
    /// ```
    pub async fn next_dtmf(&mut self) -> Option<(CallId, char)> {
        loop {
            match self.next().await? {
                Event::DtmfReceived { call_id, digit } => {
                    return Some((call_id, digit));
                }
                _ => continue,
            }
        }
    }

    /// Wait for the next provisional call-progress event on any call.
    ///
    /// Returns `(call_id, status_code, reason, sdp)` or `None` on channel close.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(mut events: rvoip_session_core::EventReceiver) {
    /// if let Some((call_id, status, reason, _sdp)) = events.next_progress().await {
    ///     println!("{call_id} received provisional {status} {reason}");
    /// }
    /// # }
    /// ```
    pub async fn next_progress(&mut self) -> Option<(CallId, u16, String, Option<String>)> {
        loop {
            match self.next().await? {
                Event::CallProgress {
                    call_id,
                    status_code,
                    reason,
                    sdp,
                } => return Some((call_id, status_code, reason, sdp)),
                _ => continue,
            }
        }
    }

    /// Wait for the next typed media-security negotiation event on any call.
    ///
    /// Returns `(call_id, state)` or `None` on channel close. The returned
    /// state does not expose SRTP key material.
    pub async fn next_media_security_negotiated(&mut self) -> Option<(CallId, MediaSecurityState)> {
        loop {
            let event = self.next().await?;
            if let Some(state) = media_security_state_from_event(event) {
                return Some(state);
            }
        }
    }

    /// Wait for the next transfer-related event, skipping all others.
    ///
    /// Matches `ReferReceived`, `TransferAccepted`, `ReferCompleted`,
    /// `TransferFailed`, `ReferProgress`, `ReferNotify`, and replacement
    /// lifecycle transfer events.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(mut events: rvoip_session_core::EventReceiver) {
    /// if let Some(event) = events.next_transfer().await {
    ///     println!("transfer event: {event:?}");
    /// }
    /// # }
    /// ```
    pub async fn next_transfer(&mut self) -> Option<Event> {
        loop {
            let event = self.next().await?;
            if event.is_transfer_event() {
                return Some(event);
            }
        }
    }

    /// Wait for the next event matching `predicate`, discarding non-matches.
    ///
    /// Returns `None` on channel close.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(mut events: rvoip_session_core::EventReceiver) {
    /// let ended = events
    ///     .next_where(|event| matches!(event, rvoip_session_core::Event::CallEnded { .. }))
    ///     .await;
    /// # let _ = ended;
    /// # }
    /// ```
    pub async fn next_where<F: FnMut(&Event) -> bool>(
        &mut self,
        mut predicate: F,
    ) -> Option<Event> {
        loop {
            let event = self.next().await?;
            if predicate(&event) {
                return Some(event);
            }
        }
    }

    /// Wait for the next event belonging to `call_id`, skipping others.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(mut events: rvoip_session_core::EventReceiver, call_id: rvoip_session_core::CallId) {
    /// if let Some(event) = events.next_for_call(&call_id).await {
    ///     println!("event for {call_id}: {event:?}");
    /// }
    /// # }
    /// ```
    pub async fn next_for_call(&mut self, call_id: &CallId) -> Option<Event> {
        loop {
            let event = self.next().await?;
            if event.call_id() == Some(call_id) {
                return Some(event);
            }
        }
    }
}

// ===== PeerControl =====

/// The command half of a [`StreamPeer`].
///
/// Cheap to clone — share across tasks or pass to spawned workers without
/// moving the whole peer.
#[derive(Clone)]
pub struct PeerControl {
    pub(crate) coordinator: Arc<UnifiedCoordinator>,
    pub(crate) local_uri: String,
}

impl PeerControl {
    /// Initiate an outgoing call. Returns a [`SessionHandle`] immediately; the
    /// call enters `Ringing` state until the remote answers.
    ///
    /// If the peer was configured with [`Config.credentials`] (or via
    /// [`StreamPeerBuilder::with_credentials`]), those credentials are
    /// attached to the session and used to transparently retry on a 401/407
    /// INVITE challenge (RFC 3261 §22.2).
    ///
    /// Use [`subscribe_events()`] to watch for [`Event::CallAnswered`].
    ///
    /// [`subscribe_events()`]: Self::subscribe_events
    /// [`Config.credentials`]: crate::api::unified::Config::credentials
    /// [`StreamPeerBuilder::with_credentials`]: StreamPeerBuilder::with_credentials
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(control: rvoip_session_core::PeerControl) -> rvoip_session_core::Result<()> {
    /// let call = control.call("sip:bob@example.com").await?;
    /// let mut events = control.subscribe_events().await?;
    /// // Wait for Event::CallAnswered for `call.id()` before using media.
    /// # let _ = (call, events.next().await);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn call(&self, target: &str) -> Result<SessionHandle> {
        let id = self.coordinator.make_call(&self.local_uri, target).await?;
        Ok(SessionHandle::new(id, self.coordinator.clone()))
    }

    /// Initiate an outgoing call with explicit digest-auth credentials,
    /// overriding any per-peer default. Useful for multi-tenant clients
    /// where each call authenticates as a different user.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(control: rvoip_session_core::PeerControl) -> rvoip_session_core::Result<()> {
    /// let call = control.call_with_auth(
    ///     "sip:bob@example.com",
    ///     rvoip_session_core::types::Credentials::new("alice", "secret"),
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

    /// Accept an incoming call that was presented as an event.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(control: rvoip_session_core::PeerControl, call_id: rvoip_session_core::CallId) -> rvoip_session_core::Result<()> {
    /// let handle = control.accept(&call_id).await?;
    /// # let _ = handle;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn accept(&self, call_id: &CallId) -> Result<SessionHandle> {
        self.coordinator.accept_call(call_id).await?;
        Ok(SessionHandle::new(
            call_id.clone(),
            self.coordinator.clone(),
        ))
    }

    /// Reject an incoming call with the given SIP status code and reason phrase.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(control: rvoip_session_core::PeerControl, call_id: rvoip_session_core::CallId) -> rvoip_session_core::Result<()> {
    /// control.reject(&call_id, 486, "Busy Here").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn reject(&self, call_id: &CallId, status: u16, reason: &str) -> Result<()> {
        self.coordinator.reject_call(call_id, status, reason).await
    }

    /// Send a reliable 183 Session Progress with early-media SDP (RFC 3262).
    ///
    /// Call before [`accept()`] on an incoming call to stream ringback,
    /// announcements, or progress audio to the caller before answering. The
    /// call enters [`CallState::EarlyMedia`](crate::CallState::EarlyMedia); a subsequent `accept()`
    /// transitions to `Active` while reusing the negotiated SDP.
    ///
    /// If `sdp` is `None`, the SDP answer is generated from the INVITE's
    /// offer. Callers wanting custom early-media streams can pass explicit
    /// SDP via `Some(body)`.
    ///
    /// Fails with [`SessionError::UnreliableProvisionalsNotSupported`] if
    /// the remote peer did not advertise `Supported: 100rel` on the INVITE.
    ///
    /// [`accept()`]: Self::accept
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(control: rvoip_session_core::PeerControl, call_id: rvoip_session_core::CallId) -> rvoip_session_core::Result<()> {
    /// control.send_early_media(&call_id, None).await?;
    /// let handle = control.accept(&call_id).await?;
    /// # let _ = handle;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_early_media(&self, call_id: &CallId, sdp: Option<String>) -> Result<()> {
        self.coordinator.send_early_media(call_id, sdp).await
    }

    /// Subscribe to all events from this coordinator.
    ///
    /// Each call returns an independent receiver (broadcast semantics).
    /// Registration lifecycle events are visible here because they do not
    /// belong to a specific call session.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(control: rvoip_session_core::PeerControl) -> rvoip_session_core::Result<()> {
    /// let mut events = control.subscribe_events().await?;
    /// tokio::spawn(async move {
    ///     while let Some(event) = events.next().await {
    ///         println!("{event:?}");
    ///     }
    /// });
    /// # Ok(())
    /// # }
    /// ```
    pub async fn subscribe_events(&self) -> Result<EventReceiver> {
        let rx = self.coordinator.subscribe_events().await?;
        Ok(EventReceiver::new(rx))
    }

    /// Subscribe to RFC 4235 dialog-package state for a target URI.
    pub async fn subscribe_dialogs(
        &self,
        target_uri: &str,
        expires: u32,
    ) -> Result<crate::api::dialog_subscription::DialogSubscriptionHandle> {
        self.coordinator
            .subscribe_dialogs(target_uri, &self.local_uri, &self.local_uri, expires)
            .await
    }

    /// Access the underlying [`UnifiedCoordinator`] for advanced use.
    ///
    /// This accessor is intentionally trivial and does not clone the
    /// coordinator.
    pub fn coordinator(&self) -> &Arc<UnifiedCoordinator> {
        &self.coordinator
    }
}

// ===== StreamPeer =====

/// A sequential SIP peer with event-stream-style access.
///
/// Use this when your application wants to drive SIP as an async workflow:
/// make a call, wait for it to answer, send DTMF, wait for hangup; register to
/// a PBX; or wait for an incoming call and resolve it inline.
///
/// `StreamPeer` owns one event receiver. Methods such as
/// [`wait_for_incoming`](Self::wait_for_incoming) and
/// [`wait_for_answered`](Self::wait_for_answered) consume and discard unrelated
/// events while they wait. If multiple tasks need to observe events, use
/// [`split`](Self::split) or [`PeerControl::subscribe_events`] to create
/// independent receivers. Use the underlying coordinator through
/// [`control`](Self::control) for detailed registration metadata or
/// deterministic unregister helpers.
///
/// # Quick start
///
/// ```rust,no_run
/// # async fn example() -> anyhow::Result<()> {
/// use rvoip_session_core::StreamPeer;
///
/// // UAC: make a call
/// let mut uac = StreamPeer::new("alice").await?;
/// let handle = uac.call("sip:bob@192.168.1.100:5060").await?;
/// let handle = handle.wait_for_answered(Some(std::time::Duration::from_secs(30))).await?;
/// handle.hangup_and_wait(Some(std::time::Duration::from_secs(5))).await?;
///
/// // UAS: answer a call
/// let mut uas = StreamPeer::new("bob").await?;
/// let incoming = uas.wait_for_incoming().await?;
/// let handle = incoming.accept().await?;
/// handle.wait_for_end(None).await?;
/// # Ok(())
/// # }
/// ```
///
/// # Splitting
///
/// For concurrent operation, [`split()`] the peer into a [`PeerControl`] (clonable,
/// for sending commands) and an [`EventReceiver`] (for receiving events in a
/// dedicated task).
///
/// [`split()`]: StreamPeer::split
pub struct StreamPeer {
    control: PeerControl,
    events: EventReceiver,
}

impl StreamPeer {
    /// Create a peer with an auto-generated SIP URI based on `name`.
    ///
    /// This uses [`Config::default`] as the base and sets
    /// `local_uri = "sip:<name>@<local_ip>:<sip_port>"`. For production or
    /// interop tests, prefer [`with_config`](Self::with_config) or
    /// [`builder`](Self::builder) so bind addresses, media ports, TLS, SRTP,
    /// registration credentials, and advertised addresses are explicit.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_session_core::Result<()> {
    /// let peer = rvoip_session_core::StreamPeer::new("alice").await?;
    /// peer.shutdown().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(name: &str) -> Result<Self> {
        let mut config = Config::default();
        config.local_uri = format!("sip:{}@{}:{}", name, config.local_ip, config.sip_port);
        Self::with_config(config).await
    }

    /// Create a peer with explicit configuration.
    ///
    /// The coordinator starts immediately. Subscribe or split the peer before
    /// triggering work if your code must not miss early events.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_session_core::Result<()> {
    /// use rvoip_session_core::{Config, StreamPeer};
    ///
    /// let peer = StreamPeer::with_config(Config::local("alice", 5060)).await?;
    /// peer.shutdown().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn with_config(config: Config) -> Result<Self> {
        let local_uri = config.local_uri.clone();
        let coordinator = UnifiedCoordinator::new(config).await?;
        let event_rx = coordinator.subscribe_events().await?;
        Ok(Self {
            control: PeerControl {
                coordinator,
                local_uri,
            },
            events: EventReceiver::new(event_rx),
        })
    }

    /// Split the peer into independent control and event halves.
    ///
    /// Useful when you want to drive the event loop in one task while issuing
    /// commands from another.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(peer: rvoip_session_core::StreamPeer) {
    /// let (control, mut events) = peer.split();
    /// tokio::spawn(async move {
    ///     while let Some(event) = events.next().await {
    ///         println!("{event:?}");
    ///     }
    /// });
    /// # let _ = control;
    /// # }
    /// ```
    pub fn split(self) -> (PeerControl, EventReceiver) {
        (self.control, self.events)
    }

    /// Access the command half without consuming the peer.
    ///
    /// This accessor is trivial; use it when one task owns the sequential
    /// receiver but another helper needs to issue commands through a cloned
    /// [`PeerControl`].
    pub fn control(&self) -> &PeerControl {
        &self.control
    }

    // ===== Sequential helpers =====

    /// Initiate an outgoing call and return a [`SessionHandle`].
    ///
    /// The handle is returned as soon as the INVITE has been dispatched; wait
    /// for [`Event::CallAnswered`] with
    /// [`SessionHandle::wait_for_answered`] before assuming media is
    /// established. The stream-style [`wait_for_answered`](Self::wait_for_answered)
    /// helper remains useful when a single task owns and consumes the peer's
    /// event stream.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(mut peer: rvoip_session_core::StreamPeer) -> rvoip_session_core::Result<()> {
    /// let call = peer.call("sip:bob@example.com").await?;
    /// let answered = call.wait_for_answered(Some(std::time::Duration::from_secs(30))).await?;
    /// # let _ = answered;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn call(&mut self, target: &str) -> Result<SessionHandle> {
        self.control.call(target).await
    }

    /// Wait for the next incoming call.
    ///
    /// Blocks until an [`Event::IncomingCall`] is received. The returned
    /// [`IncomingCall`] must be resolved (accepted, rejected, or deferred).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(mut peer: rvoip_session_core::StreamPeer) -> rvoip_session_core::Result<()> {
    /// let incoming = peer.wait_for_incoming().await?;
    /// let call = incoming.accept().await?;
    /// # let _ = call;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn wait_for_incoming(&mut self) -> Result<IncomingCall> {
        loop {
            match self.events.next().await {
                Some(Event::IncomingCall {
                    call_id,
                    from,
                    to,
                    sdp,
                }) => {
                    return Ok(IncomingCall::new(
                        call_id,
                        from,
                        to,
                        sdp,
                        self.control.coordinator.clone(),
                    ));
                }
                None => return Err(SessionError::Other("Event channel closed".to_string())),
                _ => {}
            }
        }
    }

    /// Wait for a previously initiated call to be answered.
    ///
    /// Returns an error if the matching call fails before it is answered.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(mut peer: rvoip_session_core::StreamPeer, call_id: rvoip_session_core::CallId) -> rvoip_session_core::Result<()> {
    /// let handle = peer.wait_for_answered(&call_id).await?;
    /// # let _ = handle;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn wait_for_answered(&mut self, call_id: &CallId) -> Result<SessionHandle> {
        loop {
            match self.events.next().await {
                Some(Event::CallAnswered {
                    call_id: answered_id,
                    ..
                }) if &answered_id == call_id => {
                    return Ok(SessionHandle::new(
                        answered_id,
                        self.control.coordinator.clone(),
                    ));
                }
                Some(Event::CallFailed {
                    call_id: failed_id,
                    reason,
                    status_code,
                }) if &failed_id == call_id => {
                    return Err(SessionError::Other(format!(
                        "Call failed with {}: {}",
                        status_code, reason
                    )));
                }
                None => return Err(SessionError::Other("Event channel closed".to_string())),
                _ => {}
            }
        }
    }

    /// Wait for provisional progress on a specific outgoing call.
    ///
    /// The predicate is evaluated only for matching [`Event::CallProgress`]
    /// events. Non-matching events are consumed, matching the other sequential
    /// `StreamPeer` helpers.
    pub async fn wait_for_progress<F>(
        &mut self,
        call_id: &CallId,
        mut predicate: F,
    ) -> Result<Event>
    where
        F: FnMut(&Event) -> bool,
    {
        loop {
            match self.events.next().await {
                Some(event @ Event::CallProgress { .. }) => {
                    if event.call_id() == Some(call_id) && predicate(&event) {
                        return Ok(event);
                    }
                }
                Some(Event::CallAnswered {
                    call_id: answered_id,
                    ..
                }) if &answered_id == call_id => {
                    return Err(SessionError::Other(
                        "call answered before matching provisional progress".to_string(),
                    ));
                }
                Some(Event::CallFailed {
                    call_id: failed_id,
                    reason,
                    status_code,
                }) if &failed_id == call_id => {
                    return Err(SessionError::Other(format!(
                        "Call failed with {}: {}",
                        status_code, reason
                    )));
                }
                Some(Event::CallCancelled { call_id: id }) if &id == call_id => {
                    return Err(SessionError::Other(
                        "call cancelled before matching provisional progress".to_string(),
                    ));
                }
                None => return Err(SessionError::Other("Event channel closed".to_string())),
                _ => {}
            }
        }
    }

    /// Wait for typed media-security negotiation on a specific call.
    ///
    /// This is the stream-style equivalent of
    /// [`SessionHandle::wait_for_media_security`] when an application prefers
    /// to consume events from the peer's sequential event receiver.
    pub async fn wait_for_media_security(
        &mut self,
        call_id: &CallId,
    ) -> Result<MediaSecurityState> {
        loop {
            match self.events.next().await {
                Some(event) if event.call_id() == Some(call_id) => {
                    if let Some((_, state)) = media_security_state_from_event(event) {
                        return Ok(state);
                    }
                }
                None => return Err(SessionError::Other("Event channel closed".to_string())),
                _ => {}
            }
        }
    }

    /// Wait for a specific call to end (BYE received/sent).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(mut peer: rvoip_session_core::StreamPeer, call_id: rvoip_session_core::CallId) -> rvoip_session_core::Result<()> {
    /// let reason = peer.wait_for_ended(&call_id).await?;
    /// println!("call ended: {reason}");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn wait_for_ended(&mut self, call_id: &CallId) -> Result<String> {
        loop {
            match self.events.next().await {
                Some(Event::CallEnded {
                    call_id: ended_id,
                    reason,
                }) if &ended_id == call_id => {
                    return Ok(reason);
                }
                None => return Err(SessionError::Other("Event channel closed".to_string())),
                _ => {}
            }
        }
    }

    /// Read the next event without filtering.
    ///
    /// Returns `None` when the coordinator shuts down or the event channel
    /// closes.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(mut peer: rvoip_session_core::StreamPeer) {
    /// if let Some(event) = peer.next_event().await {
    ///     println!("{event:?}");
    /// }
    /// # }
    /// ```
    pub async fn next_event(&mut self) -> Option<Event> {
        self.events.next().await
    }

    /// Subscribe to RFC 4235 dialog-package state for a target URI.
    pub async fn subscribe_dialogs(
        &self,
        target_uri: &str,
        expires: u32,
    ) -> Result<crate::api::dialog_subscription::DialogSubscriptionHandle> {
        self.control.subscribe_dialogs(target_uri, expires).await
    }

    /// Register with a SIP server (6-arg form).
    ///
    /// Prefer [`register_with()`](Self::register_with) which uses a builder and
    /// derives `from_uri`/`contact_uri` from the peer's config. Successful
    /// registration stores registrar-accepted expiry and may schedule
    /// automatic refresh according to [`Config::registration_auto_refresh`].
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(mut peer: rvoip_session_core::StreamPeer) -> rvoip_session_core::Result<()> {
    /// let handle = peer.register(
    ///     "sip:registrar.example.com",
    ///     "sip:alice@example.com",
    ///     "sip:alice@192.168.1.50:5060",
    ///     "alice",
    ///     "secret",
    ///     3600,
    /// ).await?;
    /// # let _ = handle;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn register(
        &mut self,
        registrar_uri: &str,
        from_uri: &str,
        contact_uri: &str,
        username: &str,
        password: &str,
        expires: u32,
    ) -> Result<crate::api::unified::RegistrationHandle> {
        self.control
            .coordinator
            .register(
                registrar_uri,
                from_uri,
                contact_uri,
                username,
                password,
                expires,
            )
            .await
    }

    /// Register with a SIP server using a [`Registration`](crate::Registration) builder.
    ///
    /// The returned handle identifies the registration lifecycle. Use
    /// [`is_registered`](Self::is_registered) for a simple boolean or
    /// [`UnifiedCoordinator::registration_info`](crate::UnifiedCoordinator::registration_info)
    /// through [`control`](Self::control) for accepted expiry, refresh timing,
    /// Service-Route, GRUU, and failure metadata.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_session_core::Result<()> {
    /// use rvoip_session_core::{StreamPeer, Registration};
    ///
    /// let mut peer = StreamPeer::new("alice").await?;
    /// let handle = peer.register_with(
    ///     Registration::new("sip:registrar.example.com", "alice", "secret123")
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn register_with(
        &mut self,
        reg: crate::api::unified::Registration,
    ) -> Result<crate::api::unified::RegistrationHandle> {
        self.control.coordinator.register_with(reg).await
    }

    /// Query whether a registration handle is currently registered.
    ///
    /// Returns `true` once the registrar has replied 200 OK to the REGISTER
    /// (including after a 423 Interval Too Brief retry or 401 auth retry),
    /// and `false` if the registration was rejected, unregistered, or has
    /// not yet completed. This is intentionally coarse; use
    /// `control().coordinator().registration_info(handle)` for richer state.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(peer: rvoip_session_core::StreamPeer, handle: rvoip_session_core::RegistrationHandle) -> rvoip_session_core::Result<()> {
    /// let active = peer.is_registered(&handle).await?;
    /// # let _ = active;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn is_registered(
        &self,
        handle: &crate::api::unified::RegistrationHandle,
    ) -> Result<bool> {
        self.control.coordinator.is_registered(handle).await
    }

    /// Unregister (sends REGISTER with `Expires: 0`).
    ///
    /// Returns after the unregister request is accepted by the state machine.
    /// Use [`UnifiedCoordinator::unregister_and_wait`](crate::UnifiedCoordinator::unregister_and_wait)
    /// through [`control`](Self::control) when the caller needs to wait for
    /// the registrar response.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(peer: rvoip_session_core::StreamPeer, handle: rvoip_session_core::RegistrationHandle) -> rvoip_session_core::Result<()> {
    /// peer.unregister(&handle).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn unregister(&self, handle: &crate::api::unified::RegistrationHandle) -> Result<()> {
        self.control.coordinator.unregister(handle).await
    }

    /// Graceful shutdown: unregister active registrations, stop background
    /// tasks, and drop resources.
    ///
    /// This calls [`UnifiedCoordinator::shutdown_gracefully`] with the
    /// configured unregister timeout. Set
    /// [`Config::unregister_on_shutdown_timeout_secs`] to `0` to skip the
    /// best-effort unregister phase.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(peer: rvoip_session_core::StreamPeer) -> rvoip_session_core::Result<()> {
    /// peer.shutdown().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn shutdown(self) -> Result<()> {
        // Gracefully unregister active registrations, signal the coordinator
        // to stop background event loops, then drop self so remaining Arc
        // references decrease.
        self.control.coordinator.shutdown_gracefully(None).await?;
        drop(self);
        Ok(())
    }

    /// Return a cloneable handle that can signal shutdown from another task.
    ///
    /// Mirrors [`CallbackPeer::shutdown_handle`]. Useful when the peer is owned
    /// by an event loop and a supervisor task needs to stop it:
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn demo() -> rvoip_session_core::Result<()> {
    /// use rvoip_session_core::StreamPeer;
    /// let peer = StreamPeer::new("alice").await?;
    /// let stop = peer.shutdown_handle();
    /// tokio::spawn(async move {
    ///     // ... do some work ...
    ///     stop.shutdown();
    /// });
    /// // peer.next_event().await;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`CallbackPeer::shutdown_handle`]: crate::api::callback_peer::CallbackPeer::shutdown_handle
    pub fn shutdown_handle(&self) -> crate::api::callback_peer::ShutdownHandle {
        self.control.coordinator.shutdown_handle()
    }

    /// Start building a new `StreamPeer` with configuration options.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_session_core::Result<()> {
    /// let peer = rvoip_session_core::StreamPeer::builder()
    ///     .name("alice")
    ///     .sip_port(5080)
    ///     .build()
    ///     .await?;
    /// peer.shutdown().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn builder() -> StreamPeerBuilder {
        StreamPeerBuilder::new()
    }
}

fn media_security_state_from_event(event: Event) -> Option<(CallId, MediaSecurityState)> {
    match event {
        Event::MediaSecurityNegotiated {
            call_id,
            keying,
            suite,
            profile,
            contexts_installed,
        } => Some((
            call_id,
            MediaSecurityState {
                keying,
                suite,
                profile,
                contexts_installed,
            },
        )),
        _ => None,
    }
}

// ===== StreamPeerBuilder =====

/// Builder for [`StreamPeer`] with fluent configuration.
///
/// # Example
///
/// ```rust,no_run
/// # async fn example() -> anyhow::Result<()> {
/// use rvoip_session_core::StreamPeer;
///
/// let peer = StreamPeer::builder()
///     .name("alice")
///     .sip_port(5080)
///     .build()
///     .await?;
/// # Ok(())
/// # }
/// ```
pub struct StreamPeerBuilder {
    config: Config,
    name: Option<String>,
}

impl StreamPeerBuilder {
    /// Create a new builder with default configuration.
    ///
    /// # Examples
    ///
    /// ```
    /// let builder = rvoip_session_core::StreamPeerBuilder::new();
    /// # let _ = builder;
    /// ```
    pub fn new() -> Self {
        Self {
            config: Config::default(),
            name: None,
        }
    }

    /// Set the display name (auto-generates a SIP URI from it).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_session_core::Result<()> {
    /// let peer = rvoip_session_core::StreamPeer::builder()
    ///     .name("alice")
    ///     .build()
    ///     .await?;
    /// peer.shutdown().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn name(mut self, name: &str) -> Self {
        self.name = Some(name.to_string());
        self
    }

    /// Set the SIP port.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_session_core::Result<()> {
    /// let peer = rvoip_session_core::StreamPeer::builder()
    ///     .sip_port(5080)
    ///     .build()
    ///     .await?;
    /// peer.shutdown().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn sip_port(mut self, port: u16) -> Self {
        self.config.sip_port = port;
        self.config.bind_addr.set_port(port);
        self
    }

    /// Set the local IP address.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_session_core::Result<()> {
    /// let peer = rvoip_session_core::StreamPeer::builder()
    ///     .local_ip("127.0.0.1".parse().unwrap())
    ///     .build()
    ///     .await?;
    /// peer.shutdown().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn local_ip(mut self, ip: IpAddr) -> Self {
        self.config.local_ip = ip;
        self.config.bind_addr.set_ip(ip);
        self
    }

    /// Set the media port range.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_session_core::Result<()> {
    /// let peer = rvoip_session_core::StreamPeer::builder()
    ///     .media_ports(16000, 17000)
    ///     .build()
    ///     .await?;
    /// peer.shutdown().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn media_ports(mut self, start: u16, end: u16) -> Self {
        self.config.media_port_start = start;
        self.config.media_port_end = end;
        self
    }

    /// Use a fully custom config (overrides all previous settings).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_session_core::Result<()> {
    /// let config = rvoip_session_core::Config::local("alice", 5060);
    /// let peer = rvoip_session_core::StreamPeer::builder()
    ///     .config(config)
    ///     .build()
    ///     .await?;
    /// peer.shutdown().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn config(mut self, config: Config) -> Self {
        self.config = config;
        self
    }

    /// Set per-peer default SIP digest credentials used for RFC 3261 §22.2
    /// INVITE auth retry. When the server challenges an outgoing INVITE with
    /// 401/407, these credentials are applied automatically so the call can
    /// recover without intervention.
    ///
    /// Per-call override via [`PeerControl::call_with_auth`].
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_session_core::Result<()> {
    /// let peer = rvoip_session_core::StreamPeer::builder()
    ///     .name("alice")
    ///     .with_credentials("alice", "secret")
    ///     .build()
    ///     .await?;
    /// peer.shutdown().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_credentials(mut self, username: &str, password: &str) -> Self {
        self.config.credentials = Some(crate::types::Credentials::new(username, password));
        self
    }

    /// Build the [`StreamPeer`].
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_session_core::Result<()> {
    /// let peer = rvoip_session_core::StreamPeerBuilder::new()
    ///     .name("alice")
    ///     .build()
    ///     .await?;
    /// peer.shutdown().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn build(mut self) -> Result<StreamPeer> {
        if let Some(name) = self.name {
            self.config.local_uri = format!(
                "sip:{}@{}:{}",
                name, self.config.local_ip, self.config.sip_port
            );
        }
        StreamPeer::with_config(self.config).await
    }
}

impl Default for StreamPeerBuilder {
    fn default() -> Self {
        Self::new()
    }
}
