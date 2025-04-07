use std::fmt;
use std::sync::Arc;
use std::collections::HashMap;
use std::net::SocketAddr;
use uuid::Uuid;
use tokio::sync::{RwLock, Mutex};
use anyhow::Result;
use tracing::{debug, info, warn, error};
use dashmap::DashMap;
use serde::{Serialize, Deserialize};
use std::time::Duration;

use rvoip_sip_core::{
    Request, Response, Method, StatusCode,
    Uri, Header, HeaderName,
};
use rvoip_transaction_core::TransactionManager;

use crate::dialog::{Dialog, DialogId, DialogState};
use crate::media::{MediaStream, MediaConfig, MediaType, AudioCodecType};
use crate::events::{EventBus, SessionEvent};
use crate::errors::Error;

/// Unique identifier for a SIP session
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub Uuid);

impl SessionId {
    /// Create a new session ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

/// SIP session state
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    /// Session is being initialized
    Initializing,
    
    /// Outgoing call is being established
    Dialing,
    
    /// Incoming call is being received
    Ringing,
    
    /// Call is connected and active
    Connected,
    
    /// Call is on hold
    OnHold,
    
    /// Call is being transferred
    Transferring,
    
    /// Call is being terminated
    Terminating,
    
    /// Call has ended
    Terminated,
}

impl fmt::Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionState::Initializing => write!(f, "Initializing"),
            SessionState::Dialing => write!(f, "Dialing"),
            SessionState::Ringing => write!(f, "Ringing"),
            SessionState::Connected => write!(f, "Connected"),
            SessionState::OnHold => write!(f, "OnHold"),
            SessionState::Transferring => write!(f, "Transferring"),
            SessionState::Terminating => write!(f, "Terminating"),
            SessionState::Terminated => write!(f, "Terminated"),
        }
    }
}

/// Session direction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionDirection {
    /// Outgoing call
    Outgoing,
    /// Incoming call
    Incoming,
}

/// Session configuration
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// Local address for signaling
    pub local_signaling_addr: SocketAddr,
    
    /// Local address for media
    pub local_media_addr: SocketAddr,
    
    /// Supported audio codecs
    pub supported_codecs: Vec<AudioCodecType>,
    
    /// Default display name
    pub display_name: Option<String>,
    
    /// User agent identifier
    pub user_agent: String,
    
    /// Maximum call duration in seconds (0 for unlimited)
    pub max_duration: u32,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            local_signaling_addr: "0.0.0.0:5060".parse().unwrap(),
            local_media_addr: "0.0.0.0:10000".parse().unwrap(),
            supported_codecs: vec![AudioCodecType::PCMU, AudioCodecType::PCMA],
            display_name: None,
            user_agent: "RVOIP/0.1.0".to_string(),
            max_duration: 0,
        }
    }
}

/// A SIP call session
pub struct Session {
    /// Unique identifier for this session
    pub id: SessionId,
    
    /// Current state of the session
    state: RwLock<SessionState>,
    
    /// Direction of the session
    direction: SessionDirection,
    
    /// Main dialog for this session
    dialog: RwLock<Option<Dialog>>,
    
    /// Related dialogs (e.g., for call transfers)
    related_dialogs: RwLock<HashMap<DialogId, Dialog>>,
    
    /// Media stream
    media: RwLock<Option<MediaStream>>,
    
    /// Configuration
    config: SessionConfig,
    
    /// Event bus for publishing session events
    event_bus: EventBus,
    
    /// Transaction manager reference
    transaction_manager: Arc<TransactionManager>,
}

impl Session {
    /// Create a new session
    pub fn new(
        direction: SessionDirection, 
        config: SessionConfig,
        transaction_manager: Arc<TransactionManager>,
        event_bus: EventBus,
    ) -> Self {
        let id = SessionId::new();
        
        let session = Self {
            id: id.clone(),
            state: RwLock::new(SessionState::Initializing),
            direction,
            dialog: RwLock::new(None),
            related_dialogs: RwLock::new(HashMap::new()),
            media: RwLock::new(None),
            config,
            event_bus: event_bus.clone(),
            transaction_manager,
        };
        
        // Publish session created event
        event_bus.publish(SessionEvent::Created { session_id: id });
        
        session
    }
    
    /// Get the current session state
    pub async fn state(&self) -> SessionState {
        self.state.read().await.clone()
    }
    
