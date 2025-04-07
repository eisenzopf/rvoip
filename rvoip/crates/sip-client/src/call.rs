use std::sync::Arc;
use std::net::SocketAddr;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock, watch, Mutex};
use tokio::time::Instant;
use tracing::{debug, error, info, warn};
use uuid::Uuid;
use bytes::Bytes;
use futures::future::BoxFuture;
use futures::FutureExt;

use rvoip_sip_core::{
    Request, Response, Message, Method, StatusCode, Uri, 
    Header, HeaderName, HeaderValue
};
use rvoip_session_core::sdp::{SessionDescription, extract_rtp_port_from_sdp};
use rvoip_session_core::dialog::{Dialog, DialogState};
use rvoip_transaction_core::TransactionManager;

use crate::config::CallConfig;
use crate::error::{Error, Result};
use crate::media::{MediaSession, MediaType};

/// Direction of the call
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallDirection {
    /// Outgoing call (we initiated it)
    Outgoing,
    /// Incoming call (we received it)
    Incoming,
}

/// State of a call
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallState {
    /// Initial state
    Initial,
    /// Outgoing call: INVITE sent, waiting for response
    /// Incoming call: Ringing, 180 Ringing sent
    Ringing,
    /// Outgoing call: Received final response, waiting for ACK
    /// Incoming call: 200 OK sent, waiting for ACK
    Connecting,
    /// Call is established (media flowing)
    Established,
    /// Call is being terminated
    Terminating,
    /// Call is terminated
    Terminated,
}

impl std::fmt::Display for CallState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallState::Initial => write!(f, "Initial"),
            CallState::Ringing => write!(f, "Ringing"),
            CallState::Connecting => write!(f, "Connecting"),
            CallState::Established => write!(f, "Established"),
            CallState::Terminating => write!(f, "Terminating"),
            CallState::Terminated => write!(f, "Terminated"),
        }
    }
}

/// Call events
#[derive(Debug, Clone)]
pub enum CallEvent {
    /// Incoming call received
    IncomingCall(Arc<Call>),
    /// Call state changed
    StateChanged {
        /// Call instance
        call: Arc<Call>,
        /// Previous state
        previous: CallState,
        /// New state
        current: CallState,
    },
    /// Media added to call
    MediaAdded {
        /// Call instance
        call: Arc<Call>,
        /// Media type
        media_type: MediaType,
    },
    /// Media removed from call
    MediaRemoved {
        /// Call instance
        call: Arc<Call>,
        /// Media type
        media_type: MediaType,
    },
    /// DTMF digit received
    DtmfReceived {
        /// Call instance
        call: Arc<Call>,
        /// DTMF digit
        digit: char,
    },
    /// Call terminated
    Terminated {
        /// Call instance
        call: Arc<Call>,
        /// Reason for termination
        reason: String,
    },
    /// Error occurred
    Error {
        /// Call instance
        call: Arc<Call>,
        /// Error description
        error: String,
    },
}

/// Call information and control
#[derive(Debug)]
pub struct Call {
    /// Unique call ID
    id: String,

    /// Call direction
    direction: CallDirection,

    /// Call configuration
    config: CallConfig,

    /// SIP call ID
    sip_call_id: String,

    /// Local tag
    local_tag: String,

    /// Remote tag
    remote_tag: Arc<RwLock<Option<String>>>,

    /// CSeq counter
    cseq: Arc<Mutex<u32>>,

    /// Local URI
    local_uri: Uri,

    /// Remote URI
    remote_uri: Uri,

    /// Remote display name
    remote_display_name: Arc<RwLock<Option<String>>>,

    /// Remote address
    remote_addr: SocketAddr,

    /// SIP transaction manager
    transaction_manager: Arc<TransactionManager>,

    /// Current call state
    state: Arc<RwLock<CallState>>,

    /// State change watcher
    state_watcher: watch::Receiver<CallState>,

    /// Call start time
    start_time: Option<Instant>,

    /// Call connect time
    connect_time: Option<Instant>,

    /// Call end time
    end_time: Option<Instant>,

    /// Active media sessions
    media_sessions: Arc<RwLock<Vec<MediaSession>>>,

