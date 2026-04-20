//! Simplified Unified Session API
//!
//! This is a thin wrapper over the state machine helpers.
//! All business logic is in the state table.

use crate::state_table::types::{EventType, SessionId};
use crate::types::CallState;
use crate::state_machine::{StateMachine, StateMachineHelpers};
use crate::adapters::{DialogAdapter, MediaAdapter};
use crate::errors::{Result, SessionError};
use crate::types::{SessionInfo, IncomingCallInfo};
use crate::session_store::SessionStore;
use crate::session_registry::SessionRegistry;
// Callback system removed - using event-driven approach
use rvoip_media_core::types::AudioFrame;
use std::sync::Arc;
use std::net::{IpAddr, SocketAddr};
use tokio::sync::{mpsc, RwLock};
use rvoip_infra_common::events::coordinator::GlobalEventCoordinator;

pub use rvoip_dialog_core::api::RelUsage;
pub use rvoip_media_core::relay::controller::{AudioSource, BridgeError, BridgeHandle};

/// Configuration for the unified coordinator
#[derive(Debug, Clone)]
pub struct Config {
    /// Local IP address for media
    pub local_ip: IpAddr,
    /// SIP port
    pub sip_port: u16,
    /// Starting port for media
    pub media_port_start: u16,
    /// Ending port for media
    pub media_port_end: u16,
    /// Bind address for SIP
    pub bind_addr: SocketAddr,
    /// Optional path to custom state table YAML
    /// Priority: 1) This config path, 2) RVOIP_STATE_TABLE env var, 3) Embedded default
    pub state_table_path: Option<String>,
    /// Local SIP URI (e.g., "sip:alice@127.0.0.1:5060")
    pub local_uri: String,
    /// Policy for RFC 3262 `100rel` reliable provisionals on outgoing INVITE.
    ///
    /// Default is `Supported` — advertise capability without demanding it,
    /// which is the safe setting for interop and unchanged wire behavior.
    /// Set to `Required` when connecting to a carrier that mandates 100rel,
    /// or `NotSupported` to omit the tag entirely.
    pub use_100rel: RelUsage,

    /// RFC 4028 `Session-Expires` value in seconds to advertise on outgoing
    /// INVITEs. `None` disables session timers entirely. Common carrier
    /// value is 1800 (30 min).
    pub session_timer_secs: Option<u32>,

    /// Minimum-session-expires (`Min-SE:`) we're willing to accept, in
    /// seconds. Default 90 per RFC 4028 §5.
    pub session_timer_min_se: u32,

    /// Default credentials to apply to every outgoing call for RFC 3261 §22.2
    /// INVITE digest auth retry. When the server responds 401/407 to our
    /// INVITE, session-core looks here (or at per-call credentials passed
    /// via `PeerControl::call_with_auth`) to compute the digest response. When
    /// `None`, a 401/407 on INVITE surfaces as `CallFailed` instead of
    /// retrying. Default: `None`.
    pub credentials: Option<crate::types::Credentials>,
}

impl Config {
    /// Create a config for local development/testing on 127.0.0.1.
    ///
    /// ```
    /// # use rvoip_session_core::Config;
    /// let config = Config::local("alice", 5060);
    /// assert_eq!(config.local_uri, "sip:alice@127.0.0.1:5060");
    /// ```
    pub fn local(name: &str, port: u16) -> Self {
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        Self {
            local_ip: ip,
            sip_port: port,
            media_port_start: 16000,
            media_port_end: 17000,
            bind_addr: SocketAddr::new(ip, port),
            state_table_path: None,
            local_uri: format!("sip:{}@{}:{}", name, ip, port),
            use_100rel: RelUsage::default(),
            session_timer_secs: None,
            session_timer_min_se: 90,
            credentials: None,
        }
    }

    /// Create a config bound to a specific IP address (e.g. for LAN or production).
    ///
    /// ```
    /// # use rvoip_session_core::Config;
    /// let config = Config::on("alice", "192.168.1.50".parse().unwrap(), 5060);
    /// assert_eq!(config.local_uri, "sip:alice@192.168.1.50:5060");
    /// ```
    pub fn on(name: &str, ip: IpAddr, port: u16) -> Self {
        Self {
            local_ip: ip,
            sip_port: port,
            media_port_start: 16000,
            media_port_end: 17000,
            bind_addr: SocketAddr::new(ip, port),
            state_table_path: None,
            local_uri: format!("sip:{}@{}:{}", name, ip, port),
            use_100rel: RelUsage::default(),
            session_timer_secs: None,
            session_timer_min_se: 90,
            credentials: None,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config::local("user", 5060)
    }
}

/// Simplified coordinator that uses state machine helpers
#[allow(dead_code)]
pub struct UnifiedCoordinator {
    /// State machine helpers
    pub(crate) helpers: Arc<StateMachineHelpers>,

