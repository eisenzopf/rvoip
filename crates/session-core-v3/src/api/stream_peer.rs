//! StreamPeer — sequential / event-loop SIP peer.
//!
//! `StreamPeer` provides a sequential API suitable for test code, softphones,
//! and simple scripts. All events arrive via `next_event()` or the `wait_for_*`
//! helper methods.
//!
//! For reactive server code (proxies, IVR engines) use [`CallbackPeer`] instead.
//!
//! [`CallbackPeer`]: crate::api::callback_peer::CallbackPeer

use std::net::IpAddr;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::api::events::Event;
use crate::api::handle::{CallId, SessionHandle};
use crate::api::incoming::IncomingCall;
use crate::api::unified::{Config, UnifiedCoordinator};
use crate::adapters::SessionApiCrossCrateEvent;
use crate::errors::{Result, SessionError};

// Re-export Config so callers can import it from this module
pub use crate::api::unified::Config as PeerConfig;

// ===== EventReceiver =====

/// A receiver for session API events.
///
/// Obtained via [`StreamPeer::next_event()`], [`SessionHandle::events()`], or
/// [`PeerControl::subscribe_events()`]. Each `EventReceiver` is independent —
/// slow consumers do not affect others.
///
/// Events flow through the [`GlobalEventCoordinator`]'s `"session_to_app"` channel,
/// which uses a lock-free broadcast internally.
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
        Self { rx, filter: Some(call_id) }
    }

    /// Wait for the next event (optionally filtered to one session).
    ///
    /// Returns `None` when the coordinator shuts down.
    pub async fn next(&mut self) -> Option<Event> {
        loop {
            let raw = self.rx.recv().await?;
            // Downcast from Arc<dyn CrossCrateEvent> to our concrete wrapper
            let session_event = raw
                .as_any()
                .downcast_ref::<SessionApiCrossCrateEvent>()?;
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
    pub fn try_next(&mut self) -> Option<Event> {
        loop {
            let raw = self.rx.try_recv().ok()?;
            let session_event = raw
                .as_any()
                .downcast_ref::<SessionApiCrossCrateEvent>()?;
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
    pub async fn next_incoming(&mut self) -> Option<(CallId, String, String, Option<String>)> {
        loop {
            match self.next().await? {
                Event::IncomingCall { call_id, from, to, sdp } => {
                    return Some((call_id, from, to, sdp));
                }
                _ => continue,
            }
        }
    }

    /// Wait for the next DTMF digit on any call, skipping all others.
    ///
    /// Returns `(call_id, digit)` or `None` on channel close.
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

    /// Wait for the next transfer-related event, skipping all others.
    ///
    /// Matches `ReferReceived`, `TransferAccepted`, `TransferCompleted`,
    /// `TransferFailed`, and `TransferProgress`.
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
    pub async fn next_where<F: FnMut(&Event) -> bool>(&mut self, mut predicate: F) -> Option<Event> {
        loop {
            let event = self.next().await?;
            if predicate(&event) {
                return Some(event);
            }
        }
    }

    /// Wait for the next event belonging to `call_id`, skipping others.
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
    /// Use [`subscribe_events()`] to watch for [`Event::CallAnswered`].
    ///
    /// [`subscribe_events()`]: Self::subscribe_events
    pub async fn call(&self, target: &str) -> Result<SessionHandle> {
        let id = self.coordinator.make_call(&self.local_uri, target).await?;
        Ok(SessionHandle::new(id, self.coordinator.clone()))
    }

    /// Accept an incoming call that was presented as an event.
    pub async fn accept(&self, call_id: &CallId) -> Result<SessionHandle> {
        self.coordinator.accept_call(call_id).await?;
        Ok(SessionHandle::new(call_id.clone(), self.coordinator.clone()))
    }

    /// Reject an incoming call with the given SIP status code and reason phrase.
    pub async fn reject(&self, call_id: &CallId, status: u16, reason: &str) -> Result<()> {
        self.coordinator.reject_call(call_id, status, reason).await
    }

    /// Subscribe to all events from this coordinator.
    ///
    /// Each call returns an independent receiver (broadcast semantics).
    pub async fn subscribe_events(&self) -> Result<EventReceiver> {
        let rx = self.coordinator.subscribe_events().await?;
        Ok(EventReceiver::new(rx))
    }

    /// Access the underlying `UnifiedCoordinator` for advanced use.
    pub fn coordinator(&self) -> &Arc<UnifiedCoordinator> {
        &self.coordinator
    }
}

// ===== StreamPeer =====

/// A sequential SIP peer with event-stream-style access.
///
/// # Quick start
///
/// ```rust,no_run
/// # async fn example() -> anyhow::Result<()> {
/// use rvoip_session_core_v3::StreamPeer;
///
/// // UAC: make a call
/// let mut uac = StreamPeer::new("alice").await?;
/// let handle = uac.call("sip:bob@192.168.1.100:5060").await?;
/// let handle = uac.wait_for_answered(handle.id()).await?;
/// handle.hangup().await?;
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
    pub async fn new(name: &str) -> Result<Self> {
        let mut config = Config::default();
        config.local_uri = format!("sip:{}@{}:{}", name, config.local_ip, config.sip_port);
        Self::with_config(config).await
    }

    /// Create a peer with explicit configuration.
    pub async fn with_config(config: Config) -> Result<Self> {
        let local_uri = config.local_uri.clone();
        let coordinator = UnifiedCoordinator::new(config).await?;
        let event_rx = coordinator.subscribe_events().await?;
        Ok(Self {
            control: PeerControl { coordinator, local_uri },
            events: EventReceiver::new(event_rx),
        })
    }

    /// Split the peer into independent control and event halves.
    ///
    /// Useful when you want to drive the event loop in one task while issuing
    /// commands from another.
    pub fn split(self) -> (PeerControl, EventReceiver) {
        (self.control, self.events)
    }

    /// Access the command half without consuming the peer.
    pub fn control(&self) -> &PeerControl {
        &self.control
    }

    // ===== Sequential helpers =====

    /// Initiate an outgoing call.
    pub async fn call(&mut self, target: &str) -> Result<SessionHandle> {
        self.control.call(target).await
    }

    /// Wait for the next incoming call.
    ///
    /// Blocks until an [`Event::IncomingCall`] is received. The returned
    /// [`IncomingCall`] must be resolved (accepted, rejected, or deferred).
    pub async fn wait_for_incoming(&mut self) -> Result<IncomingCall> {
        loop {
            match self.events.next().await {
                Some(Event::IncomingCall { call_id, from, to, sdp }) => {
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
    pub async fn wait_for_answered(&mut self, call_id: &CallId) -> Result<SessionHandle> {
        loop {
            match self.events.next().await {
                Some(Event::CallAnswered { call_id: answered_id, .. })
                    if &answered_id == call_id =>
                {
                    return Ok(SessionHandle::new(
                        answered_id,
                        self.control.coordinator.clone(),
                    ));
                }
                Some(Event::CallFailed { call_id: failed_id, reason, status_code })
                    if &failed_id == call_id =>
                {
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

    /// Wait for a specific call to end (BYE received/sent).
    pub async fn wait_for_ended(&mut self, call_id: &CallId) -> Result<String> {
        loop {
            match self.events.next().await {
                Some(Event::CallEnded { call_id: ended_id, reason })
                    if &ended_id == call_id =>
                {
                    return Ok(reason);
                }
                None => return Err(SessionError::Other("Event channel closed".to_string())),
                _ => {}
            }
        }
    }

    /// Read the next event without filtering.
    pub async fn next_event(&mut self) -> Option<Event> {
        self.events.next().await
    }

    /// Register with a SIP server (6-arg form).
    ///
    /// Prefer [`register_with()`](Self::register_with) which uses a builder and
    /// derives `from_uri`/`contact_uri` from the peer's config.
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
            .register(registrar_uri, from_uri, contact_uri, username, password, expires)
            .await
    }

    /// Register with a SIP server using a [`Registration`](crate::Registration) builder.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_session_core_v3::Result<()> {
    /// use rvoip_session_core_v3::{StreamPeer, Registration};
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

    /// Graceful shutdown — stops background tasks and drops resources.
    ///
    /// Previously `SimplePeer::shutdown()` called `process::exit(0)`. This version
    /// cleanly drops the coordinator, causing background tasks to terminate when
    /// they next observe their channels are closed.
    pub async fn shutdown(self) -> Result<()> {
        // Signal the coordinator to stop its background event loops,
        // then drop self so remaining Arc references decrease.
        self.control.coordinator.shutdown();
        drop(self);
        Ok(())
    }

    /// Start building a new `StreamPeer` with configuration options.
    pub fn builder() -> StreamPeerBuilder {
        StreamPeerBuilder::new()
    }
}

// ===== StreamPeerBuilder =====

/// Builder for [`StreamPeer`] with fluent configuration.
///
/// # Example
///
/// ```rust,no_run
/// # async fn example() -> anyhow::Result<()> {
/// use rvoip_session_core_v3::StreamPeer;
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
    pub fn new() -> Self {
        Self {
            config: Config::default(),
            name: None,
        }
    }

    /// Set the display name (auto-generates a SIP URI from it).
    pub fn name(mut self, name: &str) -> Self {
        self.name = Some(name.to_string());
        self
    }

    /// Set the SIP port.
    pub fn sip_port(mut self, port: u16) -> Self {
        self.config.sip_port = port;
        self.config.bind_addr.set_port(port);
        self
    }

    /// Set the local IP address.
    pub fn local_ip(mut self, ip: IpAddr) -> Self {
        self.config.local_ip = ip;
        self.config.bind_addr.set_ip(ip);
        self
    }

    /// Set the media port range.
    pub fn media_ports(mut self, start: u16, end: u16) -> Self {
        self.config.media_port_start = start;
        self.config.media_port_end = end;
        self
    }

    /// Use a fully custom config (overrides all previous settings).
    pub fn config(mut self, config: Config) -> Self {
        self.config = config;
        self
    }

    /// Build the `StreamPeer`.
    pub async fn build(mut self) -> Result<StreamPeer> {
        if let Some(name) = self.name {
            self.config.local_uri =
                format!("sip:{}@{}:{}", name, self.config.local_ip, self.config.sip_port);
        }
        StreamPeer::with_config(self.config).await
    }
}

impl Default for StreamPeerBuilder {
    fn default() -> Self {
        Self::new()
    }
}