    /// Call event sender
    event_tx: mpsc::Sender<CallEvent>,

    /// Local SDP
    local_sdp: Arc<RwLock<Option<SessionDescription>>>,

    /// Remote SDP
    remote_sdp: Arc<RwLock<Option<SessionDescription>>>,

    /// Dialog
    dialog: Arc<RwLock<Option<Dialog>>>,
}

impl Call {
    /// Create a new call
    pub(crate) fn new(
        direction: CallDirection,
        config: CallConfig,
        sip_call_id: String,
        local_tag: String,
        local_uri: Uri,
        remote_uri: Uri,
        remote_addr: SocketAddr,
        transaction_manager: Arc<TransactionManager>,
        event_tx: mpsc::Sender<CallEvent>,
    ) -> (Arc<Self>, watch::Sender<CallState>) {
        let (state_tx, state_rx) = watch::channel(CallState::Initial);

        let call = Arc::new(Self {
            id: Uuid::new_v4().to_string(),
            direction,
            config,
            sip_call_id,
            local_tag,
            remote_tag: Arc::new(RwLock::new(None)),
            cseq: Arc::new(Mutex::new(1)),
            local_uri,
            remote_uri,
            remote_display_name: Arc::new(RwLock::new(None)),
            remote_addr,
            transaction_manager,
            state: Arc::new(RwLock::new(CallState::Initial)),
            state_watcher: state_rx,
            start_time: Some(Instant::now()),
            connect_time: None,
            end_time: None,
            media_sessions: Arc::new(RwLock::new(Vec::new())),
            event_tx,
            local_sdp: Arc::new(RwLock::new(None)),
            remote_sdp: Arc::new(RwLock::new(None)),
            dialog: Arc::new(RwLock::new(None)),
        });

        (call, state_tx)
    }

    /// Get the call ID
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Get the SIP call ID
    pub fn sip_call_id(&self) -> &str {
        &self.sip_call_id
    }

    /// Get the call direction
    pub fn direction(&self) -> CallDirection {
        self.direction
    }

    /// Get the current call state
    pub async fn state(&self) -> CallState {
        *self.state.read().await
    }

    /// Get the remote URI
    pub fn remote_uri(&self) -> &Uri {
        &self.remote_uri
    }

    /// Get the remote display name
    pub async fn remote_display_name(&self) -> Option<String> {
        self.remote_display_name.read().await.clone()
    }

    /// Get the caller ID (formatted with display name if available)
    pub async fn caller_id(&self) -> String {
        if let Some(name) = self.remote_display_name().await {
            format!("{} <{}>", name, self.remote_uri)
        } else {
            self.remote_uri.to_string()
        }
    }

    /// Get the call duration
    pub fn duration(&self) -> Option<Duration> {
        let start = self.connect_time?;
        let end = self.end_time.unwrap_or_else(Instant::now);
        Some(end.duration_since(start))
    }

    /// Get the active media sessions
    pub async fn media_sessions(&self) -> Vec<MediaSession> {
        self.media_sessions.read().await.clone()
    }

    /// Answer an incoming call
    pub async fn answer(&self) -> Result<()> {
        if self.direction != CallDirection::Incoming {
            return Err(Error::Call("Cannot answer an outgoing call".into()));
        }

        let current_state = self.state().await;
        if current_state != CallState::Ringing {
            return Err(Error::Call(
                format!("Cannot answer call in {} state", current_state)
            ));
        }

        // Implementation will be filled in later
        debug!("Answering call {} not implemented yet", self.id);

        Ok(())
    }

    /// Reject an incoming call
    pub async fn reject(&self, _status: StatusCode) -> Result<()> {
        if self.direction != CallDirection::Incoming {
            return Err(Error::Call("Cannot reject an outgoing call".into()));
        }

        let current_state = self.state().await;
        if current_state != CallState::Ringing && current_state != CallState::Initial {
            return Err(Error::Call(
                format!("Cannot reject call in {} state", current_state)
            ));
        }

        // Implementation will be filled in later
        debug!("Rejecting call {} not implemented yet", self.id);

        Ok(())
    }