    /// Set the session state
    pub async fn set_state(&self, new_state: SessionState) -> Result<()> {
        let old_state = self.state.read().await.clone();
        
        // Check if this is a valid state transition
        if !Self::is_valid_transition(&old_state, &new_state) {
            return Err(Error::InvalidStateTransition(
                old_state.to_string(),
                new_state.to_string(),
            ).into());
        }
        
        // Update state
        *self.state.write().await = new_state.clone();
        
        // Publish state change event
        self.event_bus.publish(SessionEvent::StateChanged {
            session_id: self.id.clone(),
            old_state,
            new_state,
        });
        
        Ok(())
    }
    
    /// Check if the state transition is valid
    fn is_valid_transition(from: &SessionState, to: &SessionState) -> bool {
        match (from, to) {
            // From Initializing
            (SessionState::Initializing, SessionState::Dialing) => true,
            (SessionState::Initializing, SessionState::Ringing) => true,
            (SessionState::Initializing, SessionState::Terminated) => true,
            
            // From Dialing
            (SessionState::Dialing, SessionState::Connected) => true,
            (SessionState::Dialing, SessionState::Terminating) => true,
            (SessionState::Dialing, SessionState::Terminated) => true,
            
            // From Ringing
            (SessionState::Ringing, SessionState::Connected) => true,
            (SessionState::Ringing, SessionState::Terminating) => true,
            (SessionState::Ringing, SessionState::Terminated) => true,
            
            // From Connected
            (SessionState::Connected, SessionState::OnHold) => true,
            (SessionState::Connected, SessionState::Transferring) => true,
            (SessionState::Connected, SessionState::Terminating) => true,
            (SessionState::Connected, SessionState::Terminated) => true,
            
            // From OnHold
            (SessionState::OnHold, SessionState::Connected) => true,
            (SessionState::OnHold, SessionState::Transferring) => true,
            (SessionState::OnHold, SessionState::Terminating) => true,
            (SessionState::OnHold, SessionState::Terminated) => true,
            
            // From Transferring
            (SessionState::Transferring, SessionState::Connected) => true,
            (SessionState::Transferring, SessionState::OnHold) => true,
            (SessionState::Transferring, SessionState::Terminating) => true,
            (SessionState::Transferring, SessionState::Terminated) => true,
            
            // From Terminating
            (SessionState::Terminating, SessionState::Terminated) => true,
            
            // Any state can transition to itself (no-op)
            (a, b) if a == b => true,
            
            // Any other transition is invalid
            _ => false,
        }
    }
    
    /// Set the dialog for this session
    pub async fn set_dialog(&self, dialog: Dialog) -> Result<()> {
        let mut dlg = self.dialog.write().await;
        *dlg = Some(dialog);
        Ok(())
    }
    
    /// Get the current dialog
    pub async fn dialog(&self) -> Option<Dialog> {
        self.dialog.read().await.clone()
    }
    
    /// Add a related dialog
    pub async fn add_related_dialog(&self, dialog: Dialog) -> Result<()> {
        let mut dialogs = self.related_dialogs.write().await;
        dialogs.insert(dialog.id.clone(), dialog);
        Ok(())
    }
    
    /// Remove a related dialog
    pub async fn remove_related_dialog(&self, dialog_id: &DialogId) -> Result<()> {
        let mut dialogs = self.related_dialogs.write().await;
        dialogs.remove(dialog_id);
        Ok(())
    }
    
    /// Handle an incoming SIP request
    pub async fn handle_request(&self, request: Request) -> Result<Response> {
        match request.method {
            Method::Invite => self.handle_invite(request).await,
            Method::Ack => self.handle_ack(request).await,
            Method::Bye => self.handle_bye(request).await,
            Method::Cancel => self.handle_cancel(request).await,
            Method::Update => self.handle_update(request).await,
            Method::Info => self.handle_info(request).await,
            Method::Message => self.handle_message(request).await,
            Method::Refer => self.handle_refer(request).await,
            Method::Notify => self.handle_notify(request).await,
            Method::Options => self.handle_options(request).await,
            _ => Ok(Response::new(StatusCode::MethodNotAllowed)),
        }
    }
    
    /// Handle an incoming INVITE request
    async fn handle_invite(&self, request: Request) -> Result<Response> {
        // For now, just return a simple 200 OK
        let mut response = Response::new(StatusCode::Ok);
        
        // If this is a re-INVITE, need to handle it differently
        if self.state.read().await.clone() == SessionState::Connected {
            // Handle re-INVITE (media update, etc.)
            debug!("Received re-INVITE for session {}", self.id);
        } else {
            // New call
            debug!("Received new INVITE for session {}", self.id);
            self.set_state(SessionState::Ringing).await?;
        }
        
        // For a real implementation, would need to:
        // 1. Parse SDP to get media info
        // 2. Setup media streams
        // 3. Generate SDP answer
        // 4. Add SDP to response body
        
        Ok(response)
    }
    