    /// Media adapter for audio operations
    media_adapter: Arc<MediaAdapter>,

    /// Dialog adapter for SIP operations
    dialog_adapter: Arc<DialogAdapter>,

    /// Incoming call receiver
    incoming_rx: Arc<RwLock<mpsc::Receiver<IncomingCallInfo>>>,

    /// Global event coordinator — used to publish and subscribe to session API events.
    /// Events are published to the "session_to_app" channel.
    pub(crate) global_coordinator: Arc<GlobalEventCoordinator>,

    /// Configuration
    config: Config,

    /// Shutdown signal — send `true` to stop all background tasks.
    shutdown_tx: tokio::sync::watch::Sender<bool>,
}

impl UnifiedCoordinator {
    /// Create a new coordinator
    pub async fn new(config: Config) -> Result<Arc<Self>> {
        // Get the global event coordinator singleton
        let global_coordinator = rvoip_infra_common::events::global_coordinator()
            .await
            .clone();
        
        // Create core components
        let store = Arc::new(SessionStore::new());
        let registry = Arc::new(SessionRegistry::new());
        
        // Create adapters
        let dialog_api = Self::create_dialog_api(&config, global_coordinator.clone()).await?;
        let dialog_adapter = Arc::new(DialogAdapter::new(
            dialog_api,
            store.clone(),
            global_coordinator.clone(),
        ));
        
        let media_controller = Self::create_media_controller(&config, global_coordinator.clone()).await?;
        let media_adapter = Arc::new(MediaAdapter::new(
            media_controller,
            store.clone(),
            config.local_ip,
            config.media_port_start,
            config.media_port_end,
        ));
        
        // Load state table based on config
        let state_table = Arc::new(
            crate::state_table::load_state_table_with_config(
                config.state_table_path.as_deref()
            )
        );
        
        // Create state machine without event channel (original constructor)
        let state_machine = Arc::new(StateMachine::new(
            state_table,
            store.clone(),
            dialog_adapter.clone(),
            media_adapter.clone(),
        ));
        
        // Wire the state machine into the dialog adapter (for REGISTER
        // response handling). The adapter holds an `Arc<OnceLock<_>>`
        // internally so this post-construction init is sound without
        // `unsafe`.
        let _ = dialog_adapter.init_state_machine(state_machine.clone());
        
        // Create helpers
        let helpers = Arc::new(StateMachineHelpers::new(state_machine.clone()));

        // Create incoming call channel
        let (incoming_tx, incoming_rx) = mpsc::channel(100);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        let coordinator = Arc::new(Self {
            helpers,
            media_adapter: media_adapter.clone(),
            dialog_adapter: dialog_adapter.clone(),
            incoming_rx: Arc::new(RwLock::new(incoming_rx)),
            global_coordinator: global_coordinator.clone(),
            config,
            shutdown_tx,
        });

        // Start the dialog adapter
        dialog_adapter.start().await?;

        // Create and start the centralized event handler.
        // Events are published to the global coordinator's "session_to_app" channel.
        let event_handler = crate::adapters::SessionCrossCrateEventHandler::with_event_broadcast(
            state_machine.clone(),
            global_coordinator.clone(),
            dialog_adapter.clone(),
            media_adapter.clone(),
            registry.clone(),
            incoming_tx,
        );

        // Start the event handler (sets up channels and subscriptions)
        event_handler.start(shutdown_rx).await?;

        Ok(coordinator)
    }
    
    /// Create a new coordinator with SimplePeer event integration.
    ///
    /// **Deprecated** — use [`UnifiedCoordinator::new()`] then [`subscribe_events()`][Self::subscribe_events].
    /// The `simple_peer_event_tx` parameter is ignored; events are now broadcast internally.
    #[deprecated(note = "Use UnifiedCoordinator::new() then subscribe_events()")]
    pub async fn with_simple_peer_events(
        config: Config,
        _simple_peer_event_tx: tokio::sync::mpsc::Sender<crate::api::events::Event>,
    ) -> Result<Arc<Self>> {
        Self::new(config).await
    }
    
    // ===== Event Subscription =====

