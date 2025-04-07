use std::sync::Arc;
use std::sync::Weak;
use std::net::SocketAddr;
use std::time::Duration;
use std::str::FromStr;
use std::collections::HashMap;
use std::net::IpAddr;

use tokio::sync::{mpsc, RwLock, watch, Mutex};
use tokio::time::Instant;
use tracing::{debug, error, info, warn};
use uuid::Uuid;
use bytes::Bytes;
use serde::{Serialize, Deserialize};

use rvoip_sip_core::{
    Request, Response, Message, Method, StatusCode, Uri, 
    Header, HeaderName, HeaderValue
};
use rvoip_session_core::sdp::{SessionDescription, extract_rtp_port_from_sdp};
use rvoip_session_core::dialog::{Dialog, DialogState, DialogId, extract_tag, extract_uri};
use rvoip_transaction_core::TransactionManager;

use crate::config::CallConfig;
use crate::error::{Error, Result};
use crate::media::{MediaSession, MediaType};

/// Direction of the call
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallDirection {
    /// Outgoing call (we initiated it)
    Outgoing,
    /// Incoming call (we received it)
    Incoming,
}

/// State of a call
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    /// Call has failed (special terminated state with error)
    Failed,
}

/// Error type for invalid state transitions
#[derive(Debug, Clone)]
pub struct StateChangeError {
    current: CallState,
    requested: CallState,
    message: String,
}

impl std::fmt::Display for StateChangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Invalid state transition from {} to {}: {}", 
            self.current, self.requested, self.message)
    }
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
            CallState::Failed => write!(f, "Failed"),
        }
    }
}

/// Call events
#[derive(Debug, Clone)]
pub enum CallEvent {
    /// Event system is ready
    Ready,
    
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
    
    /// Response received for a call
    ResponseReceived {
        /// Call instance
        call: Arc<Call>,
        /// Response received
        response: Response,
        /// Transaction ID
        transaction_id: String,
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

    /// State change watcher (receiver)
    state_watcher: watch::Receiver<CallState>,
    
    /// State change sender
    state_sender: Arc<watch::Sender<CallState>>,

    /// Call start time
    start_time: Option<Instant>,

    /// Call connect time
    connect_time: Arc<RwLock<Option<Instant>>>,

    /// Call end time
    end_time: Arc<RwLock<Option<Instant>>>,

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

    /// Last response received
    last_response: Arc<RwLock<Option<Response>>>,

    // Store the original INVITE request
    original_invite: Arc<RwLock<Option<Request>>>,
    
    // Store the transaction ID of the INVITE request
    invite_transaction_id: Arc<RwLock<Option<String>>>,
    
    // Call registry reference
    registry: Arc<RwLock<Option<Arc<dyn CallRegistryInterface + Send + Sync>>>>,
}

/// Interface for call registry methods needed by Call
#[async_trait::async_trait]
pub trait CallRegistryInterface {
    /// Log a transaction
    async fn log_transaction(&self, call_id: &str, transaction: crate::call_registry::TransactionRecord) -> Result<()>;
    
    /// Get transactions for a call
    async fn get_transactions(&self, call_id: &str) -> Result<Vec<crate::call_registry::TransactionRecord>>;
    
