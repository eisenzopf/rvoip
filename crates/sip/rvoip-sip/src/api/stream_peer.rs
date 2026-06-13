//! Sequential peer API for clients, scripts, softphones, and tests.
//!
//! [`StreamPeer`] wraps a [`UnifiedCoordinator`]
//! with two ergonomic pieces:
//!
//! - [`PeerControl`] for commands such as `invite`, `accept`, registration, and
//!   early media. Chain `.with_extra_headers(...)` on the `invite` builder to
//!   attach caller-supplied typed headers to the very first INVITE for PBX/SBC
//!   integrations that require non-standard or vendor headers.
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
use crate::api::endpoint::SipAccount;
use crate::api::events::{Event, MediaSecurityState, SipTrace};
use crate::api::handle::{CallId, SessionHandle};
use crate::api::incoming::IncomingCall;
use crate::api::performance::PerformanceConfig;
use crate::api::unified::{
    Config, MediaMode, MediaSessionControllerConfig, RegistrationHandle, RegistrationInfo,
    RtpSessionBufferConfig, RtpTransportBufferConfig, UnifiedCoordinator,
};
use crate::auth::SipClientAuth;
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
/// # async fn example(mut events: rvoip_sip::EventReceiver) {
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
    /// Events synthesized at receiver-construction time to compensate for the
    /// subscribe-after-event race: the session-to-app channel is broadcast,
    /// so a subscriber added by `events_for_session` after the relevant
    /// transition already fired would otherwise hang. `events_for_session`
    /// inspects the session's current state and pushes the event the
    /// caller would have observed had they been subscribed earlier.
    primed: std::collections::VecDeque<Event>,
}

impl EventReceiver {
    pub(crate) fn new(
        rx: mpsc::Receiver<Arc<dyn rvoip_infra_common::events::cross_crate::CrossCrateEvent>>,
    ) -> Self {
        Self {
            rx,
            filter: None,
            primed: std::collections::VecDeque::new(),
        }
    }

    /// Create a receiver pre-filtered to a specific session.
    pub(crate) fn filtered(
        rx: mpsc::Receiver<Arc<dyn rvoip_infra_common::events::cross_crate::CrossCrateEvent>>,
        call_id: CallId,
    ) -> Self {
        Self {
            rx,
            filter: Some(call_id),
            primed: std::collections::VecDeque::new(),
        }
    }

    /// Push a synthesized event to the front of this receiver's queue.
    /// Used by `events_for_session` to repair the subscribe-after-event
    /// race for callers that registered a per-session subscriber after a
    /// state transition had already fired.
    pub(crate) fn prime(&mut self, event: Event) {
        self.primed.push_back(event);
    }