    /// Subscribe to all session API events.
    ///
    /// Returns an [`mpsc::Receiver`] that receives every [`crate::api::events::Event`] published
    /// by this coordinator. Each call to `subscribe_events()` returns an independent receiver —
    /// all subscribers receive the same events (broadcast semantics via the global event bus).
    ///
    /// Events are published on the `"session_to_app"` channel. Use this to build custom peer
    /// types on top of `UnifiedCoordinator`, or to get a raw event stream.
    /// Shut down this coordinator and all its background tasks.
    ///
    /// After calling this, the coordinator stops processing events. Existing
    /// call sessions are not explicitly terminated — use [`hangup()`] first if
    /// you need clean call teardown.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Return a cloneable handle that can signal
    /// [`shutdown`](Self::shutdown) from another task. Mirrors
    /// [`CallbackPeer::shutdown_handle`].
    ///
    /// [`CallbackPeer::shutdown_handle`]: crate::api::callback_peer::CallbackPeer::shutdown_handle
    pub fn shutdown_handle(&self) -> crate::api::callback_peer::ShutdownHandle {
        crate::api::callback_peer::ShutdownHandle::from_sender(self.shutdown_tx.clone())
    }

    pub async fn subscribe_events(&self) -> crate::errors::Result<tokio::sync::mpsc::Receiver<std::sync::Arc<dyn rvoip_infra_common::events::cross_crate::CrossCrateEvent>>> {
        self.global_coordinator
            .subscribe(crate::adapters::SESSION_TO_APP_CHANNEL)
            .await
            .map_err(|e| crate::errors::SessionError::InternalError(
                format!("Failed to subscribe to session events: {}", e)
            ))
    }

    /// Return a typed, unfiltered [`EventReceiver`] that yields
    /// [`crate::api::events::Event`] values across all sessions.
    ///
    /// Use when a single consumer needs every session API event (b2bua
    /// coordinator, activity log). For per-leg logic prefer
    /// [`events_for_session`][Self::events_for_session].
    ///
    /// The returned receiver already handles the downcast from the raw
    /// cross-crate broadcast and exposes filtering helpers like
    /// [`EventReceiver::next_dtmf`], [`EventReceiver::next_incoming`], and
    /// [`EventReceiver::next_transfer`].
    pub async fn events(&self) -> Result<crate::api::stream_peer::EventReceiver> {
        let rx = self.subscribe_events().await?;
        Ok(crate::api::stream_peer::EventReceiver::new(rx))
    }

    /// Return an [`EventReceiver`] that only yields events whose
    /// `call_id` matches `id`. Per-session filtering happens in the
    /// receiver's `next()` loop.
    ///
    /// Intended for b2bua-style consumers that need to watch both legs of
    /// a bridged call independently:
    ///
    /// ```no_run
    /// # use rvoip_session_core::{Event, SessionId, UnifiedCoordinator};
    /// # async fn example(coord: &UnifiedCoordinator, inbound: &SessionId, outbound: &SessionId) {
    /// let mut inbound_events = coord.events_for_session(inbound).await.unwrap();
    /// let mut outbound_events = coord.events_for_session(outbound).await.unwrap();
    /// tokio::select! {
    ///     Some(Event::CallEnded { .. }) = inbound_events.next() => {
    ///         // inbound leg ended — tear down the outbound leg
    ///     }
    ///     Some(Event::CallEnded { .. }) = outbound_events.next() => {
    ///         // outbound leg ended — tear down the inbound leg
    ///     }
    /// }
    /// # }
    /// ```
    ///
    /// **Caller contract:** open the receiver *before* any event of
    /// interest fires. Events are lost if no subscriber is attached at
    /// publish time. For incoming calls the safe pattern is:
    /// 1. Wait for an `IncomingCall` event on the unfiltered
    ///    [`events()`][Self::events] receiver.
    /// 2. Open `events_for_session(&id)` with the new `SessionId`.
    /// 3. Call `accept_call_with_sdp()` (post-acceptance events then
    ///    reach the filtered receiver).
    pub async fn events_for_session(
        &self,
        id: &SessionId,
    ) -> Result<crate::api::stream_peer::EventReceiver> {
        let rx = self.subscribe_events().await?;
        Ok(crate::api::stream_peer::EventReceiver::filtered(rx, id.clone()))
    }

    // ===== Simple Call Operations =====

    /// Make an outgoing call. If the `Config.credentials` default is set,
    /// those credentials are applied to the session before dispatch so the
    /// state machine can transparently retry on a 401/407 INVITE challenge
    /// (RFC 3261 §22.2).
    pub async fn make_call(&self, from: &str, to: &str) -> Result<SessionId> {
        self.helpers
            .make_call_with_credentials(from, to, self.config.credentials.clone())
            .await
    }

    /// Make an outgoing call with explicit credentials, overriding the
    /// per-peer default. Useful for multi-tenant clients where each call
    /// authenticates with a different user.
    pub async fn make_call_with_auth(
        &self,
        from: &str,
        to: &str,
        credentials: crate::types::Credentials,
    ) -> Result<SessionId> {
        self.helpers
            .make_call_with_credentials(from, to, Some(credentials))
            .await
    }
    
    /// Accept an incoming call
    pub async fn accept_call(&self, session_id: &SessionId) -> Result<()> {
        self.helpers.accept_call(session_id).await
    }