    /// Update transaction status
    async fn update_transaction_status(&self, call_id: &str, transaction_id: &str, status: &str, info: Option<String>) -> Result<()>;
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
            id: sip_call_id.clone(),
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
            state_sender: Arc::new(state_tx.clone()),
            start_time: Some(Instant::now()),
            connect_time: Arc::new(RwLock::new(None)),
            end_time: Arc::new(RwLock::new(None)),
            media_sessions: Arc::new(RwLock::new(Vec::new())),
            event_tx,
            local_sdp: Arc::new(RwLock::new(None)),
            remote_sdp: Arc::new(RwLock::new(None)),
            dialog: Arc::new(RwLock::new(None)),
            last_response: Arc::new(RwLock::new(None)),
            original_invite: Arc::new(RwLock::new(None)),
            invite_transaction_id: Arc::new(RwLock::new(None)),
            registry: Arc::new(RwLock::new(None)),
        });

        // Initialize local SDP based on call direction and configuration
        let call_clone = call.clone();
        tokio::spawn(async move {
            if let Err(e) = call_clone.setup_local_sdp().await {
                tracing::error!("Failed to set up local SDP: {}", e);
            }
        });

        (call, state_tx)
    }

    /// Setup local SDP for use in INVITE or answer
    pub(crate) async fn setup_local_sdp(&self) -> Result<()> {
        if self.local_sdp.read().await.is_some() {
            // Local SDP already initialized
            return Ok(());
        }

        tracing::debug!("Setting up local SDP for call {}", self.id);

        // Parse local address
        let local_ip = if let Ok(ip) = self.local_uri.host.parse::<IpAddr>() {
            ip
        } else {
            // Default to localhost if can't parse
            IpAddr::from([127, 0, 0, 1])
        };

        tracing::debug!("Local IP for SDP: {}", local_ip);

        // Create local SDP - use a specific port range based on config
        let rtp_port = if self.direction == CallDirection::Outgoing {
            10000 // Fixed port for caller
        } else {
            10002 // Fixed port for receiver
        };

        // Create the SDP
        let local_sdp = rvoip_session_core::sdp::SessionDescription::new_audio_call(
            &self.local_uri.user.clone().unwrap_or_else(|| "anonymous".to_string()),
            local_ip,
            rtp_port,
        );

        // Log the SDP for debugging
        let sdp_str = local_sdp.to_string();
        tracing::debug!("Created local SDP for call {}:\n{}", self.id, sdp_str);

        // Store the SDP
        {
            let mut local_sdp_guard = self.local_sdp.write().await;
            *local_sdp_guard = Some(local_sdp);
        }

        Ok(())
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
    pub async fn duration(&self) -> Option<Duration> {
        let connect_guard = self.connect_time.read().await;
        let connect = connect_guard.clone()?;
        
        let end = if let Some(end_time) = self.end_time.read().await.clone() {
            end_time
        } else {
            Instant::now()
        };
        
        Some(end.duration_since(connect))
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

        info!("Answering incoming call {}", self.id);
        
        // Ensure local SDP is initialized
        self.setup_local_sdp().await?;
        
        // Get the local SDP
        let local_sdp = match self.local_sdp.read().await.clone() {
            Some(sdp) => sdp,
            None => {
                error!("No local SDP available when answering call {}", self.id);
                return Err(Error::Call("No local SDP available for answer".into()));
            }
        };
        
        debug!("Using local SDP for answer:\n{}", local_sdp.to_string());
        
        // Create 200 OK response with SDP
        let mut response = rvoip_sip_core::Response::new(rvoip_sip_core::StatusCode::Ok);
        
        // Get the original INVITE request
        let original_request = {
            let invite = self.original_invite.read().await;
            if let Some(req) = invite.as_ref() {
                req.clone()
            } else {
                return Err(Error::Call("No INVITE request to answer".into()));
            }
        };
        
        // Add headers from To, From, Call-ID, CSeq
        for header in &original_request.headers {
            match header.name {
                rvoip_sip_core::HeaderName::Via | 
                rvoip_sip_core::HeaderName::From |
                rvoip_sip_core::HeaderName::CallId | 
                rvoip_sip_core::HeaderName::CSeq => {
                    response.headers.push(header.clone());
                },
                _ => {},
            }
        }
        
        // Add To header with tag
        let to_value = format!("<{}>;tag={}", self.local_uri, self.local_tag);
        response.headers.push(rvoip_sip_core::Header::text(
            rvoip_sip_core::HeaderName::To, 
            to_value
        ));
        
        // Add Contact header
        let contact = format!("<sip:{}@{};transport=udp>", 
                            self.local_uri.user.clone().unwrap_or_default(), 
                            self.local_uri.host);
        response.headers.push(rvoip_sip_core::Header::text(
            rvoip_sip_core::HeaderName::Contact,
            contact
        ));
        
        // Add Content-Type for SDP
        response.headers.push(rvoip_sip_core::Header::text(
            rvoip_sip_core::HeaderName::ContentType,
            "application/sdp"
        ));
        
        // Add SDP body
        let sdp_string = local_sdp.to_string();
        response.body = bytes::Bytes::from(sdp_string);
        
        // Add Content-Length
        response.headers.push(rvoip_sip_core::Header::integer(
            rvoip_sip_core::HeaderName::ContentLength,
            response.body.len() as i64
        ));
        
        // Change state to Connecting
        self.update_state(CallState::Connecting).await?;
        
        // Send 200 OK
        debug!("Sending 200 OK response to INVITE");
        self.transaction_manager.transport().send_message(
            rvoip_sip_core::Message::Response(response),
            self.remote_addr
        ).await.map_err(|e| Error::Transport(e.to_string()))?;
        
        // Set connect time
        {
            let mut connect_time = self.connect_time.write().await;
            *connect_time = Some(Instant::now());
        }
        
        // Create dialog
        {
            let remote_tag = self.remote_tag.read().await.clone()
                .ok_or_else(|| Error::SipProtocol("Missing remote tag".into()))?;
            
            // Create a dummy request and response for Dialog construction
            let mut dummy_request = rvoip_sip_core::Request::new(
                rvoip_sip_core::Method::Invite,
                self.remote_uri.clone()
            );
            dummy_request.headers.push(rvoip_sip_core::Header::text(
                rvoip_sip_core::HeaderName::CallId,
                self.sip_call_id.clone()
            ));
            dummy_request.headers.push(rvoip_sip_core::Header::text(
                rvoip_sip_core::HeaderName::From,
                format!("<{}>;tag={}", self.remote_uri, remote_tag)
            ));
            dummy_request.headers.push(rvoip_sip_core::Header::text(
                rvoip_sip_core::HeaderName::To,
                format!("<{}>", self.local_uri)
            ));
            
            let mut dummy_response = rvoip_sip_core::Response::new(
                rvoip_sip_core::StatusCode::Ok
            );
            dummy_response.headers.push(rvoip_sip_core::Header::text(
                rvoip_sip_core::HeaderName::CallId,
                self.sip_call_id.clone()
            ));
            dummy_response.headers.push(rvoip_sip_core::Header::text(
                rvoip_sip_core::HeaderName::From,
                format!("<{}>;tag={}", self.remote_uri, remote_tag)
            ));
            dummy_response.headers.push(rvoip_sip_core::Header::text(
                rvoip_sip_core::HeaderName::To,
                format!("<{}>;tag={}", self.local_uri, self.local_tag)
            ));
            
            // Use from_2xx_response to create the dialog
            if let Some(dialog) = rvoip_session_core::dialog::Dialog::from_2xx_response(
                &dummy_request, &dummy_response, false
            ) {
                let mut dialog_guard = self.dialog.write().await;
                *dialog_guard = Some(dialog);
            }
        }
        
        // The call will move to Established when we receive the ACK
        info!("Call {} answered, waiting for ACK", self.id);
        
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
            let transaction_id = self.transaction_manager.create_client_transaction(
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

        debug!("Preparing to send DTMF {} for call {}", digit, self.id);
        
        // Create a DTMF payload according to RFC 2833
        let dtmf_body = format!("Signal={}\r\nDuration=250", digit);
        let dtmf_body_bytes = Bytes::from(dtmf_body);
        let body_len = dtmf_body_bytes.len() as i64;

        // Try using the dialog if available
        let request = {
            let dialog_opt = self.dialog.read().await.clone();
            if let Some(mut dialog) = dialog_opt {
                debug!("Creating INFO request using dialog for call {}", self.id);
                let mut req = dialog.create_request(Method::Info);
                
                // Add Content-Type for DTMF
                req.headers.push(Header::text(HeaderName::ContentType, "application/dtmf-relay"));
                
                // Set body and Content-Length
                req.body = dtmf_body_bytes;
                req.headers.push(Header::integer(HeaderName::ContentLength, body_len));
                
                req
            } else {
                debug!("No dialog available, creating INFO request manually for call {}", self.id);
                
                // Create a new request - we'll let the TransactionManager handle required headers
                let mut req = Request::new(Method::Info, self.remote_uri.clone());
                
                // Add the headers that will be needed but won't be added automatically by transaction layer
                
                // From header with tag
                let from_value = format!("<{}>;tag={}", self.local_uri, self.local_tag);
                req.headers.push(Header::text(HeaderName::From, from_value));
                
                // To header with remote tag if available
                let remote_tag_value = self.remote_tag.read().await.clone();
                let to_value = if let Some(tag) = remote_tag_value {
                    format!("<{}>;tag={}", self.remote_uri, tag)
                } else {
                    format!("<{}>", self.remote_uri)
                };
                req.headers.push(Header::text(HeaderName::To, to_value));
                
                // Call-ID
                req.headers.push(Header::text(HeaderName::CallId, self.sip_call_id.clone()));
                
                // CSeq - use the next CSeq with INFO method
                let mut cseq = self.cseq.lock().await;
                let current_cseq = *cseq;
                *cseq += 1;
                req.headers.push(Header::text(
                    HeaderName::CSeq,
                    format!("{} {}", current_cseq, Method::Info)
                ));
                
                // Content-Type for DTMF
                req.headers.push(Header::text(HeaderName::ContentType, "application/dtmf-relay"));
                
                // Body and Content-Length
                req.body = dtmf_body_bytes;
                req.headers.push(Header::integer(HeaderName::ContentLength, body_len));
                
                req
            }
        };
        
        // Send the request via transaction manager, which will properly add Via, Max-Forwards, etc.
        debug!("Creating transaction for INFO request (DTMF)");
        let transaction_id = self.transaction_manager.create_client_transaction(
            request,
            self.remote_addr
        ).await.map_err(|e| {
            error!("Failed to create transaction for DTMF: {}", e);
            Error::Transport(e.to_string())
        })?;
        
        debug!("Sending INFO request via transaction {}", transaction_id);
        match self.transaction_manager.send_request(&transaction_id).await {
            Ok(_) => {
                debug!("Successfully sent DTMF {} for call {}", digit, self.id);
                Ok(())
            },
            Err(e) => {
                error!("Failed to send DTMF request: {}", e);
                Err(Error::Transport(e.to_string()))
            }
        }
    }

    /// Wait until the call is established or terminated
    pub async fn wait_until_established(&self) -> Result<()> {
        // Check the actual call state directly, not the watcher
        let current_state = self.state().await;
        debug!("Initial call state in wait_until_established: {}", current_state);
        
        if current_state == CallState::Established {
            debug!("Call is already established, returning immediately");
            return Ok(());
        }
        
        if current_state == CallState::Terminated || current_state == CallState::Failed {
            debug!("Call is in terminal state {}, cannot establish", current_state);
            return Err(Error::Call("Call terminated before being established".into()));
        }
        
        // If we're here, the call is not yet established or terminated
        // Wait for state changes via polling with timeout
        debug!("Waiting for call to establish...");
        
        // Use a longer timeout (30 seconds is common for SIP call setup)
        let start = std::time::Instant::now();
        let timeout_duration = std::time::Duration::from_secs(30);
        
        loop {
            // Check if we've exceeded the timeout
            if start.elapsed() > timeout_duration {
                debug!("Timed out waiting for call to establish");
                return Err(Error::Timeout("Timed out waiting for call to establish".into()));
            }
            
            // Sleep a bit between checks
            tokio::time::sleep(Duration::from_millis(100)).await;
            
            // Check state again
            let state = self.state().await;
            debug!("Current call state while waiting: {}", state);
            
            match state {
                CallState::Established => {
                    debug!("Call established successfully after waiting");
                    return Ok(());
                },
                CallState::Terminated | CallState::Failed => {
                    debug!("Call terminated while waiting for establishment");
                    return Err(Error::Call("Call terminated before being established".into()));
                },
                _ => {
                    // Continue waiting
                    continue;
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
                    debug!("ACK received for call {} in Connecting state", self.id);
                    
                    // Update dialog state to confirmed if there's a dialog
                    let needs_dialog_update = {
                        let dialog_guard = self.dialog.read().await;
                        
                        match dialog_guard.as_ref() {
                            Some(dialog) if dialog.state != DialogState::Confirmed => {
                                debug!("Updating dialog state to Confirmed for ACK");
                                true
                            },
                            Some(_) => {
                                // Dialog exists but already confirmed
                                debug!("Dialog already in Confirmed state");
                                false
                            },
                            None => {
                                // If there's no dialog yet, we need to create one
                                debug!("No dialog found for ACK, creating one");
                                true
                            }
                        }
                    }; // End of read lock scope
                    
                    // If we need to update or create a dialog, do it now
                    if needs_dialog_update {
                        let remote_tag = match extract_tag(request.from().unwrap_or_default()) {
                            Some(tag) => tag,
                            None => format!("remote-tag-{}", Uuid::new_v4())
                        };
                        
                        let local_tag = self.local_tag.clone();
                        let call_id = self.sip_call_id.clone();
                        
                        // Create or update dialog in confirmed state
                        let mut dialog = Dialog {
                            id: DialogId::new(),
                            state: DialogState::Confirmed, 
                            call_id,
                            local_uri: self.local_uri.clone(),
                            remote_uri: self.remote_uri.clone(),
                            local_tag: Some(local_tag),
                            remote_tag: Some(remote_tag),
                            local_seq: 0,
                            remote_seq: 0,
                            remote_target: self.remote_uri.clone(), 
                            route_set: vec![],
                            is_initiator: self.direction == CallDirection::Outgoing,
                        };
                        
                        // Store the dialog
                        {
                            let mut dialog_write = self.dialog.write().await;
                            *dialog_write = Some(dialog);
                        }
                        
                        debug!("Dialog created/updated for call {}", self.id);
                    }
                    
                    // Update call state to established
                    self.transition_to(CallState::Established).await?;
                    debug!("Call state updated to Established after receiving ACK");
                    
                    // Update call connect time
                    {
                        let mut connect_time = self.connect_time.write().await;
                        if connect_time.is_none() {
                            *connect_time = Some(Instant::now());
                            debug!("Call connect time set for call {}", self.id);
                        }
                    }
                } else {
                    debug!("ACK received for call {} in unexpected state {}", self.id, current_state);
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
                return Ok(Some(response));
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
                return Ok(Some(response));
            }
        }
    }

    /// Handle an incoming response for this call
    pub(crate) async fn handle_response(&self, response: Response) -> Result<()> {
        use tracing::{info, debug, error};
        
        let status = response.status;
        debug!("Call {} handling response: {}", self.id, status);
        
        // Store the response
        {
            let mut last_response = self.last_response.write().await;
            *last_response = Some(response.clone());
        }
        
        // Get current state
        let current_state = self.state().await;
        
        // Extract the call ID from the response
        let call_id = response.call_id()
            .ok_or_else(|| Error::SipProtocol("Response missing Call-ID".into()))?;
        
        // Verify this is for our call
        if call_id != self.sip_call_id {
            return Err(Error::Call(format!(
                "Response has different Call-ID: {} vs {}", 
                call_id, self.sip_call_id
            )));
        }
        
        // Log the response details
        info!("Call {} received {} response in state {}", self.id, status, current_state);
        
        // Update dialog if final response
        if !status.is_provisional() {
            // Extract remote tag if it exists
            if let Some(to) = response.to() {
                if let Some(tag_pos) = to.find("tag=") {
                    let tag_start = tag_pos + 4;
                    let tag_end = to[tag_start..].find(';')
                        .map(|pos| tag_start + pos)
                        .unwrap_or(to.len());
                    let tag = &to[tag_start..tag_end];
                    
                    // Store remote tag
                    *self.remote_tag.write().await = Some(tag.to_string());
                    
                    // Create a fake request to use in dialog creation based on what we know
                    if status.is_success() && current_state != CallState::Established {
                        // Try to create a dialog from 2xx response
                        let invite_req = self.create_invite_request().await?;
                        
                        debug!("Created fake INVITE request to build dialog");
                        
                        // Create dialog from 2xx response
                        if let Some(dialog) = Dialog::from_2xx_response(&invite_req, &response, true) {
                            debug!("Created dialog from 2xx response");
                            *self.dialog.write().await = Some(dialog);
                        } else {
                            warn!("Failed to create dialog from 2xx response");
                        }
                    }
                }
            }
        }
        
        // Handle response based on call state
        if self.direction == CallDirection::Outgoing {
            // This is our outgoing call
            match (current_state, response.status.as_u16()) {
                // 100 Trying - ignore, we're already in a calling state
                (CallState::Ringing, 100) => {
                    debug!("Received 100 Trying for outgoing call in ringing state");
                },
                
                // 180 Ringing - update state if necessary
                (CallState::Initial, 180) | (CallState::Ringing, 180) => {
                    debug!("Received 180 Ringing for outgoing call");
                    if current_state != CallState::Ringing {
                        self.transition_to(CallState::Ringing).await?;
                    }
                },
                
                // 200 OK - call is accepted
                (CallState::Initial, 200) | (CallState::Ringing, 200) => {
                    debug!("Received 200 OK for outgoing call, call is accepted");
                    
                    // Update call state to connecting
                    self.transition_to(CallState::Connecting).await?;
                    
                    // Extract remote tag
                    if let Some(to_header) = response.header(&HeaderName::To) {
                        if let Some(to_text) = to_header.value.as_text() {
                            if let Some(tag_start) = to_text.find("tag=") {
                                let tag_start = tag_start + 4; // "tag=" length
                                let tag_end = to_text[tag_start..]
                                    .find(|c: char| c == ';' || c.is_whitespace())
                                    .map(|pos| tag_start + pos)
                                    .unwrap_or(to_text.len());
                                let tag = to_text[tag_start..tag_end].to_string();
                                debug!("Extracted remote tag: {}", tag);
                                self.set_remote_tag(tag).await;
                            }
                        }
                    }
                    
                    // Process SDP for media setup
                    if !response.body.is_empty() {
                        if let Ok(body_str) = std::str::from_utf8(&response.body) {
                            if let Ok(sdp) = rvoip_session_core::sdp::SessionDescription::parse(body_str) {
                                debug!("Successfully parsed SDP from 200 OK response");
                                self.setup_media_from_sdp(&sdp).await?;
                            } else {
                                warn!("Failed to parse SDP from 200 OK response");
                            }
                        } else {
                            warn!("Failed to parse SDP from 200 OK response - invalid UTF-8");
                        }
                    }
                    
                    // Send ACK immediately
                    debug!("Immediately sending ACK after 200 OK");
                    match self.send_ack().await {
                        Ok(_) => {
                            debug!("ACK sent successfully, moving to Established state");
                            // Transition to established state after ACK is sent
                            self.transition_to(CallState::Established).await?;
                        },
                        Err(e) => {
                            error!("Failed to send ACK: {}", e);
                            self.transition_to(CallState::Failed).await?;
                            return Err(e);
                        }
                    }
                },
                
                // 4xx-6xx responses - call failed
                (_, status) if status >= 400 && status < 700 => {
                    warn!("Call {} failed with status {}", self.id, status);
                    self.transition_to(CallState::Failed).await?;
                },
                
                // Any other response in established state - possibly a re-INVITE response
                (CallState::Established, _) => {
                    debug!("Received response in established state, possibly re-INVITE response");
                    // Handle re-INVITE response (future implementation)
                },
                
                // Log unexpected responses
                (state, status) => {
                    debug!("Unexpected response status {} in state {}", status, state);
                }
            }
        } else {
            // This is an incoming call
            match (current_state, response.status.as_u16()) {
                // 100 Trying - ignore, we're already in a calling state
                (CallState::Ringing, 100) => {
                    debug!("Received 100 Trying for incoming call in ringing state");
                },
                
                // 180 Ringing - update state if necessary
                (CallState::Initial, 180) | (CallState::Ringing, 180) => {
                    debug!("Received 180 Ringing for incoming call");
                    if current_state != CallState::Ringing {
                        self.transition_to(CallState::Ringing).await?;
                    }
                },
                
                // 200 OK - call is accepted
                (CallState::Initial, 200) | (CallState::Ringing, 200) => {
                    debug!("Received 200 OK for incoming call, call is accepted");
                    
                    // Update call state to connecting
                    self.transition_to(CallState::Connecting).await?;
                    
                    // Extract remote tag
                    if let Some(to_header) = response.header(&HeaderName::To) {
                        if let Some(to_text) = to_header.value.as_text() {
                            if let Some(tag_start) = to_text.find("tag=") {
                                let tag_start = tag_start + 4; // "tag=" length
                                let tag_end = to_text[tag_start..]
                                    .find(|c: char| c == ';' || c.is_whitespace())
                                    .map(|pos| tag_start + pos)
                                    .unwrap_or(to_text.len());
                                let tag = to_text[tag_start..tag_end].to_string();
                                debug!("Extracted remote tag: {}", tag);
                                self.set_remote_tag(tag).await;
                            }
                        }
                    }
                    
                    // Process SDP for media setup
                    if !response.body.is_empty() {
                        if let Ok(body_str) = std::str::from_utf8(&response.body) {
                            if let Ok(sdp) = rvoip_session_core::sdp::SessionDescription::parse(body_str) {
                                debug!("Successfully parsed SDP from 200 OK response");
                                self.setup_media_from_sdp(&sdp).await?;
                            } else {
                                warn!("Failed to parse SDP from 200 OK response");
                            }
                        } else {
                            warn!("Failed to parse SDP from 200 OK response - invalid UTF-8");
                        }
                    }
                    
                    // Send ACK immediately
                    debug!("Immediately sending ACK after 200 OK");
                    match self.send_ack().await {
                        Ok(_) => {
                            debug!("ACK sent successfully, moving to Established state");
                            // Transition to established state after ACK is sent
                            self.transition_to(CallState::Established).await?;
                        },
                        Err(e) => {
                            error!("Failed to send ACK: {}", e);
                            self.transition_to(CallState::Failed).await?;
                            return Err(e);
                        }
                    }
                },
                
                // 4xx-6xx responses - call failed
                (_, status) if status >= 400 && status < 700 => {
                    warn!("Call {} failed with status {}", self.id, status);
                    self.transition_to(CallState::Failed).await?;
                },
                
                // Any other response in established state - possibly a re-INVITE response
                (CallState::Established, _) => {
                    debug!("Received response in established state, possibly re-INVITE response");
                    // Handle re-INVITE response (future implementation)
                },
                
                // Log unexpected responses
                (state, status) => {
                    debug!("Unexpected response status {} in state {}", status, state);
                }
            }
        }
        
        Ok(())
    }

    /// Create an INVITE request for outgoing call
    pub(crate) async fn create_invite_request(&self) -> Result<Request> {
        // Ensure local SDP is initialized
        self.setup_local_sdp().await?;

        // Create a new INVITE request to the remote URI
        let mut request = Request::new(Method::Invite, self.remote_uri.clone());
        
        // Add From header with tag
        let from_value = format!("<{}>;tag={}", self.local_uri, self.local_tag);
        request.headers.push(Header::text(HeaderName::From, from_value));
        
        // Add To header (no tag for initial INVITE)
        let to_value = format!("<{}>", self.remote_uri);
        request.headers.push(Header::text(HeaderName::To, to_value));
        
        // Add Call-ID header
        request.headers.push(Header::text(HeaderName::CallId, self.sip_call_id.clone()));
        
        // Add CSeq header
        let cseq = *self.cseq.lock().await;
        request.headers.push(Header::text(HeaderName::CSeq, format!("{} INVITE", cseq)));
        
        // Add Via header (placeholder - will be filled by transaction layer)
        request.headers.push(Header::text(
            HeaderName::Via,
            format!("SIP/2.0/UDP {};branch=z9hG4bK-{}", self.local_uri.host, Uuid::new_v4())
        ));
        
        // Add Max-Forwards header
        request.headers.push(Header::integer(HeaderName::MaxForwards, 70));
        
        // Add Contact header
        let username = self.local_uri.user.clone().unwrap_or_else(|| "anonymous".to_string());
        request.headers.push(Header::text(
            HeaderName::Contact,
            format!("<sip:{}@{}>", username, self.local_uri.host)
        ));
        
        // Add User-Agent header
        request.headers.push(Header::text(HeaderName::UserAgent, "RVOIP SIP Client"));
        
        // Add Content-Type and Content-Length for SDP
        if let Some(sdp) = self.local_sdp.read().await.as_ref() {
            let sdp_string = sdp.to_string();
            
            // Debug log the SDP
            tracing::debug!("Including SDP in INVITE request for call {}:\n{}", self.id, sdp_string);
            
            request.headers.push(Header::text(HeaderName::ContentType, "application/sdp"));
            request.headers.push(Header::integer(HeaderName::ContentLength, sdp_string.len() as i64));
            request.body = Bytes::from(sdp_string);
        } else {
            // No SDP, just add Content-Length: 0
            tracing::warn!("No local SDP available when creating INVITE request for call {}", self.id);
            request.headers.push(Header::integer(HeaderName::ContentLength, 0));
        }
        
        Ok(request)
    }

    /// Send an ACK for a successful response
    pub(crate) async fn send_ack(&self) -> Result<()> {
        debug!("Sending ACK for call {}", self.id);
        
        // Create the ACK request 
        let request = {
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
                ack.headers.push(Header::text(HeaderName::Via, format!(
                    "SIP/2.0/UDP {};branch={}",
                    self.local_uri.host,
                    branch
                )));
                
                // Add From header with tag
                let from_value = format!(
                    "<{}>;tag={}",
                    self.local_uri,
                    self.local_tag
                );
                ack.headers.push(Header::text(HeaderName::From, from_value));
                
                // Add To header with remote tag if available
                let to_tag = self.remote_tag.read().await.clone();
                let to_value = if let Some(tag) = to_tag {
                    format!("<{}>;tag={}", self.remote_uri, tag)
                } else {
                    format!("<{}>", self.remote_uri)
                };
                ack.headers.push(Header::text(HeaderName::To, to_value));
                
                // Add Call-ID header
                ack.headers.push(Header::text(HeaderName::CallId, self.sip_call_id.clone()));
                
                // Add CSeq header
                let cseq = *self.cseq.lock().await;
                ack.headers.push(Header::text(HeaderName::CSeq, format!("{} ACK", cseq)));
                
                // Add Max-Forwards header
                ack.headers.push(Header::integer(HeaderName::MaxForwards, 70));
                
                // Add Content-Length header
                ack.headers.push(Header::integer(HeaderName::ContentLength, 0));
                
                ack
            }
        };
        
        // Get the last response and check if dialog exists
        let response_opt = self.last_response.read().await.clone();
        let has_dialog = self.dialog.read().await.is_some();
        
        // Get stored transaction ID (if any)
        let invite_tx_id_opt = self.invite_transaction_id.read().await.clone();
        
        // Flag to track if we've sent the ACK
        let mut ack_sent = false;
        
        // Try sending ACK with the transaction ID from the response Via header
        if has_dialog && !ack_sent {
            if let Some(response) = response_opt.clone() {
                if response.status.is_success() {
                    // Try to extract transaction ID from Via header
                    if let Some(tx_id) = rvoip_transaction_core::utils::extract_transaction_id_from_response(&response) {
                        debug!("Using transaction ID from response Via header: {} for ACK", tx_id);
                        
                        // Try sending ACK via transaction manager
                        match self.transaction_manager.send_2xx_ack(&tx_id, &response).await {
                            Ok(_) => {
                                debug!("ACK sent successfully via transaction manager using response Via");
                                ack_sent = true;
                            },
                            Err(e) => {
                                warn!("[{}] Failed to send ACK for 2xx response using Via header: {}", tx_id, e);
                                // Continue with fallbacks
                            }
                        }
                    } else {
                        debug!("Could not extract transaction ID from response Via header");
                    }
                }
            }
        }
        
        // Try using stored INVITE transaction ID
        if !ack_sent && invite_tx_id_opt.is_some() {
            let tx_id = invite_tx_id_opt.unwrap();
            debug!("Using stored INVITE transaction ID: {} for ACK", tx_id);
            
            if let Some(response) = response_opt.clone() {
                if response.status.is_success() {
                    // Try sending ACK via transaction manager with stored ID
                    match self.transaction_manager.send_2xx_ack(&tx_id, &response).await {
                        Ok(_) => {
                            debug!("ACK sent successfully via transaction manager using stored ID");
                            ack_sent = true;
                        },
                        Err(e) => {
                            warn!("[{}] Failed to send ACK using stored transaction ID: {}", tx_id, e);
                            // Continue with fallbacks
                        }
                    }
                }
            }
        }
        
        // Fallback: Send directly via transport as a last resort
        if !ack_sent {
            debug!("Sending ACK directly via transport to {}", self.remote_addr);
            match self.transaction_manager.transport().send_message(
                Message::Request(request),
                self.remote_addr
            ).await {
                Ok(_) => {
                    debug!("ACK sent successfully via direct transport");
                    ack_sent = true;
                },
                Err(e) => {
                    error!("Failed to send ACK via direct transport: {}", e);
                    return Err(Error::Transport(format!("Failed to send ACK: {}", e)));
                }
            }
        }
        
        if ack_sent {
            debug!("ACK sent for call {}", self.id);
            Ok(())
        } else {
            let error_msg = "Failed to send ACK - all methods exhausted";
            error!("{}", error_msg);
            Err(Error::Transport(error_msg.to_string()))
        }
    }

    /// Get the call's most recent received response
    pub async fn last_response(&self) -> Option<Response> {
        // Return stored response if available
        self.last_response.read().await.clone()
    }

    /// Update the call state from any state to a new state
    pub async fn transition_to(&self, new_state: CallState) -> Result<()> {
        // Get current state before changing it
        let previous_state = self.state().await;
        
        // Check if this is a valid state transition
        if !Self::is_valid_transition(previous_state, new_state) {
            return Err(Error::Call(format!(
                "Invalid state transition from {} to {}",
                previous_state, new_state
            )));
        }
        
        debug!("Call {} state transition: {} -> {}", self.id, previous_state, new_state);
        
        // Update state
        {
            let mut state = self.state.write().await;
            *state = new_state;
        }
        
        // Notify state change via watcher
        if let Err(e) = self.state_sender.send(new_state) {
            debug!("Failed to update state watcher: {:?}", e);
            // Not a fatal error, continue
        } else {
            debug!("Updated state watcher to: {}", new_state);
        }
        
        // Send state change event
        let _ = self.event_tx.send(CallEvent::StateChanged {
            call: Arc::new(self.clone()),
            previous: previous_state,
            current: new_state,
        }).await;
        
        // Update dialog state based on call state (ensure they're in sync)
        if let Some(dialog) = self.dialog.read().await.as_ref() {
            let mut dialog_update_needed = false;
            let mut new_dialog_state = dialog.state.clone();
            
            match new_state {
                CallState::Connecting => {
                    if dialog.state != DialogState::Early {
                        new_dialog_state = DialogState::Early;
                        dialog_update_needed = true;
                    }
                },
                CallState::Established => {
                    if dialog.state != DialogState::Confirmed {
                        new_dialog_state = DialogState::Confirmed;
                        dialog_update_needed = true;
                    }
                },
                CallState::Terminating | CallState::Terminated | CallState::Failed => {
                    if dialog.state != DialogState::Terminated {
                        new_dialog_state = DialogState::Terminated;
                        dialog_update_needed = true;
                    }
                },
                _ => {
                    // Other states don't need dialog state updates
                }
            }
            
            // Update dialog state if needed
            if dialog_update_needed {
                debug!("Updating dialog state from {:?} to {:?} for call {}", 
                    dialog.state, new_dialog_state, self.id);
                
                let mut dialog_clone = dialog.clone();
                dialog_clone.state = new_dialog_state;
                
                let mut dialog_write = self.dialog.write().await;
                *dialog_write = Some(dialog_clone);
            }
        }
        
        Ok(())
    }

    /// Check if a state transition is valid
    fn is_valid_transition(from: CallState, to: CallState) -> bool {
        match (from, to) {
            // Self-transitions are always allowed
            (a, b) if a == b => true,
            
            // Valid transitions from Initial
            (CallState::Initial, CallState::Ringing) => true,
            (CallState::Initial, CallState::Connecting) => true,
            (CallState::Initial, CallState::Established) => true, // For fast setup scenarios
            
            // Valid transitions from Ringing
            (CallState::Ringing, CallState::Connecting) => true,
            (CallState::Ringing, CallState::Established) => true, // Allow direct to Established for efficiency
            (CallState::Ringing, CallState::Terminating) => true,
            (CallState::Ringing, CallState::Terminated) => true,
            (CallState::Ringing, CallState::Failed) => true,
            
            // Valid transitions from Connecting
            (CallState::Connecting, CallState::Established) => true,
            (CallState::Connecting, CallState::Terminating) => true,
            (CallState::Connecting, CallState::Terminated) => true,
            (CallState::Connecting, CallState::Failed) => true,
            
            // Valid transitions from Established
            (CallState::Established, CallState::Terminating) => true,
            (CallState::Established, CallState::Terminated) => true,
            (CallState::Established, CallState::Failed) => true,
            
            // Valid transitions from Terminating
            (CallState::Terminating, CallState::Terminated) => true,
            (CallState::Terminating, CallState::Failed) => true,
            
            // Any state can transition to Failed or Terminated (for error handling)
            (_, CallState::Failed) => true, 
            (_, CallState::Terminated) => true,
            
            // No valid transitions from Terminated
            (CallState::Terminated, _) => false,
            
            // Any other transition is invalid
            _ => false,
        }
    }

    /// Set up media session based on remote SDP
    async fn setup_media_internal(&self, remote_sdp: &rvoip_session_core::sdp::SessionDescription) -> Result<()> {
        // Implementation of setup_media that can be called from handle_response
        debug!("Setting up media session based on remote SDP");
        
        // Set up audio session if there's an audio stream
        let sdp_str = remote_sdp.to_string();
        let sdp_bytes = sdp_str.as_bytes();
        let rtp_port = rvoip_session_core::sdp::extract_rtp_port_from_sdp(sdp_bytes).unwrap_or(0);
        
        if rtp_port > 0 {
            debug!("Remote audio RTP port: {}", rtp_port);
            
            // In a real implementation, we would initialize the RTP media session here
            // For now, we just emit an event notifying about media
            let _ = self.event_tx.send(CallEvent::MediaAdded {
                call: Arc::new(self.clone()),
                media_type: MediaType::Audio,
            }).await;
        }
        
        Ok(())
    }

    /// Set the remote tag
    pub async fn set_remote_tag(&self, tag: String) {
        let mut remote_tag = self.remote_tag.write().await;
        *remote_tag = Some(tag);
    }

    /// Store the last response
    pub async fn store_last_response(&self, response: Response) {
        let mut last_response = self.last_response.write().await;
        *last_response = Some(response);
    }

    /// Set up media from SDP
    pub async fn setup_media_from_sdp(&self, sdp: &rvoip_session_core::sdp::SessionDescription) -> Result<()> {
        // Store remote SDP
        {
            let mut remote_sdp = self.remote_sdp.write().await;
            *remote_sdp = Some(sdp.clone());
        }
        
        // Call the internal setup method
        self.setup_media_internal(sdp).await
    }

    /// Get the current dialog for this call, if any
    pub async fn dialog(&self) -> Option<Dialog> {
        self.dialog.read().await.clone()
    }

    /// Update the call state (deprecated, use transition_to instead)
    pub async fn update_state(&self, new_state: CallState) -> Result<()> {
        // Simply forward to transition_to
        self.transition_to(new_state).await
    }

    /// Store the original INVITE request for later answering
    pub async fn store_invite_request(&self, request: Request) -> Result<()> {
        debug!("Storing original INVITE request for call {}", self.id);
        let mut invite = self.original_invite.write().await;
        *invite = Some(request);
        Ok(())
    }

    /// Set the transaction ID for the INVITE
    pub async fn store_invite_transaction_id(&self, transaction_id: String) -> Result<()> {
        debug!("Storing INVITE transaction ID: {} for call {}", transaction_id, self.id);
        let mut tx_id = self.invite_transaction_id.write().await;
        *tx_id = Some(transaction_id);
        Ok(())
    }

    /// Set the call registry
    pub fn set_registry(&self, registry: Arc<dyn CallRegistryInterface + Send + Sync>) {
        let mut registry_guard = self.registry.blocking_write();
        *registry_guard = Some(registry);
    }
    
    /// Get the call registry
    pub fn registry(&self) -> Option<Arc<dyn CallRegistryInterface + Send + Sync>> {
        self.registry.blocking_read().clone()
    }
}

impl Clone for Call {
    /// Standard clone that creates strong references - use sparingly
    /// For most cases, prefer using .weak_clone() instead
    fn clone(&self) -> Self {
        debug!("Creating a strong clone of Call {}", self.id);
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
            state_sender: self.state_sender.clone(),
            start_time: self.start_time,
            connect_time: self.connect_time.clone(),
            end_time: self.end_time.clone(),
            media_sessions: self.media_sessions.clone(),
            event_tx: self.event_tx.clone(),
            local_sdp: self.local_sdp.clone(),
            remote_sdp: self.remote_sdp.clone(),
            dialog: self.dialog.clone(),
            last_response: self.last_response.clone(),
            original_invite: self.original_invite.clone(),
            invite_transaction_id: self.invite_transaction_id.clone(),
            registry: self.registry.clone(),
        }
    }
}