    /// Hang up a call
    pub async fn hangup(&self) -> Result<()> {
        let current_state = self.state().await;
        if current_state == CallState::Terminated || current_state == CallState::Terminating {
            return Ok(());
        }

        // Update state to Terminating
        self.update_state(CallState::Terminating).await?;
        
        // Check if we have a dialog to use for creating the BYE
        let mut dialog_guard = self.dialog.write().await;
        
        if let Some(ref mut dialog) = *dialog_guard {
            // Create BYE request using dialog
            let mut bye_request = dialog.create_request(Method::Bye);
            
            // Add Content-Length if not present
            if !bye_request.headers.iter().any(|h| h.name == HeaderName::ContentLength) {
                bye_request.headers.push(Header::text(HeaderName::ContentLength, "0"));
            }
            
            // Mark dialog as terminated
            dialog.terminate();
            
            // Need to drop dialog_guard before making the transaction to avoid deadlocks
            drop(dialog_guard);
            
            // Send BYE via transaction
            let transaction_id = self.transaction_manager.create_client_non_invite_transaction(
                bye_request,
                self.remote_addr,
            ).await.map_err(|e| Error::Transport(e.to_string()))?;
            
            self.transaction_manager.send_request(&transaction_id).await
                .map_err(|e| Error::Transport(e.to_string()))?;
                        
            // We'll update to Terminated state when we receive a response to the BYE
            
            Ok(())
        } else {
            // No dialog available - call not properly established
            // Update state to Terminated
            drop(dialog_guard); // Release the lock before updating state
            self.update_state(CallState::Terminated).await?;
            
            Err(Error::Call("Cannot hang up: no dialog established".into()))
        }
    }

    /// Send DTMF digit
    pub async fn send_dtmf(&self, digit: char) -> Result<()> {
        let current_state = self.state().await;
        if current_state != CallState::Established {
            return Err(Error::Call(
                format!("Cannot send DTMF in {} state", current_state)
            ));
        }

        // Check if we have a dialog
        let mut dialog_guard = self.dialog.write().await;
        
        if let Some(ref mut dialog) = *dialog_guard {
            // Create INFO request using dialog
            let mut info_request = dialog.create_request(Method::Info);
            
            // Add Content-Type for DTMF
            info_request.headers.push(Header::text(
                HeaderName::ContentType, 
                "application/dtmf-relay"
            ));
            
            // Create DTMF payload according to RFC 2833
            let dtmf_body = format!("Signal={}\r\nDuration=250", digit);
            info_request.body = Bytes::from(dtmf_body);
            
            // Set Content-Length
            info_request.headers.push(Header::integer(
                HeaderName::ContentLength, 
                info_request.body.len() as i64
            ));
            
            // Drop the dialog lock before making the transaction call
            drop(dialog_guard);
            
            // Send INFO via transaction
            let transaction_id = self.transaction_manager.create_client_non_invite_transaction(
                info_request,
                self.remote_addr,
            ).await.map_err(|e| Error::Transport(e.to_string()))?;
            
            self.transaction_manager.send_request(&transaction_id).await
                .map_err(|e| Error::Transport(e.to_string()))?;
            
            debug!("Sent DTMF {} for call {}", digit, self.id);
            
            Ok(())
        } else {
            // Release the lock
            drop(dialog_guard);
            Err(Error::Call("Cannot send DTMF: no dialog established".into()))
        }
    }

    /// Wait until the call is established or terminated
    pub async fn wait_until_established(&self) -> Result<()> {
        loop {
            let state = *self.state_watcher.borrow();
            match state {
                CallState::Established => return Ok(()),
                CallState::Terminated => {
                    return Err(Error::Call("Call terminated before being established".into()));
                }
                _ => {
                    // Wait for state change
                    let mut cloned_watcher = self.state_watcher.clone();
                    if cloned_watcher.changed().await.is_err() {
                        return Err(Error::Call("Call state watcher closed".into()));
                    }
                }
            }
        }
    }

    /// Wait until the call is terminated
    pub async fn wait_until_terminated(&self) -> Result<()> {
        loop {
            let state = *self.state_watcher.borrow();
            if state == CallState::Terminated {
                return Ok(());
            }

            // Wait for state change
            let mut cloned_watcher = self.state_watcher.clone();
            if cloned_watcher.changed().await.is_err() {
                return Err(Error::Call("Call state watcher closed".into()));
            }
        }
    }