    /// Handle an incoming ACK request
    async fn handle_ack(&self, request: Request) -> Result<Response> {
        // ACK doesn't have a response, but we can use it to finalize dialog setup
        debug!("Received ACK for session {}", self.id);
        
        if let Some(dialog) = self.dialog.read().await.clone() {
            if dialog.state == DialogState::Early {
                let mut dialog = dialog.clone();
                dialog.state = DialogState::Confirmed;
                self.set_dialog(dialog).await?;
            }
        }
        
        // Start media if not already started
        self.start_media().await?;
        
        // Set state to connected if not already
        let current_state = self.state.read().await.clone();
        if current_state == SessionState::Ringing || current_state == SessionState::Dialing {
            self.set_state(SessionState::Connected).await?;
        }
        
        // ACK doesn't have a response in SIP
        Ok(Response::new(StatusCode::Ok))
    }
    
    /// Handle an incoming BYE request
    async fn handle_bye(&self, request: Request) -> Result<Response> {
        debug!("Received BYE for session {}", self.id);
        
        // Set session state to terminating
        self.set_state(SessionState::Terminating).await?;
        
        // Stop media
        self.stop_media().await?;
        
        // Update dialog state
        if let Some(mut dialog) = self.dialog.read().await.clone() {
            dialog.terminate();
            self.set_dialog(dialog).await?;
        }
        
        // Set session state to terminated
        self.set_state(SessionState::Terminated).await?;
        
        // Return 200 OK
        Ok(Response::new(StatusCode::Ok))
    }
    
    /// Handle an incoming CANCEL request
    async fn handle_cancel(&self, _request: Request) -> Result<Response> {
        debug!("Received CANCEL for session {}", self.id);
        
        // Only process CANCEL if in early dialog state
        let current_state = self.state.read().await.clone();
        if current_state != SessionState::Ringing && current_state != SessionState::Dialing {
            return Ok(Response::new(StatusCode::BadRequest));
        }
        
        // Set session state to terminated
        self.set_state(SessionState::Terminated).await?;
        
        // Return 200 OK
        Ok(Response::new(StatusCode::Ok))
    }
    
    /// Handle an incoming UPDATE request
    async fn handle_update(&self, request: Request) -> Result<Response> {
        // For now, just accept the UPDATE
        debug!("Received UPDATE for session {}", self.id);
        Ok(Response::new(StatusCode::Ok))
    }
    
    /// Handle an incoming INFO request
    async fn handle_info(&self, request: Request) -> Result<Response> {
        debug!("Received INFO for session {}", self.id);
        
        // Process INFO request (e.g., DTMF)
        // For now, just accept it
        Ok(Response::new(StatusCode::Ok))
    }
    
    /// Handle an incoming MESSAGE request
    async fn handle_message(&self, request: Request) -> Result<Response> {
        debug!("Received MESSAGE for session {}", self.id);
        
        // Process MESSAGE request
        // For now, just accept it
        Ok(Response::new(StatusCode::Ok))
    }
    
    /// Handle an incoming REFER request
    async fn handle_refer(&self, request: Request) -> Result<Response> {
        debug!("Received REFER for session {}", self.id);
        
        // Process REFER request (call transfer)
        // For now, just accept it
        Ok(Response::new(StatusCode::Accepted))
    }
    
    /// Handle an incoming NOTIFY request
    async fn handle_notify(&self, request: Request) -> Result<Response> {
        debug!("Received NOTIFY for session {}", self.id);
        
        // Process NOTIFY request
        // For now, just accept it
        Ok(Response::new(StatusCode::Ok))
    }
    
    /// Handle an incoming OPTIONS request
    async fn handle_options(&self, request: Request) -> Result<Response> {
        debug!("Received OPTIONS for session {}", self.id);
        
        // Process OPTIONS request
        // For now, just accept it with supported methods
        let mut response = Response::new(StatusCode::Ok);
        
        // Add Allow header
        let allow_value = "INVITE, ACK, BYE, CANCEL, OPTIONS, UPDATE, INFO, MESSAGE, REFER, NOTIFY";
        response.headers.push(Header::text(HeaderName::Allow, allow_value));
        
        Ok(response)
    }
    