    /// Accept an incoming call with a caller-supplied SDP answer. Bypasses
    /// local media negotiation — intended for b2bua flows where the answer
    /// body comes from the outbound leg's 200 OK. See
    /// [`StateMachineHelpers::accept_call_with_sdp`] for the mechanism.
    pub async fn accept_call_with_sdp(
        &self,
        session_id: &SessionId,
        sdp: String,
    ) -> Result<()> {
        self.helpers.accept_call_with_sdp(session_id, sdp).await
    }

    /// Reject an incoming call with a specific SIP status code and reason phrase.
    pub async fn reject_call(
        &self,
        session_id: &SessionId,
        status: u16,
        reason: &str,
    ) -> Result<()> {
        self.helpers.reject_call(session_id, status, reason).await
    }

    /// Redirect an incoming call to one or more alternate URIs (RFC 3261
    /// §8.1.3.4 / §21.3). Sends a 3xx response with a `Contact:` header
    /// listing the supplied URIs. `status` should be 300-399; `contacts`
    /// must be non-empty.
    pub async fn redirect_call(
        &self,
        session_id: &SessionId,
        status: u16,
        contacts: Vec<String>,
    ) -> Result<()> {
        self.helpers.redirect_call(session_id, status, contacts).await
    }
    
    /// Hangup a call
    pub async fn hangup(&self, session_id: &SessionId) -> Result<()> {
        self.helpers.hangup(session_id).await
    }

    /// Bridge the RTP streams of two active sessions at the media layer.
    ///
    /// Transparent packet-level relay: inbound RTP from session A is
    /// forwarded as outbound RTP on session B and vice versa, without
    /// transcoding. Intended for b2bua-style consumers that need to connect
    /// two SIP legs without shuffling AudioFrames through app code.
    ///
    /// # Preconditions
    ///
    /// - Both sessions must exist and be in `CallState::Active` (i.e. have
    ///   a negotiated remote RTP address).
    /// - Both sessions must have negotiated the same codec payload type.
    ///   Codec mismatch returns [`BridgeError::CodecMismatch`].
    /// - Neither session may already be bridged.
    ///
    /// Dropping the returned [`BridgeHandle`] tears the bridge down. DTMF
    /// (RFC 2833) rides the RTP stream and is forwarded transparently;
    /// RTCP is not bridged — each leg keeps generating its own reports.
    pub async fn bridge(
        &self,
        session_a: &SessionId,
        session_b: &SessionId,
    ) -> std::result::Result<BridgeHandle, BridgeError> {
        self.media_adapter.bridge_rtp_sessions(session_a, session_b).await
    }

    /// Send a reliable 183 Session Progress with early-media SDP (RFC 3262).
    ///
    /// - `sdp: Some(body)` sends the supplied SDP verbatim.
    /// - `sdp: None` generates an answer from the stored remote offer via
    ///   `MediaAdapter::negotiate_sdp_as_uas` (same path as `accept_call`).
    ///
    /// Fails fast with `UnreliableProvisionalsNotSupported` when the peer
    /// did not advertise `Supported: 100rel` on the INVITE. Transitions the
    /// session to `CallState::EarlyMedia`. Valid from `Ringing` and
    /// `EarlyMedia` (re-emission updates the SDP and bumps `RSeq`).
    pub async fn send_early_media(
        &self,
        session_id: &SessionId,
        sdp: Option<String>,
    ) -> Result<()> {
        if !self.dialog_adapter.peer_supports_100rel(session_id).await? {
            return Err(SessionError::UnreliableProvisionalsNotSupported);
        }
        self.helpers.send_early_media(session_id, sdp).await
    }

    /// Swap the audio source on the running transmitter for a session.
    ///
    /// Typical use: after [`send_early_media`][Self::send_early_media] has
    /// put the session into `EarlyMedia` (which starts a pass-through
    /// transmitter by default), call this to replace silence with a
    /// ringback tone, a "please hold" WAV, or any other
    /// [`AudioSource`][crate::api::unified::AudioSource] variant.
    ///
    /// On transition to `Active` (after `accept_call`), the state machine
    /// automatically swaps the transmitter back to `AudioSource::PassThrough`
    /// so bidirectional audio flows without further action from the app.
    /// Apps that want a *different* source after answer (e.g., continued
    /// announcement playback over an active call) should call this method
    /// again *after* the `CallEstablished` event fires.
    pub async fn set_audio_source(
        &self,
        session_id: &SessionId,
        source: AudioSource,
    ) -> Result<()> {
        self.media_adapter.set_audio_source(session_id, source).await
    }
    
    /// Put a call on hold
    pub async fn hold(&self, session_id: &SessionId) -> Result<()> {
        self.helpers.state_machine.process_event(
            session_id,
            EventType::HoldCall,
        ).await?;
        Ok(())
    }
    