impl Call {
    /// Create a weak clone that won't contribute to over-pinning
    /// Use this for temporary references or when passing to systems that don't need ownership
    pub fn weak_clone(&self) -> WeakCall {
        WeakCall {
            id: self.id.clone(),
            direction: self.direction,
            sip_call_id: self.sip_call_id.clone(),
            local_uri: self.local_uri.clone(),
            remote_uri: self.remote_uri.clone(),
            remote_addr: self.remote_addr,
            state_watcher: self.state_watcher.clone(),
            // Store weak references to other fields
            remote_tag: Arc::downgrade(&self.remote_tag),
            state: Arc::downgrade(&self.state),
            connect_time: Arc::downgrade(&self.connect_time),
            end_time: Arc::downgrade(&self.end_time),
            media_sessions: Arc::downgrade(&self.media_sessions),
            local_sdp: Arc::downgrade(&self.local_sdp),
            remote_sdp: Arc::downgrade(&self.remote_sdp),
            dialog: Arc::downgrade(&self.dialog),
            last_response: Arc::downgrade(&self.last_response),
            original_invite: Arc::downgrade(&self.original_invite),
            invite_transaction_id: Arc::downgrade(&self.invite_transaction_id),
            registry: Arc::downgrade(&self.registry),
            // Store strong refs to essential components
            transaction_manager: self.transaction_manager.clone(),
            event_tx: self.event_tx.clone(),
            state_sender: self.state_sender.clone(),
        }
    }
}