    /// Handle an incoming request for this call
    pub(crate) async fn handle_request(&self, request: Request) -> Result<Option<Response>> {
        debug!("Handling incoming request {} for call {}", request.method, self.id);
        
        // Get current call state
        let current_state = self.state().await;
        
        match request.method {
            Method::Ack => {
                // ACK received for 200 OK, call is established
                if current_state == CallState::Connecting {
                    self.update_state(CallState::Established).await?;
                    debug!("Call state updated to Established after receiving ACK");
                }
                Ok(None) // No response needed for ACK
            },
            Method::Bye => {
                // Remote side wants to hang up
                debug!("BYE received, terminating call");
                
                // Update dialog state
                {
                    let mut dialog_write = self.dialog.write().await;
                    if let Some(ref mut dialog) = *dialog_write {
                        dialog.terminate();
                        debug!("Dialog terminated for call {}", self.id);
                    }
                } // Release the write lock before updating call state
                
                // Update call state
                self.update_state(CallState::Terminated).await?;
                
                // Create 200 OK response
                let mut response = Response::new(StatusCode::Ok);
                
                // Add required headers from request
                for header in &request.headers {
                    match header.name {
                        HeaderName::Via | HeaderName::From | HeaderName::To |
                        HeaderName::CallId | HeaderName::CSeq => {
                            response.headers.push(header.clone());
                        },
                        _ => {},
                    }
                }
                
                // Add Content-Length
                response.headers.push(Header::text(HeaderName::ContentLength, "0"));
                
                // Send termination event
                let _ = self.event_tx.send(CallEvent::Terminated {
                    call: Arc::new(self.clone()),
                    reason: "Remote side terminated the call".to_string(),
                }).await;
                
                Ok(Some(response))
            },
            Method::Info => {
                // Handle INFO requests (commonly used for DTMF)
                if current_state != CallState::Established {
                    // Only process INFO during established state
                    let mut response = Response::new(StatusCode::CallOrTransactionDoesNotExist);
                    
                    // Add required headers
                    for header in &request.headers {
                        match header.name {
                            HeaderName::Via | HeaderName::From | HeaderName::To |
                            HeaderName::CallId | HeaderName::CSeq => {
                                response.headers.push(header.clone());
                            },
                            _ => {},
                        }
                    }
                    
                    response.headers.push(Header::text(HeaderName::ContentLength, "0"));
                    return Ok(Some(response));
                }
                
                // Check for DTMF content
                if let Some(content_type) = request.headers.iter().find(|h| h.name == HeaderName::ContentType) {
                    if let Some(content_type_str) = content_type.value.as_text() {
                        if content_type_str.contains("application/dtmf") || 
                           content_type_str.contains("application/dtmf-relay") {
                            // Process DTMF info
                            if let Ok(dtmf_str) = std::str::from_utf8(&request.body) {
                                debug!("Received DTMF: {}", dtmf_str);
                                
                                // Extract the digit - very simple parsing
                                let digit = if dtmf_str.contains("Signal=") {
                                    dtmf_str.split("Signal=").nth(1)
                                        .and_then(|s| s.chars().next())
                                } else {
                                    dtmf_str.trim().chars().next()
                                };
                                
                                if let Some(digit) = digit {
                                    // Send DTMF event
                                    let _ = self.event_tx.send(CallEvent::DtmfReceived {
                                        call: Arc::new(self.clone()),
                                        digit,
                                    }).await;
                                }
                            }
                        }
                    }
                }
                
                // Create 200 OK response for INFO
                let mut response = Response::new(StatusCode::Ok);
                
                // Add required headers
                for header in &request.headers {
                    match header.name {
                        HeaderName::Via | HeaderName::From | HeaderName::To |
                        HeaderName::CallId | HeaderName::CSeq => {
                            response.headers.push(header.clone());
                        },
                        _ => {},
                    }
                }
                
                response.headers.push(Header::text(HeaderName::ContentLength, "0"));
                Ok(Some(response))
            },
            // Add other method handlers as needed
            _ => {
                // Default: Not implemented
                debug!("Unhandled request method: {}", request.method);
                
                // Create 501 Not Implemented response
                let mut response = Response::new(StatusCode::NotImplemented);
                
                // Add required headers
                for header in &request.headers {
                    match header.name {
                        HeaderName::Via | HeaderName::From | HeaderName::To |
                        HeaderName::CallId | HeaderName::CSeq => {
                            response.headers.push(header.clone());
                        },
                        _ => {},
                    }
                }
                
                response.headers.push(Header::text(HeaderName::ContentLength, "0"));
                Ok(Some(response))
            }
        }
    }