    /// Create and send an outgoing INVITE request
    pub async fn send_invite(&self, target_uri: Uri) -> Result<()> {
        debug!("Sending INVITE to {} for session {}", target_uri, self.id);
        
        // Create a string representation of the target URI for headers
        let target_uri_str = format!("{}", target_uri);
        
        // Create INVITE request
        let mut request = Request::new(Method::Invite, target_uri);
        
        // Add basic headers
        // In a real implementation, would need to:
        // 1. Add From header with local URI and tag
        // 2. Add To header with target URI
        // 3. Add Call-ID
        // 4. Add CSeq
        // 5. Add Contact
        // 6. Add Via
        // 7. Generate SDP for media offer
        // 8. Add SDP to request body
        
        // Add some basic headers for our simplified example
        let random_call_id = format!("{}@{}", Uuid::new_v4(), self.config.local_signaling_addr);
        request.headers.push(Header::text(HeaderName::CallId, random_call_id));
        
        let local_tag = Uuid::new_v4().to_string();
        let from_value = format!("<sip:user@{};tag={}>", self.config.local_signaling_addr, local_tag);
        request.headers.push(Header::text(HeaderName::From, from_value));
        
        let to_value = format!("<{}>", target_uri_str);
        request.headers.push(Header::text(HeaderName::To, to_value));
        
        let cseq_value = "1 INVITE";
        request.headers.push(Header::text(HeaderName::CSeq, cseq_value));
        
        // Send the request through the transaction layer
        // In a real implementation, we would extract the host/port from target_uri
        // For this example, assume it's a simple host:port format
        let destination = format!("{}:{}", 
            request.uri.host, 
            request.uri.port.unwrap_or(5060)
        ).parse()?;
        
        let tx_id = self.transaction_manager.create_client_transaction(
            request, 
            destination
        ).await?;
        
        // Send the request
        self.transaction_manager.send_request(&tx_id).await?;
        
        // Set state to dialing
        self.set_state(SessionState::Dialing).await?;
        
        // Wait for response in a separate task
        let session_id = self.id.clone();
        let event_bus = self.event_bus.clone();
        
        // For our simple example, we'll just simulate a successful call after a delay
        tokio::spawn(async move {
            // Simulate some network delay
            tokio::time::sleep(Duration::from_secs(1)).await;
            
            // Simulate successful call setup
            debug!("Call established for session {}", session_id);
            
            // Publish event
            event_bus.publish(SessionEvent::StateChanged {
                session_id: session_id.clone(),
                old_state: SessionState::Dialing,
                new_state: SessionState::Connected,
            });
        });
        
        Ok(())
    }
    
    /// Send a BYE request to terminate the session
    pub async fn send_bye(&self) -> Result<()> {
        debug!("Sending BYE for session {}", self.id);
        
        // Need an established dialog to send BYE
        let dialog = match self.dialog.read().await.clone() {
            Some(dialog) => dialog,
            None => return Err(Error::DialogNotFound(self.id.to_string()).into()),
        };
        
        if dialog.state != DialogState::Confirmed {
            return Err(Error::InvalidStateTransition(
                dialog.state.to_string(),
                "terminated".to_string(),
            ).into());
        }
        
        // Set state to terminating
        self.set_state(SessionState::Terminating).await?;
        
        // Create BYE request within dialog
        let mut dialog = dialog.clone();
        let request = dialog.create_request(Method::Bye);
        
        // Send the request through the transaction layer
        // In a real implementation, we would extract the host/port from request.uri
        // For this example, assume it's a simple host:port format
        let destination = format!("{}:{}", 
            request.uri.host, 
            request.uri.port.unwrap_or(5060)
        ).parse()?;
        
        let tx_id = self.transaction_manager.create_client_transaction(
            request, 
            destination
        ).await?;
        
        // Send the request
        self.transaction_manager.send_request(&tx_id).await?;
        
        // Stop media
        self.stop_media().await?;
        
        // Update dialog state
        dialog.terminate();
        self.set_dialog(dialog).await?;
        
        // Wait for response in a separate task
        let session_id = self.id.clone();
        let event_bus = self.event_bus.clone();
        
        // For our simple example, we'll just transition to the terminated state directly
        // because we can't directly access the TransactionEvent stream
        tokio::spawn(async move {
            // Allow some time for the BYE to be processed
            tokio::time::sleep(Duration::from_millis(500)).await;
            
            // BYE complete
            debug!("BYE complete for session {}", session_id);
            
            // Publish event
            event_bus.publish(SessionEvent::StateChanged {
                session_id: session_id.clone(),
                old_state: SessionState::Terminating,
                new_state: SessionState::Terminated,
            });
        });
        
        Ok(())
    }
    