/// A weak reference to a Call, safe to pass around without keeping the call alive
#[derive(Debug, Clone)]
pub struct WeakCall {
    /// Unique call ID (strong - small string)
    pub id: String,
    /// Call direction (copy type)
    pub direction: CallDirection,
    /// SIP call ID (strong - small string)
    pub sip_call_id: String,
    /// Local URI (strong - small)
    pub local_uri: Uri,
    /// Remote URI (strong - small)
    pub remote_uri: Uri,
    /// Remote address (copy type)
    pub remote_addr: SocketAddr,
    /// State watcher receiver (needs to be strong to receive updates)
    pub state_watcher: watch::Receiver<CallState>,
    
    // Weak references to internal state
    remote_tag: Weak<RwLock<Option<String>>>,
    state: Weak<RwLock<CallState>>,
    connect_time: Weak<RwLock<Option<Instant>>>,
    end_time: Weak<RwLock<Option<Instant>>>,
    media_sessions: Weak<RwLock<Vec<MediaSession>>>,
    local_sdp: Weak<RwLock<Option<SessionDescription>>>,
    remote_sdp: Weak<RwLock<Option<SessionDescription>>>,
    dialog: Weak<RwLock<Option<Dialog>>>,
    last_response: Weak<RwLock<Option<Response>>>,
    original_invite: Weak<RwLock<Option<Request>>>,
    invite_transaction_id: Weak<RwLock<Option<String>>>,
    registry: Weak<RwLock<Option<Arc<dyn CallRegistryInterface + Send + Sync>>>>,
    