    /// Handle an incoming response for this call
    pub(crate) async fn handle_response(&self, response: Response) -> Result<()> {
        debug!("Handling incoming response {} for call {}", response.status, self.id);
        
        let current_state = self.state().await;
        
        // Handle based on response status and current state
        match response.status {
            StatusCode::Ringing | StatusCode::SessionProgress => {
                if current_state == CallState::Initial {
                    // Check if we should create an early dialog
                    if let Some(to_header) = response.headers.iter().find(|h| h.name == HeaderName::To) {
                        if let Some(to_text) = to_header.value.as_text() {
                            if to_text.contains(";tag=") {
                                // This response can create an early dialog (it has a to-tag)
                                debug!("Creating early dialog from {} response", response.status);
                                
                                // Create a mock request for dialog creation
                                // In a more complete implementation, we'd store the original request
                                let mut request = Request::new(Method::Invite, self.remote_uri.clone());
                                request.headers.push(Header::text(HeaderName::CallId, self.sip_call_id.clone()));
                                request.headers.push(Header::text(HeaderName::From, 
                                    format!("<{}>;tag={}", self.local_uri, self.local_tag)));
                                request.headers.push(Header::text(HeaderName::To, 
                                    format!("<{}>", self.remote_uri)));
                                
                                if let Some(dialog) = Dialog::from_provisional_response(&request, &response, true) {
                                    let mut dialog_lock = self.dialog.write().await;
                                    *dialog_lock = Some(dialog);
                                    debug!("Early dialog created for call {}", self.id);
                                }
                            }
                        }
                    }
                    
                    // Update state to Ringing
                    self.update_state(CallState::Ringing).await?;
                }
            },
            status if status.is_success() => {
                if current_state == CallState::Initial || current_state == CallState::Ringing {
                    // Extract remote tag from To header
                    if let Some(to_header) = response.headers.iter().find(|h| h.name == HeaderName::To) {
                        if let Some(to_text) = to_header.value.as_text() {
                            if let Some(tag_start) = to_text.find(";tag=") {
                                let tag = &to_text[tag_start + 5..];
                                debug!("Extracted remote tag: {}", tag);
                                
                                // Store the remote tag
                                let mut remote_tag = self.remote_tag.write().await;
                                *remote_tag = Some(tag.to_string());
                            }
                        }
                    }
                    
                    // Handle dialog creation/updating
                    let mut dialog_lock = self.dialog.write().await;
                    
                    if let Some(ref mut dialog) = *dialog_lock {
                        // We have an early dialog, update it
                        if dialog.state == DialogState::Early {
                            dialog.update_from_2xx(&response);
                            debug!("Updated early dialog to confirmed for call {}", self.id);
                        }
                    } else {
                        // Create a new dialog
                        // Create a mock request for dialog creation
                        let mut request = Request::new(Method::Invite, self.remote_uri.clone());
                        request.headers.push(Header::text(HeaderName::CallId, self.sip_call_id.clone()));
                        request.headers.push(Header::text(HeaderName::From, 
                            format!("<{}>;tag={}", self.local_uri, self.local_tag)));
                        request.headers.push(Header::text(HeaderName::To, 
                            format!("<{}>", self.remote_uri)));
                        
                        if let Some(dialog) = Dialog::from_2xx_response(&request, &response, true) {
                            *dialog_lock = Some(dialog);
                            debug!("Created confirmed dialog for call {}", self.id);
                        }
                    }
                    
                    drop(dialog_lock); // Release the lock
                    
                    // Extract remote SDP
                    if !response.body.is_empty() {
                        match std::str::from_utf8(&response.body)
                            .map_err(|_| Error::SipProtocol("Invalid UTF-8 in SDP".into()))
                            .and_then(|sdp_str| SessionDescription::parse(sdp_str)
                                .map_err(|e| Error::SipProtocol(format!("Invalid SDP: {}", e))))
                        {
                            Ok(sdp) => {
                                // Store the remote SDP
                                let mut remote_sdp = self.remote_sdp.write().await;
                                *remote_sdp = Some(sdp);
                                
                                // Extract remote RTP port
                                if let Some(port) = extract_rtp_port_from_sdp(&response.body) {
                                    debug!("Extracted remote RTP port: {}", port);
                                }
                            },
                            Err(e) => {
                                warn!("Failed to parse SDP in response: {}", e);
                            }
                        }
                    }
                    
                    // Update state to Connecting
                    self.update_state(CallState::Connecting).await?;
                    
                    // Send ACK for 2xx response
                    if let Err(e) = self.send_ack().await {
                        error!("Failed to send ACK: {}", e);
                    } else {
                        // We've sent the ACK successfully, move to Established state
                        self.update_state(CallState::Established).await?;
                    }
                }
            },
            _ => {
                // Other responses (failure, etc.)
                if current_state != CallState::Terminated {
                    self.update_state(CallState::Terminated).await?;
                    
                    // If we have a dialog, terminate it
                    let mut dialog_lock = self.dialog.write().await;
                    if let Some(ref mut dialog) = *dialog_lock {
                        dialog.terminate();
                        debug!("Dialog terminated for call {}", self.id);
                    }
                    
                    // Emit termination event with reason from response
                    let reason = format!("Call failed with status: {}", response.status);
                    self.event_tx.send(CallEvent::Terminated {
                        call: Arc::new(self.clone()),
                        reason,
                    }).await.map_err(|_| Error::Call("Failed to send termination event".into()))?;
                }
            }
        }
        
        Ok(())
    }