    /// Resume a call from hold
    pub async fn resume(&self, session_id: &SessionId) -> Result<()> {
        self.helpers.state_machine.process_event(
            session_id,
            EventType::ResumeCall,
        ).await?;
        Ok(())
    }
    
    // ===== Conference Operations =====
    
    /// Create a conference from an active call
    pub async fn create_conference(&self, session_id: &SessionId, name: &str) -> Result<()> {
        self.helpers.create_conference(session_id, name).await
    }
    
    /// Add a participant to a conference
    pub async fn add_to_conference(
        &self,
        host_session_id: &SessionId,
        participant_session_id: &SessionId,
    ) -> Result<()> {
        self.helpers.add_to_conference(host_session_id, participant_session_id).await
    }
    
    /// Join an existing conference
    pub async fn join_conference(&self, session_id: &SessionId, conference_id: &str) -> Result<()> {
        self.helpers.state_machine.process_event(
            session_id,
            EventType::JoinConference { conference_id: conference_id.to_string() },
        ).await?;
        Ok(())
    }
    
    // ===== Event System Integration =====
    // Callback registry removed - using event-driven approach via SimplePeer
    
    /// Terminate the current session (for single session constraint)
    pub async fn terminate_current_session(&self) -> Result<()> {
        // Get the current session ID
        if let Some(session_id) = self.helpers.state_machine.store.get_current_session_id().await {
            self.hangup(&session_id).await
        } else {
            Ok(()) // No session to terminate
        }
    }
    
    /// Send REFER message to initiate transfer (this will trigger callback on recipient)
    pub async fn send_refer(&self, session_id: &SessionId, refer_to: &str) -> Result<()> {
        self.dialog_adapter.send_refer_session(session_id, refer_to).await
    }

    /// Send an in-dialog INFO request (RFC 6086) with a caller-chosen
    /// `Content-Type`.
    ///
    /// Used for SIP-INFO DTMF (`application/dtmf-relay` — some carriers
    /// prefer this over in-band RFC 2833), fax flow control
    /// (`application/sipfrag`), and other application-level mid-dialog
    /// signalling.
    ///
    /// The call must already be in an established dialog (past `Active`).
    /// The supplied `body` is sent verbatim; the method does not transcode
    /// or validate it against the declared content type.
    pub async fn send_info(
        &self,
        session_id: &SessionId,
        content_type: &str,
        body: &[u8],
    ) -> Result<()> {
        self.dialog_adapter
            .send_info(session_id, content_type, body)
            .await
    }
    
    /// Send NOTIFY message for REFER status (used after handling transfer)
    pub async fn send_refer_notify(&self, session_id: &SessionId, status_code: u16, reason: &str) -> Result<()> {
        self.dialog_adapter.send_refer_notify(session_id, status_code, reason).await
    }

    /// Send REFER with a pre-built `Replaces` header value (RFC 3891).
    ///
    /// Primitive for attended-transfer orchestration: a caller managing two
    /// sessions (original + consultation) constructs the Replaces value from
    /// the consultation session's [`DialogIdentity`] and passes it here for
    /// the original session to send.
    pub async fn send_refer_with_replaces(
        &self,
        session_id: &SessionId,
        target_uri: &str,
        replaces: &str,
    ) -> Result<()> {
        self.dialog_adapter
            .send_refer_with_replaces(session_id, target_uri, replaces)
            .await
    }