    // These need to remain strong since they're external connections
    transaction_manager: Arc<TransactionManager>,
    event_tx: mpsc::Sender<CallEvent>,
    state_sender: Arc<watch::Sender<CallState>>,
}

impl WeakCall {
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

    /// Get the current call state from the watcher (without awaiting)
    pub fn current_state(&self) -> CallState {
        *self.state_watcher.borrow()
    }
    
    /// Get the current call state - async to match Call.state() signature
    pub async fn state(&self) -> CallState {
        // For compatibility with Call.state() which is async
        *self.state_watcher.borrow()
    }

    /// Get the remote URI
    pub fn remote_uri(&self) -> &Uri {
        &self.remote_uri
    }
    
    /// Attempt to upgrade to a strong Call reference
    /// Returns None if the original Call has been dropped
    pub fn upgrade(&self) -> Option<Arc<Call>> {
        // Try to upgrade the dialog field since it's critical for operations like hangup
        let _dialog = self.dialog.upgrade()?;
        
        // Create a BYE request directly using our stored information rather than
        // trying to recreate the entire Call object
        
        // The most important part for hangup is to have:
        // 1. Transaction manager
        // 2. Remote address
        // 3. Dialog information
        // 4. State update mechanisms
        
        // Rather than reconstructing everything, we have all the essentials to
        // implement direct methods on WeakCall that don't require upgrade()
        
        // This is a fallback implementation
        error!("WeakCall.upgrade() called - this is inefficient and may not work fully");
        error!("Consider implementing the needed operation directly on WeakCall");
        
        // For most use cases, we should use strong references for vital operations
        // and implement them directly on WeakCall without reconstructing Call
        None
    }
    