    /// Create a dummy call for testing
    pub(crate) fn dummy() -> Self {
        use std::str::FromStr;
        
        Self {
            id: "dummy-id".to_string(),
            direction: CallDirection::Incoming,
            config: CallConfig::default(),
            sip_call_id: "dummy-call-id@example.com".to_string(),
            local_tag: "dummy-tag".to_string(),
            remote_tag: Arc::new(RwLock::new(None)),
            cseq: Arc::new(Mutex::new(1)),
            local_uri: Uri::from_str("sip:dummy@example.com").unwrap(),
            remote_uri: Uri::from_str("sip:remote@example.com").unwrap(),
            remote_display_name: Arc::new(RwLock::new(Some("Dummy User".to_string()))),
            remote_addr: "127.0.0.1:5060".parse().unwrap(),
            transaction_manager: Arc::new(TransactionManager::dummy()),
            state: Arc::new(RwLock::new(CallState::Initial)),
            state_watcher: watch::channel(CallState::Initial).1,
            start_time: None,
            connect_time: None,
            end_time: None,
            media_sessions: Arc::new(RwLock::new(Vec::new())),
            event_tx: mpsc::channel(1).0,
            local_sdp: Arc::new(RwLock::new(None)),
            remote_sdp: Arc::new(RwLock::new(None)),
            dialog: Arc::new(RwLock::new(None)),
        }
    }

    /// Update the call state and emit state change event
    pub(crate) async fn update_state(&self, new_state: CallState) -> Result<()> {
        let previous = {
            let mut state = self.state.write().await;
            let previous = *state;
            *state = new_state;
            previous
        };
        
        // Only emit event if the state actually changed
        if previous != new_state {
            debug!("Call {} state changed: {} -> {}", self.id, previous, new_state);
            
            // Set timing information based on state transitions
            match new_state {
                CallState::Established => {
                    if self.connect_time.is_none() {
                        // In a real implementation we'd update the connect_time field
                        // Can't modify fields through immutable reference 
                        // This would need to be handled through interior mutability or refactoring
                        debug!("Call established at {}", Instant::now().elapsed().as_secs_f32());
                    }
                },
                CallState::Terminated => {
                    if self.end_time.is_none() {
                        // In a real implementation we'd update the end_time field
                        // Can't modify fields through immutable reference
                        // This would need to be handled through interior mutability or refactoring
                        debug!("Call terminated at {}", Instant::now().elapsed().as_secs_f32());
                    }
                },
                _ => {},
            }
            
            // Send state change event
            self.event_tx.send(CallEvent::StateChanged {
                call: Arc::new(self.clone()),
                previous,
                current: new_state,
            }).await.map_err(|_| Error::Call("Failed to send state change event".into()))?;
        }
        
        Ok(())
    }