    /// Start media for this session
    pub async fn start_media(&self) -> Result<()> {
        debug!("Starting media for session {}", self.id);
        
        // Check if media is already started
        if let Some(media) = self.media.read().await.as_ref() {
            if media.is_active().await {
                return Ok(());
            }
        }
        
        // In a real implementation, would:
        // 1. Get remote media info from SDP
        // 2. Configure local media stream
        // 3. Start RTP session
        
        // For now, create a dummy media config
        let media_config = MediaConfig {
            local_addr: self.config.local_media_addr,
            remote_addr: None, // Would come from SDP
            media_type: MediaType::Audio,
            payload_type: 0, // 0 = PCMU, 8 = PCMA
            clock_rate: 8000,
            audio_codec: AudioCodecType::PCMU,
        };
        
        // Create and start media stream
        let media_stream = MediaStream::new(media_config).await?;
        media_stream.start().await?;
        
        // Store the media stream
        let mut media = self.media.write().await;
        *media = Some(media_stream);
        
        // Publish media started event
        self.event_bus.publish(SessionEvent::MediaStarted {
            session_id: self.id.clone(),
        });
        
        Ok(())
    }
    
    /// Stop media for this session
    pub async fn stop_media(&self) -> Result<()> {
        debug!("Stopping media for session {}", self.id);
        
        // Stop media if active
        if let Some(media) = self.media.read().await.as_ref() {
            if media.is_active().await {
                media.stop().await?;
                
                // Publish media stopped event
                self.event_bus.publish(SessionEvent::MediaStopped {
                    session_id: self.id.clone(),
                });
            }
        }
        
        Ok(())
    }
    
    /// Check if the session is active
    pub async fn is_active(&self) -> bool {
        let state = self.state.read().await.clone();
        matches!(state, 
            SessionState::Dialing | 
            SessionState::Ringing | 
            SessionState::Connected | 
            SessionState::OnHold |
            SessionState::Transferring
        )
    }
    
    /// Check if the session is terminated
    pub async fn is_terminated(&self) -> bool {
        let state = self.state.read().await.clone();
        matches!(state, SessionState::Terminated)
    }
}

/// Manager for SIP sessions
pub struct SessionManager {
    /// Active sessions
    sessions: DashMap<SessionId, Arc<Session>>,
    
    /// Transaction manager
    transaction_manager: Arc<TransactionManager>,
    
    /// Session configuration
    config: SessionConfig,
    
    /// Event bus
    event_bus: EventBus,
}

impl SessionManager {
    /// Create a new session manager
    pub fn new(
        transaction_manager: Arc<TransactionManager>,
        config: SessionConfig,
        event_bus: EventBus,
    ) -> Self {
        Self {
            sessions: DashMap::new(),
            transaction_manager,
            config,
            event_bus,
        }
    }
    
    /// Create a new outgoing session
    pub async fn create_outgoing_session(&self) -> Result<Arc<Session>> {
        let session = Arc::new(Session::new(
            SessionDirection::Outgoing,
            self.config.clone(),
            self.transaction_manager.clone(),
            self.event_bus.clone(),
        ));
        
        self.sessions.insert(session.id.clone(), session.clone());
        Ok(session)
    }
    
    /// Create a new incoming session from an INVITE request
    pub async fn create_incoming_session(&self, _request: Request) -> Result<Arc<Session>> {
        let session = Arc::new(Session::new(
            SessionDirection::Incoming,
            self.config.clone(),
            self.transaction_manager.clone(),
            self.event_bus.clone(),
        ));
        
        // Set initial state to ringing
        session.set_state(SessionState::Ringing).await?;
        
        self.sessions.insert(session.id.clone(), session.clone());
        Ok(session)
    }
    
    /// Get a session by ID
    pub fn get_session(&self, id: &SessionId) -> Option<Arc<Session>> {
        self.sessions.get(id).map(|s| s.clone())
    }
    
    /// Get all active sessions
    pub fn list_sessions(&self) -> Vec<Arc<Session>> {
        self.sessions.iter().map(|s| s.clone()).collect()
    }
    
    /// Remove a terminated session
    pub fn remove_session(&self, id: &SessionId) -> bool {
        self.sessions.remove(id).is_some()
    }
    
    /// Terminate all sessions
    pub async fn terminate_all(&self) -> Result<()> {
        for session in self.list_sessions() {
            if session.is_active().await {
                if let Err(e) = session.send_bye().await {
                    warn!("Error terminating session {}: {}", session.id, e);
                }
            }
        }
        
        Ok(())
    }
    
    /// Clean up terminated sessions
    pub async fn cleanup_terminated(&self) -> usize {
        let mut terminated_ids = Vec::new();
        
        for session in self.list_sessions() {
            if session.is_terminated().await {
                terminated_ids.push(session.id.clone());
            }
        }
        
        for id in &terminated_ids {
            self.sessions.remove(id);
        }
        
        terminated_ids.len()
    }
} 