    /// Attempt to send a DTMF digit using direct implementation
    pub async fn send_dtmf(&self, digit: char) -> Result<()> {
        debug!("Sending DTMF {} for call {}", digit, self.id);
        
        // Create a DTMF payload according to RFC 2833
        let dtmf_body = format!("Signal={}\r\nDuration=250", digit);
        let dtmf_body_bytes = Bytes::from(dtmf_body);
        let body_len = dtmf_body_bytes.len() as i64;
        
        // Get current state to verify we can send DTMF
        let current_state = self.current_state();
        if current_state != CallState::Established {
            return Err(Error::Call(
                format!("Cannot send DTMF in {} state", current_state)
            ));
        }
        
        // Generate an INFO request directly
        let mut req = Request::new(Method::Info, self.remote_uri.clone());
        
        // Add necessary headers
        req.headers.push(Header::text(HeaderName::From, 
            format!("<{}>;tag=", self.local_uri)));
        req.headers.push(Header::text(HeaderName::To, 
            format!("<{}>", self.remote_uri)));
        req.headers.push(Header::text(HeaderName::CallId, 
            self.sip_call_id.clone()));
        req.headers.push(Header::text(HeaderName::ContentType, 
            "application/dtmf-relay"));
        
        // Set body and Content-Length
        req.body = dtmf_body_bytes;
        req.headers.push(Header::integer(HeaderName::ContentLength, body_len));
        
        // Send via transaction manager
        let transaction_id = self.transaction_manager.create_client_transaction(
            req,
            self.remote_addr,
        ).await.map_err(|e| Error::Transport(e.to_string()))?;
        
        match self.transaction_manager.send_request(&transaction_id).await {
            Ok(_) => {
                debug!("Successfully sent DTMF {} for call {}", digit, self.id);
                Ok(())
            },
            Err(e) => {
                error!("Failed to send DTMF request: {}", e);
                Err(Error::Transport(e.to_string()))
            }
        }
    }
    