    /// Fetch the SIP-level identity (`Call-ID`, local/remote tags) of a
    /// session's dialog. Returns `None` if the dialog isn't established
    /// yet or has already been cleaned up.
    pub async fn dialog_identity(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<crate::api::types::DialogIdentity>> {
        self.dialog_adapter.dialog_identity(session_id).await
    }

    // ===== DTMF Operations =====
    
    /// Send DTMF digit
    pub async fn send_dtmf(&self, session_id: &SessionId, digit: char) -> Result<()> {
        self.helpers.state_machine.process_event(
            session_id,
            EventType::SendDTMF { digits: digit.to_string() },
        ).await?;
        Ok(())
    }
    
    // ===== Recording Operations =====
    
    /// Start recording a call
    pub async fn start_recording(&self, session_id: &SessionId) -> Result<()> {
        self.helpers.state_machine.process_event(
            session_id,
            EventType::StartRecording,
        ).await?;
        Ok(())
    }
    
    /// Stop recording a call
    pub async fn stop_recording(&self, session_id: &SessionId) -> Result<()> {
        self.helpers.state_machine.process_event(
            session_id,
            EventType::StopRecording,
        ).await?;
        Ok(())
    }
    
    // ===== Query Operations =====
    
    /// Get session information
    pub async fn get_session_info(&self, session_id: &SessionId) -> Result<SessionInfo> {
        self.helpers.get_session_info(session_id).await
    }
    
    /// List all active sessions
    pub async fn list_sessions(&self) -> Vec<SessionInfo> {
        self.helpers.list_sessions().await
    }
    
    /// Get current state of a session
    pub async fn get_state(&self, session_id: &SessionId) -> Result<CallState> {
        self.helpers.get_state(session_id).await
    }
    
    /// Check if session is in conference
    pub async fn is_in_conference(&self, session_id: &SessionId) -> Result<bool> {
        self.helpers.is_in_conference(session_id).await
    }
    
    // ===== Audio Operations =====
    
    /// Subscribe to audio frames for a session
    pub async fn subscribe_to_audio(
        &self,
        session_id: &SessionId,
    ) -> Result<crate::types::AudioFrameSubscriber> {
        self.media_adapter.subscribe_to_audio_frames(session_id).await
    }
    
    /// Send audio frame to a session
    pub async fn send_audio(&self, session_id: &SessionId, frame: AudioFrame) -> Result<()> {
        self.media_adapter.send_audio_frame(session_id, frame).await
    }
    
    // ===== Event Subscriptions =====
    
    /// Subscribe to session events
    pub async fn subscribe<F>(&self, session_id: SessionId, callback: F)
    where
        F: Fn(crate::state_machine::helpers::SessionEvent) + Send + Sync + 'static,
    {
        self.helpers.subscribe(session_id, callback).await
    }
    
    /// Unsubscribe from session events
    pub async fn unsubscribe(&self, session_id: &SessionId) {
        self.helpers.unsubscribe(session_id).await
    }
    
    // ===== Incoming Call Handling =====

    /// Get the next incoming call
    pub async fn get_incoming_call(&self) -> Option<IncomingCallInfo> {
        self.incoming_rx.write().await.recv().await
    }

    // ===== Auto-Transfer Handling =====

    /// Enable automatic blind transfer handling - DISABLED
    /// Auto-transfer now handled in SessionEventHandler to avoid event stealing
    pub fn enable_auto_transfer(self: &Arc<Self>) {
        tracing::info!("🔄 Auto-transfer: handled by SessionEventHandler");
    }

    // extract_field method removed - no longer needed without transfer coordinator
    
    // ===== Server-Side Registration =====
    
    /// Start server-side registration handling
    /// 
    /// This creates and starts a RegistrationAdapter that handles incoming REGISTER
    /// requests via the global event bus. The registrar service authenticates users
    /// and manages registrations.
    /// 
    /// # Arguments
    /// * `realm` - The SIP realm for digest authentication (e.g., "example.com")
    /// * `users` - Map of username -> password for authentication
    /// 
    /// # Returns
    /// Arc<RegistrarService> - The registrar service for managing registrations
    pub async fn start_registration_server(
        &self,
        realm: &str,
        users: std::collections::HashMap<String, String>,
    ) -> Result<Arc<rvoip_registrar_core::RegistrarService>> {
        use rvoip_registrar_core::{RegistrarService, api::ServiceMode, types::RegistrarConfig};
        use crate::adapters::RegistrationAdapter;
        
        tracing::info!("🔐 Starting server-side registration handler with realm: {}", realm);
        
        // Create registrar service with authentication
        let registrar = RegistrarService::with_auth(
            ServiceMode::B2BUA,
            RegistrarConfig::default(),
            realm,
        ).await
        .map_err(|e| SessionError::InternalError(format!("Failed to create registrar: {}", e)))?;
        
        // Add users to the registrar
        if let Some(user_store) = registrar.user_store() {
            for (username, password) in users {
                user_store.add_user(&username, &password)
                    .map_err(|e| SessionError::InternalError(format!("Failed to add user: {}", e)))?;
                tracing::debug!("Added user: {}", username);
            }
        }
        
        let registrar = Arc::new(registrar);
        
        // Get the global event coordinator
        let global_coordinator = rvoip_infra_common::events::global_coordinator()
            .await
            .clone();
        
        // Create and start the registration adapter
        let adapter = Arc::new(RegistrationAdapter::new(
            registrar.clone(),
            global_coordinator,
        ));
        
        adapter.start().await
            .map_err(|e| SessionError::InternalError(format!("Failed to start registration adapter: {}", e)))?;
        
        tracing::info!("✅ Server-side registration handler started");
        
        Ok(registrar)
    }

    // ===== Internal Helpers =====
    
    async fn create_dialog_api(config: &Config, global_coordinator: Arc<GlobalEventCoordinator>) -> Result<Arc<rvoip_dialog_core::api::unified::UnifiedDialogApi>> {
        use rvoip_dialog_core::config::DialogManagerConfig;
        use rvoip_dialog_core::api::unified::UnifiedDialogApi;
        use rvoip_dialog_core::transaction::{TransactionManager, transport::{TransportManager, TransportManagerConfig}};
        
        // Create transport manager first (dialog-core's own transport manager)
        let transport_config = TransportManagerConfig {
            enable_udp: true,
            enable_tcp: false,
            enable_ws: false,
            enable_tls: false,
            bind_addresses: vec![config.bind_addr],
            ..Default::default()
        };
        
        let (mut transport_manager, transport_event_rx) = TransportManager::new(transport_config)
            .await
            .map_err(|e| SessionError::InternalError(format!("Failed to create transport manager: {}", e)))?;
        
        // Initialize the transport manager
        transport_manager.initialize()
            .await
            .map_err(|e| SessionError::InternalError(format!("Failed to initialize transport: {}", e)))?;
        
        // Create transaction manager using transport manager
        let (transaction_manager, event_rx) = TransactionManager::with_transport_manager(
            transport_manager,
            transport_event_rx,
            None, // No max transactions limit
        )
        .await
        .map_err(|e| SessionError::InternalError(format!("Failed to create transaction manager: {}", e)))?;
        
        let transaction_manager = Arc::new(transaction_manager);
        
        // Create dialog config - use hybrid mode to support both incoming and outgoing calls
        let dialog_config = DialogManagerConfig::hybrid(config.bind_addr)
            .with_from_uri(&config.local_uri)
            .with_100rel(config.use_100rel)
            .with_session_timer(config.session_timer_secs)
            .with_min_se(config.session_timer_min_se)
            .build();
        
        // Create dialog API with global event coordination AND transaction events
        let dialog_api = Arc::new(
            UnifiedDialogApi::with_global_events_and_coordinator(
                transaction_manager, 
                event_rx,
                dialog_config,
                global_coordinator.clone()
            )
            .await
            .map_err(|e| SessionError::InternalError(format!("Failed to create dialog API: {}", e)))?
        );
        
        dialog_api.start().await
            .map_err(|e| SessionError::InternalError(format!("Failed to start dialog API: {}", e)))?;
        
        Ok(dialog_api)
    }
    
    
    async fn create_media_controller(
        config: &Config,
        global_coordinator: Arc<GlobalEventCoordinator>
    ) -> Result<Arc<rvoip_media_core::relay::controller::MediaSessionController>> {
        use rvoip_media_core::relay::controller::MediaSessionController;
        
        // Create media controller with port range
        let controller = Arc::new(
            MediaSessionController::with_port_range(
                config.media_port_start,
                config.media_port_end
            )
        );
        
        // Create and set up the event hub
        let event_hub = rvoip_media_core::events::MediaEventHub::new(
            global_coordinator,
            controller.clone(),
        ).await
        .map_err(|e| SessionError::InternalError(format!("Failed to create media event hub: {}", e)))?;
        
        // Set the event hub on the media controller
        controller.set_event_hub(event_hub).await;

        Ok(controller)
    }
}

/// Simple helper to create a session and make a call
impl UnifiedCoordinator {
    /// Quick method to create a UAC session and make a call
    pub async fn quick_call(&self, from: &str, to: &str) -> Result<SessionId> {
        self.make_call(from, to).await
    }
}

/// Registration API
impl UnifiedCoordinator {
    /// Register with SIP server
    ///
    /// # Arguments
    /// * `registrar_uri` - URI of the registrar server (e.g., "sip:registrar.example.com")
    /// * `from_uri` - From URI (e.g., "sip:user@example.com")
    /// * `contact_uri` - Contact URI (e.g., "sip:user@192.168.1.100:5060")
    /// * `username` - Username for authentication
    /// * `password` - Password for digest authentication
    /// * `expires` - Registration expiry in seconds (typically 3600)
    ///
    /// # Returns
    /// A `RegistrationHandle` that can be used to unregister or refresh
    pub async fn register(
        &self,
        registrar_uri: &str,
        from_uri: &str,
        contact_uri: &str,
        username: &str,
        password: &str,
        expires: u32,
    ) -> Result<RegistrationHandle> {
        // Create registration session
        let session_id = SessionId::new();
        tracing::info!("📝 Created registration session: {}", session_id.0);
        self.helpers.create_session(
            session_id.clone(),
            from_uri.to_string(),
            registrar_uri.to_string(),
            crate::state_table::types::Role::UAC
        ).await?;
        
        // Store credentials
        let credentials = crate::types::Credentials::new(username, password);
        
        // Get session store and update
        let session_store = &self.helpers.state_machine.store;
        let mut session = session_store.get_session(&session_id).await?;
        session.credentials = Some(credentials);
        session.registrar_uri = Some(registrar_uri.to_string());
        session.registration_contact = Some(contact_uri.to_string());
        session.registration_expires = Some(expires);
        session_store.update_session(session).await?;
        
        // Trigger registration via state machine
        let _result = self.helpers.state_machine.process_event(&session_id, crate::state_table::types::EventType::StartRegistration).await
            .map_err(|e| SessionError::InternalError(format!("Failed to trigger registration: {}", e)))?;
        
        Ok(RegistrationHandle { session_id })
    }