    /// Send an ACK for a successful response
    pub(crate) async fn send_ack(&self) -> Result<()> {
        debug!("Sending ACK for call {}", self.id);
        
        // Check if we have a dialog to use for creating the ACK
        let mut request = {
            let mut dialog_write = self.dialog.write().await;
            if let Some(ref mut dialog) = *dialog_write {
                // Use dialog to create the ACK request
                debug!("Creating ACK using dialog");
                dialog.create_request(Method::Ack)
            } else {
                // Fall back to manual ACK creation
                debug!("Creating ACK manually (no dialog)");
                let mut ack = Request::new(Method::Ack, self.remote_uri.clone());
                
                // Add Via header with branch parameter
                let branch = format!("z9hG4bK-{}", Uuid::new_v4());
                let via_value = format!(
                    "SIP/2.0/UDP {};branch={}",
                    self.local_uri.host,
                    branch
                );
                ack.headers.push(Header::text(HeaderName::Via, via_value));
                
                // Add Max-Forwards
                ack.headers.push(Header::integer(HeaderName::MaxForwards, 70));
                
                // Add From header with tag
                let from_value = format!(
                    "<{}>;tag={}",
                    self.local_uri,
                    self.local_tag
                );
                ack.headers.push(Header::text(HeaderName::From, from_value));
                
                // Add To header with remote tag if available
                let remote_tag_value = self.remote_tag.read().await.clone();
                let to_value = if let Some(tag) = remote_tag_value {
                    format!("<{}>;tag={}", self.remote_uri, tag)
                } else {
                    format!("<{}>", self.remote_uri)
                };
                ack.headers.push(Header::text(HeaderName::To, to_value));
                
                // Add Call-ID
                ack.headers.push(Header::text(HeaderName::CallId, self.sip_call_id.clone()));
                
                // Add CSeq - use the current CSeq with ACK method
                let cseq = *self.cseq.lock().await;
                ack.headers.push(Header::text(
                    HeaderName::CSeq,
                    format!("{} {}", cseq, Method::Ack)
                ));
                
                ack
            }
        };
        
        // Add Content-Length if not present
        if !request.headers.iter().any(|h| h.name == HeaderName::ContentLength) {
            request.headers.push(Header::text(HeaderName::ContentLength, "0"));
        }
        
        // Send directly via transport (ACK is end-to-end, not transaction-based)
        self.transaction_manager.transport().send_message(
            Message::Request(request),
            self.remote_addr
        ).await.map_err(|e| Error::Transport(e.to_string()))?;
        
        debug!("ACK sent for call {}", self.id);
        
        Ok(())
    }
}

impl Clone for Call {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            direction: self.direction,
            config: self.config.clone(),
            sip_call_id: self.sip_call_id.clone(),
            local_tag: self.local_tag.clone(),
            remote_tag: self.remote_tag.clone(),
            cseq: self.cseq.clone(),
            local_uri: self.local_uri.clone(),
            remote_uri: self.remote_uri.clone(),
            remote_display_name: self.remote_display_name.clone(),
            remote_addr: self.remote_addr,
            transaction_manager: self.transaction_manager.clone(),
            state: self.state.clone(),
            state_watcher: self.state_watcher.clone(),
            start_time: self.start_time,
            connect_time: self.connect_time,
            end_time: self.end_time,
            media_sessions: self.media_sessions.clone(),
            event_tx: self.event_tx.clone(),
            local_sdp: self.local_sdp.clone(),
            remote_sdp: self.remote_sdp.clone(),
            dialog: self.dialog.clone(),
        }
    }
} 