    /// Hang up a call using direct implementation
    pub async fn hangup(&self) -> Result<()> {
        let current_state = self.current_state();
        if current_state == CallState::Terminated || current_state == CallState::Terminating {
            return Ok(());
        }
        
        debug!("Sending hangup (BYE) for call {}", self.id);
        
        // Create BYE request
        let mut bye_request = Request::new(Method::Bye, self.remote_uri.clone());
        
        // Add From header with tag
        bye_request.headers.push(Header::text(
            HeaderName::From, 
            format!("<{}>;tag=", self.local_uri) // The tag might be missing
        ));
        
        // Add To header with remote tag if available
        bye_request.headers.push(Header::text(
            HeaderName::To, 
            format!("<{}>", self.remote_uri)
        ));
        
        // Add Call-ID header
        bye_request.headers.push(Header::text(
            HeaderName::CallId, 
            self.sip_call_id.clone()
        ));
        
        // Add CSeq header (use a reasonable value)
        bye_request.headers.push(Header::text(
            HeaderName::CSeq,
            "21 BYE" // Hard-coded CSeq as we don't track it in WeakCall
        ));
        
        // Add Content-Length
        bye_request.headers.push(Header::integer(
            HeaderName::ContentLength, 
            0
        ));
        
        // Send via transaction manager
        debug!("Creating transaction for BYE request");
        let transaction_id = self.transaction_manager.create_client_transaction(
            bye_request,
            self.remote_addr
        ).await.map_err(|e| {
            error!("Failed to create transaction for BYE: {}", e);
            Error::Transport(e.to_string())
        })?;
        
        debug!("Sending BYE request via transaction {}", transaction_id);
        match self.transaction_manager.send_request(&transaction_id).await {
            Ok(_) => {
                // Update our tracked state - we know it's terminating now
                if let Err(e) = self.state_sender.send(CallState::Terminating) {
                    warn!("Failed to update call state to Terminating: {:?}", e);
                }
                
                debug!("Successfully sent BYE for call {}", self.id);
                Ok(())
            },
            Err(e) => {
                error!("Failed to send BYE request: {}", e);
                Err(Error::Transport(e.to_string()))
            }
        }
    }