    /// Register with a SIP server using a [`Registration`] builder.
    ///
    /// This is the preferred way to register — `from_uri` and `contact_uri`
    /// default to the peer's `local_uri` from [`Config`].
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_session_core::UnifiedCoordinator>) -> rvoip_session_core::Result<()> {
    /// use rvoip_session_core::Registration;
    ///
    /// let handle = coordinator.register_with(
    ///     Registration::new("sip:registrar.example.com", "alice", "secret123")
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn register_with(&self, reg: Registration) -> Result<RegistrationHandle> {
        let from_uri = reg.from_uri.as_deref().unwrap_or(&self.config.local_uri);
        let contact_uri = reg.contact_uri.as_deref().unwrap_or(&self.config.local_uri);
        self.register(&reg.registrar, from_uri, contact_uri, &reg.username, &reg.password, reg.expires).await
    }
    
    /// Unregister from SIP server
    ///
    /// Sends REGISTER with expires=0 to remove registration
    pub async fn unregister(&self, handle: &RegistrationHandle) -> Result<()> {
        // Trigger unregistration via state machine
        let _result = self.helpers.state_machine.process_event(
            &handle.session_id,
            crate::state_table::types::EventType::StartUnregistration
        ).await
            .map_err(|e| SessionError::InternalError(format!("Failed to trigger unregistration: {}", e)))?;
        Ok(())
    }
    
    /// Refresh registration before it expires
    ///
    /// Sends a new REGISTER request with the same expiry time
    pub async fn refresh_registration(&self, handle: &RegistrationHandle) -> Result<()> {
        // Trigger refresh via state machine
        let _result = self.helpers.state_machine.process_event(
            &handle.session_id,
            crate::state_table::types::EventType::RefreshRegistration
        ).await
            .map_err(|e| SessionError::InternalError(format!("Failed to trigger refresh: {}", e)))?;
        Ok(())
    }
    
    /// Get registration status
    pub async fn is_registered(&self, handle: &RegistrationHandle) -> Result<bool> {
        let session = self.helpers.state_machine.store.get_session(&handle.session_id).await?;
        tracing::info!("🔍 Checking registration for session {}: is_registered={}, retry_count={}",
                       handle.session_id.0, session.is_registered, session.registration_retry_count);
        Ok(session.is_registered)
    }
}

/// Handle for managing a registration
#[derive(Debug, Clone)]
pub struct RegistrationHandle {
    pub session_id: SessionId,
}

/// Configuration for SIP registration.
///
/// Use [`Registration::new()`] for the common case where `from_uri` and
/// `contact_uri` are derived from the peer's [`Config`].
///
/// # Example
///
/// ```
/// use rvoip_session_core::Registration;
///
/// let reg = Registration::new("sip:registrar.example.com", "alice", "secret123")
///     .expires(1800);
/// ```
#[derive(Debug, Clone)]
pub struct Registration {
    /// SIP URI of the registrar server (e.g. `sip:registrar.example.com`)
    pub registrar: String,
    /// Username for digest authentication
    pub username: String,
    /// Password for digest authentication
    pub password: String,
    /// Registration expiry in seconds (default: 3600)
    pub expires: u32,
    /// Override the From URI (defaults to the peer's local_uri)
    pub from_uri: Option<String>,
    /// Override the Contact URI (defaults to the peer's local_uri)
    pub contact_uri: Option<String>,
}

impl Registration {
    /// Create a registration with the minimum required fields.
    ///
    /// `from_uri` and `contact_uri` will be derived from the peer's config.
    pub fn new(registrar: impl Into<String>, username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            registrar: registrar.into(),
            username: username.into(),
            password: password.into(),
            expires: 3600,
            from_uri: None,
            contact_uri: None,
        }
    }

    /// Set the registration expiry in seconds (default: 3600).
    pub fn expires(mut self, secs: u32) -> Self {
        self.expires = secs;
        self
    }

    /// Override the From URI (defaults to the peer's local_uri).
    pub fn from_uri(mut self, uri: impl Into<String>) -> Self {
        self.from_uri = Some(uri.into());
        self
    }

    /// Override the Contact URI (defaults to the peer's local_uri).
    pub fn contact_uri(mut self, uri: impl Into<String>) -> Self {
        self.contact_uri = Some(uri.into());
        self
    }
}