    /// Wait for the next event (optionally filtered to one session).
    ///
    /// Returns `None` when the coordinator shuts down.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(mut events: rvoip_sip::EventReceiver) {
    /// while let Some(event) = events.next().await {
    ///     println!("session event: {event:?}");
    /// }
    /// # }
    /// ```
    pub async fn next(&mut self) -> Option<Event> {
        // Drain primed events first — these were synthesized by
        // `events_for_session` so the caller observes a state transition
        // that fired before this receiver was subscribed.
        if let Some(event) = self.primed.pop_front() {
            return Some(event);
        }
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
    /// # fn example(mut events: rvoip_sip::EventReceiver) {
    /// if let Some(event) = events.try_next() {
    ///     println!("ready event: {event:?}");
    /// }
    /// # }
    /// ```
    pub fn try_next(&mut self) -> Option<Event> {
        if let Some(event) = self.primed.pop_front() {
            return Some(event);
        }
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
    /// # async fn example(mut events: rvoip_sip::EventReceiver) {
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
    /// # async fn example(mut events: rvoip_sip::EventReceiver) {
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
    /// # async fn example(mut events: rvoip_sip::EventReceiver) {
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

    /// Wait for the next SIP trace event, skipping all others.
    pub async fn next_sip_trace(&mut self) -> Option<SipTrace> {
        loop {
            match self.next().await? {
                Event::SipTrace(trace) => return Some(trace),
                _ => continue,
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
    /// # async fn example(mut events: rvoip_sip::EventReceiver) {
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
    /// # async fn example(mut events: rvoip_sip::EventReceiver) {
    /// let ended = events
    ///     .next_where(|event| matches!(event, rvoip_sip::Event::CallEnded { .. }))
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
    /// # async fn example(mut events: rvoip_sip::EventReceiver, call_id: rvoip_sip::CallId) {
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
    /// Accept an incoming call that was presented as an event.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(control: rvoip_sip::PeerControl, call_id: rvoip_sip::CallId) -> rvoip_sip::Result<()> {
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
    /// # async fn example(control: rvoip_sip::PeerControl, call_id: rvoip_sip::CallId) -> rvoip_sip::Result<()> {
    /// control.reject(&call_id, 486, "Busy Here").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn reject(&self, call_id: &CallId, status: u16, reason: &str) -> Result<()> {
        self.coordinator
            .reject(call_id)
            .with_status(status)
            .with_reason(reason.to_string())
            .send()
            .await
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
    /// # async fn example(control: rvoip_sip::PeerControl, call_id: rvoip_sip::CallId) -> rvoip_sip::Result<()> {
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
    /// # async fn example(control: rvoip_sip::PeerControl) -> rvoip_sip::Result<()> {
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

    /// Access the underlying [`UnifiedCoordinator`] for advanced use.
    ///
    /// This accessor is intentionally trivial and does not clone the
    /// coordinator.
    pub fn coordinator(&self) -> &Arc<UnifiedCoordinator> {
        &self.coordinator
    }

    /// Get the [`SessionHandle`] for a call by id.
    ///
    /// Works for inbound and outbound calls alike: pair it with the [`CallId`]
    /// returned by [`invite().send()`](crate::api::send::OutboundCallBuilder::send)
    /// to drive per-call control — hold/resume, mute, DTMF, transfer, audio —
    /// without reaching into [`coordinator()`](Self::coordinator).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(control: rvoip_sip::PeerControl, call_id: rvoip_sip::CallId) -> rvoip_sip::Result<()> {
    /// control.session(&call_id).hold().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn session(&self, call_id: &CallId) -> SessionHandle {
        SessionHandle::new(call_id.clone(), self.coordinator.clone())
    }

    /// Begin building an outbound INVITE from this peer's configured
    /// `local_uri`. Equivalent to
    /// `peer.coordinator().invite(Some(local_uri), target)`.
    pub fn invite(&self, target: impl Into<String>) -> crate::api::send::OutboundCallBuilder {
        self.coordinator
            .invite(Some(self.local_uri.clone()), target)
    }

    /// Begin building an outbound REGISTER from this peer. Equivalent to
    /// `peer.coordinator().register(registrar, user, pw)`.
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

    /// Send a REGISTER and await the registrar's final answer.
    ///
    /// [`register()`](Self::register)`.send()` returns as soon as the REGISTER
    /// is dispatched — success or failure arrives later as an
    /// [`Event::RegistrationSuccess`]/[`Event::RegistrationFailed`]. This helper
    /// subscribes first, sends, then resolves once the outcome lands, so a
    /// client doesn't advance its UI to "connected" before the binding exists.
    /// `timeout` of `None` waits indefinitely.
    ///
    /// Mirrors [`Endpoint::register_and_wait`](crate::api::endpoint::Endpoint::register_and_wait)
    /// on the reactive `StreamPeer`/`PeerControl` surface.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(control: rvoip_sip::PeerControl) -> rvoip_sip::Result<()> {
    /// let info = control
    ///     .register_and_wait(
    ///         "sip:pbx.example.com",
    ///         "2001",
    ///         "secret",
    ///         Some(std::time::Duration::from_secs(10)),
    ///     )
    ///     .await?;
    /// println!("registered; refresh in {:?}", info.next_refresh_in);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn register_and_wait(
        &self,
        registrar: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
        timeout: Option<std::time::Duration>,
    ) -> Result<RegistrationInfo> {
        // Subscribe before sending so the success/failure event can't be
        // emitted before the receiver exists.
        let mut events = self.subscribe_events().await?;
        let handle = self.register(registrar, username, password).send().await?;
        wait_for_peer_registration(&self.coordinator, &mut events, &handle, timeout).await
    }
}

/// Block on the REGISTER outcome for `handle`, resolving to the coordinator's
/// [`RegistrationInfo`] on success or a descriptive error on failure / timeout /
/// stream close. Matches the registrar reported for the handle so a peer with
/// several registrations in flight resolves the right one.
async fn wait_for_peer_registration(
    coordinator: &Arc<UnifiedCoordinator>,
    events: &mut EventReceiver,
    handle: &RegistrationHandle,
    timeout: Option<std::time::Duration>,
) -> Result<RegistrationInfo> {
    let registrar = coordinator
        .registration_info(handle)
        .await?
        .registrar
        .unwrap_or_default();
    let matches = |ev: &str| registrar.is_empty() || ev == registrar;
    let fut = async {
        loop {
            match events.next().await {
                Some(Event::RegistrationSuccess { registrar: r, .. }) if matches(&r) => {
                    return coordinator.registration_info(handle).await;
                }
                Some(Event::RegistrationFailed {
                    registrar: r,
                    status_code,
                    reason,
                }) if matches(&r) => {
                    return Err(SessionError::Other(format!(
                        "registration failed for {r}: {status_code} {reason}"
                    )));
                }
                Some(_) => {}
                None => {
                    return Err(SessionError::Other(
                        "event stream closed while waiting for registration".to_string(),
                    ));
                }
            }
        }
    };
    match timeout {
        Some(duration) => tokio::time::timeout(duration, fut)
            .await
            .map_err(|_| SessionError::Timeout("register_and_wait timed out".to_string()))?,
        None => fut.await,
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
/// use rvoip_sip::StreamPeer;
///
/// // UAC: make a call
/// let mut uac = StreamPeer::new("alice").await?;
/// let call_id = uac.invite("sip:bob@192.168.1.100:5060").send().await?;
/// let handle = uac.coordinator().session(&call_id);
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
    /// # async fn example() -> rvoip_sip::Result<()> {
    /// let peer = rvoip_sip::StreamPeer::new("alice").await?;
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
    /// # async fn example() -> rvoip_sip::Result<()> {
    /// use rvoip_sip::{Config, StreamPeer};
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
    /// # async fn example(peer: rvoip_sip::StreamPeer) {
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

    /// Shorthand for [`control().coordinator()`](PeerControl::coordinator).
    pub fn coordinator(&self) -> &Arc<UnifiedCoordinator> {
        self.control.coordinator()
    }

    /// Get the [`SessionHandle`] for a call by id. Delegates to
    /// [`PeerControl::session`].
    pub fn session(&self, call_id: &CallId) -> SessionHandle {
        self.control.session(call_id)
    }

    // ===== Sequential helpers =====

    /// Begin building an outbound INVITE from this peer's configured
    /// `local_uri`. Returns an
    /// [`OutboundCallBuilder`](crate::api::send::OutboundCallBuilder)
    /// that exposes `with_header`, `with_credentials`, `with_pai`,
    /// `with_headers_from`, etc. before dispatching.
    pub fn invite(&self, target: impl Into<String>) -> crate::api::send::OutboundCallBuilder {
        self.control.invite(target)
    }

    /// Begin building an outbound REGISTER. Delegates to
    /// [`PeerControl::register`].
    pub fn register(
        &self,
        registrar: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> crate::api::send::RegisterBuilder {
        self.control.register(registrar, username, password)
    }

    /// Begin building an outbound REGISTER from a shared SIP account.
    pub fn register_account(&self, account: &SipAccount) -> crate::api::send::RegisterBuilder {
        self.control.register_account(account)
    }

    /// Send a REGISTER and await the registrar's final answer. Delegates to
    /// [`PeerControl::register_and_wait`].
    pub async fn register_and_wait(
        &self,
        registrar: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
        timeout: Option<std::time::Duration>,
    ) -> Result<RegistrationInfo> {
        self.control
            .register_and_wait(registrar, username, password, timeout)
            .await
    }

    /// Wait for the next incoming call.
    ///
    /// Blocks until an [`Event::IncomingCall`] is received. The returned
    /// [`IncomingCall`] must be resolved (accepted, rejected, or deferred).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(mut peer: rvoip_sip::StreamPeer) -> rvoip_sip::Result<()> {
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
                    // SIP_API_DESIGN_2 Phase A: prefer the typed
                    // `Arc<Request>` view when the bus enriched the
                    // inbound INVITE; falls back to the legacy empty
                    // headers shape when synthesized in tests.
                    let coord = self.control.coordinator.clone();
                    let parsed = coord.session_registry.peek_pending_incoming_request().await;
                    let transport = coord
                        .session_registry
                        .peek_pending_incoming_transport()
                        .await;
                    let incoming = match parsed {
                        Some(req) => IncomingCall::with_request(call_id, from, to, sdp, coord, req),
                        None => IncomingCall::new(call_id, from, to, sdp, coord),
                    }
                    .with_transport_context(
                        transport
                            .as_deref()
                            .cloned()
                            .unwrap_or_else(crate::auth::SipTransportSecurityContext::unknown),
                    );
                    return Ok(incoming);
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
    /// # async fn example(mut peer: rvoip_sip::StreamPeer, call_id: rvoip_sip::CallId) -> rvoip_sip::Result<()> {
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
    /// # async fn example(mut peer: rvoip_sip::StreamPeer, call_id: rvoip_sip::CallId) -> rvoip_sip::Result<()> {
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
    /// # async fn example(mut peer: rvoip_sip::StreamPeer) {
    /// if let Some(event) = peer.next_event().await {
    ///     println!("{event:?}");
    /// }
    /// # }
    /// ```
    pub async fn next_event(&mut self) -> Option<Event> {
        self.events.next().await
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
    /// # async fn example(peer: rvoip_sip::StreamPeer, handle: rvoip_sip::RegistrationHandle) -> rvoip_sip::Result<()> {
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
    /// # async fn example(peer: rvoip_sip::StreamPeer, handle: rvoip_sip::RegistrationHandle) -> rvoip_sip::Result<()> {
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
    /// # async fn example(peer: rvoip_sip::StreamPeer) -> rvoip_sip::Result<()> {
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
    /// # async fn demo() -> rvoip_sip::Result<()> {
    /// use rvoip_sip::StreamPeer;
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
    /// # async fn example() -> rvoip_sip::Result<()> {
    /// let peer = rvoip_sip::StreamPeer::builder()
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
/// use rvoip_sip::StreamPeer;
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
    /// let builder = rvoip_sip::StreamPeerBuilder::new();
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
    /// # async fn example() -> rvoip_sip::Result<()> {
    /// let peer = rvoip_sip::StreamPeer::builder()
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
    /// # async fn example() -> rvoip_sip::Result<()> {
    /// let peer = rvoip_sip::StreamPeer::builder()
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
    /// # async fn example() -> rvoip_sip::Result<()> {
    /// let peer = rvoip_sip::StreamPeer::builder()
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
    /// # async fn example() -> rvoip_sip::Result<()> {
    /// let peer = rvoip_sip::StreamPeer::builder()
    ///     .media_ports(16000, 17000)
    ///     .build()
    ///     .await?;
    /// peer.shutdown().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn media_ports(mut self, start: u16, end: u16) -> Self {
        self.config = self.config.with_media_ports(start, end);
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

    /// Use a fully custom config (overrides all previous settings).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_sip::Result<()> {
    /// let config = rvoip_sip::Config::local("alice", 5060);
    /// let peer = rvoip_sip::StreamPeer::builder()
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

    /// Set per-peer default SIP Digest credentials used for UAC 401/407 retry.
    ///
    /// These credentials are the Digest shorthand. Use [`Self::with_auth`] for
    /// Bearer, Basic, AKA, or multi-challenge negotiation.
    ///
    /// Per-call override via `control.invite(...).with_credentials(...)`.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_sip::Result<()> {
    /// let peer = rvoip_sip::StreamPeer::builder()
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

    /// Set per-peer default UAC SIP auth used for 401/407 retry.
    ///
    /// Use [`SipClientAuth::any`] when the peer may offer multiple schemes and
    /// the UAC should negotiate among Digest, Bearer, Basic, and AKA options.
    pub fn with_auth(mut self, auth: SipClientAuth) -> Self {
        self.config.auth = Some(auth);
        self
    }

    /// Set per-peer Bearer auth used for UAC 401/407 retry.
    pub fn with_bearer_token(mut self, token: impl Into<String>) -> Self {
        self.config.auth = Some(SipClientAuth::bearer_token(token));
        self
    }

    /// Set per-peer Basic auth used for UAC 401/407 retry.
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

    /// Build the [`StreamPeer`].
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_sip::Result<()> {
    /// let peer = rvoip_sip::StreamPeerBuilder::new()
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

#[cfg(test)]
mod tests {
    use super::{EventReceiver, StreamPeerBuilder};
    use crate::adapters::SessionApiCrossCrateEvent;
    use crate::api::events::{Event, SipTrace, SipTraceDirection};
    use crate::api::unified::{
        MediaSessionControllerConfig, RtpSessionBufferConfig, RtpTransportBufferConfig,
    };
    use crate::state_table::types::SessionId;
    use rvoip_infra_common::events::cross_crate::CrossCrateEvent;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    #[test]
    fn stream_peer_builder_exposes_rtp_media_buffer_tuning() {
        let session_buffers = RtpSessionBufferConfig {
            sender_channel_capacity: 7,
            receiver_channel_capacity: 5,
            event_channel_capacity: 11,
        };
        let transport_buffers = RtpTransportBufferConfig {
            event_channel_capacity: 13,
            recv_buffer_size: 2048,
            rtcp_recv_buffer_size: 1024,
        };
        let mut media_config = MediaSessionControllerConfig::default();
        media_config.rtp_buffer_size = 960;
        media_config.rtp_buffer_initial_count = 3;
        media_config.rtp_buffer_max_count = 9;

        let builder = StreamPeerBuilder::new()
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
            3
        );
        assert_eq!(
            builder
                .config
                .media_session_controller_config
                .rtp_buffer_max_count,
            9
        );
    }

    #[tokio::test]
    async fn event_receiver_returns_sip_trace_events() {
        let (tx, rx) = mpsc::channel::<Arc<dyn CrossCrateEvent>>(4);
        tx.send(SessionApiCrossCrateEvent::new(Event::CallEnded {
            call_id: SessionId("other".into()),
            reason: "done".into(),
        }))
        .await
        .unwrap();
        tx.send(SessionApiCrossCrateEvent::new(Event::SipTrace(
            trace_event(),
        )))
        .await
        .unwrap();
        drop(tx);

        let mut receiver = EventReceiver::new(rx);
        let trace = receiver.next_sip_trace().await.unwrap();

        assert_eq!(trace.sip_call_id.as_deref(), Some("wire-call"));
        assert_eq!(trace.session_id, Some(SessionId("session-1".into())));
    }

    fn trace_event() -> SipTrace {
        SipTrace {
            direction: SipTraceDirection::Outbound,
            transport: "UDP".into(),
            local_addr: "127.0.0.1:5060".into(),
            remote_addr: "127.0.0.1:5080".into(),
            timestamp_unix_millis: 1,
            start_line: "INVITE sip:bob@example.com SIP/2.0".into(),
            sip_call_id: Some("wire-call".into()),
            session_id: Some(SessionId("session-1".into())),
            raw_message: "INVITE sip:bob@example.com SIP/2.0\n\n".into(),
            original_len: 40,
            truncated: false,
            redacted: true,
        }
    }
}