    /// Wait until the call is established or terminated
    pub async fn wait_until_established(&self) -> Result<()> {
        // First check if the call is already established using the state_watcher
        let current_state = *self.state_watcher.borrow();
        debug!("Initial call state in wait_until_established (WeakCall): {}", current_state);
        
        if current_state == CallState::Established {
            debug!("Call is already established, returning immediately");
            return Ok(());
        }
        
        if current_state == CallState::Terminated || current_state == CallState::Failed {
            debug!("Call is in terminal state {}, cannot establish", current_state);
            return Err(Error::Call("Call terminated before being established".into()));
        }
        
        // Wait for state changes via the state_watcher
        debug!("Waiting for call to establish...");
        
        // Use a longer timeout (30 seconds is common for SIP call setup)
        let start = std::time::Instant::now();
        let timeout_duration = std::time::Duration::from_secs(30);
        
        // Create a clone of the state_watcher to avoid borrowing issues
        let mut watcher = self.state_watcher.clone();
        
        loop {
            // Check if we've exceeded the timeout
            if start.elapsed() > timeout_duration {
                debug!("Timed out waiting for call to establish");
                return Err(Error::Timeout("Timed out waiting for call to establish".into()));
            }
            
            // Wait for the next state change
            if watcher.changed().await.is_err() {
                // The sender was dropped, which could mean the Call was dropped
                return Err(Error::Call("Call state watcher closed".into()));
            }
            
            // Check the new state
            let state = *watcher.borrow();
            debug!("Call state changed to: {}", state);
            
            match state {
                CallState::Established => {
                    debug!("Call established successfully after waiting");
                    return Ok(());
                },
                CallState::Terminated | CallState::Failed => {
                    debug!("Call terminated while waiting for establishment");
                    return Err(Error::Call("Call terminated before being established".into()));
                },
                _ => {
                    // Continue waiting
                    debug!("Call in intermediate state {}, continuing to wait", state);
                    continue;
                }
            }
        }
    }

    /// Wait until the call is terminated
    pub async fn wait_until_terminated(&self) -> Result<()> {
        // Create a clone of the state_watcher to avoid borrowing issues
        let mut watcher = self.state_watcher.clone();
        
        loop {
            let state = *watcher.borrow();
            if state == CallState::Terminated || state == CallState::Failed {
                return Ok(());
            }

            // Wait for state change
            if watcher.changed().await.is_err() {
                return Err(Error::Call("Call state watcher closed".into()));
            }
        }
    }
} 