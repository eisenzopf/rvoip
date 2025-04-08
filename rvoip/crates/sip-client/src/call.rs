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
#[derive(Debug, Clone)]
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
pub trait CallRegistryInterface: std::fmt::Debug + Send + Sync {
    /// Log a transaction
    async fn log_transaction(&self, call_id: &str, transaction: crate::call_registry::TransactionRecord) -> Result<()>;
    
    /// Get transactions for a call
    async fn get_transactions(&self, call_id: &str) -> Result<Vec<crate::call_registry::TransactionRecord>>;
    
    /// Update transaction status
    async fn update_transaction_status(&self, call_id: &str, transaction_id: &str, status: &str, info: Option<String>) -> Result<()>;
    
    /// Get transaction destination (SocketAddr) from the registry, used for ACK fallback
    async fn get_transaction_destination(&self, call_id: &str) -> Result<Option<SocketAddr>>;
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
    
    // Registry reference (weak)
    registry: Weak<RwLock<Option<Arc<dyn CallRegistryInterface + Send + Sync>>>>,
}

impl WeakCall {
    /// Get the call registry
    pub fn registry(&self) -> Option<Arc<dyn CallRegistryInterface + Send + Sync>> {
        // Try to upgrade the weak reference
        match self.registry.upgrade() {
            Some(registry_lock) => {
                // Try to acquire the read lock
                match registry_lock.try_read() {
                    Ok(registry_guard) => registry_guard.clone(),
                    Err(_) => None,
                }
            },
            None => None,
        }
    }
    
    /// Get the current call state
    pub async fn state(&self) -> CallState {
        // Use the state_watcher which we keep a strong reference to
        *self.state_watcher.borrow()
    }
    
    /// Hang up the call
    pub async fn hangup(&self) -> Result<()> {
        // Try to upgrade to a full Call
        if let Some(call) = self.upgrade() {
            call.hangup().await
        } else {
            Err(Error::Call("Cannot hang up: call no longer exists".into()))
        }
    }
    
    /// Wait until the call is established or fails
    pub async fn wait_until_established(&self) -> Result<()> {
        // First check if the call is already established using the state_watcher
        let current_state = *self.state_watcher.borrow();
        
        if current_state == CallState::Established {
            return Ok(());
        }
        
        if current_state == CallState::Terminated || current_state == CallState::Failed {
            return Err(Error::Call("Call terminated before being established".into()));
        }
        
        // Wait for state changes via the state_watcher
        let start = std::time::Instant::now();
        let timeout_duration = std::time::Duration::from_secs(30);
        
        // Create a clone of the state_watcher to avoid borrowing issues
        let mut watcher = self.state_watcher.clone();
        
        loop {
            // Check if we've exceeded the timeout
            if start.elapsed() > timeout_duration {
                return Err(Error::Timeout("Timed out waiting for call to establish".into()));
            }
            
            // Wait for the next state change
            if watcher.changed().await.is_err() {
                // The sender was dropped, which could mean the Call was dropped
                return Err(Error::Call("Call state watcher closed".into()));
            }
            
            // Check the new state
            let state = *watcher.borrow();
            match state {
                CallState::Established => {
                    return Ok(());
                },
                CallState::Terminated | CallState::Failed => {
                    return Err(Error::Call("Call terminated before being established".into()));
                },
                _ => {
                    // Continue waiting
                    continue;
                }
            }
        }
    }
    
    /// Upgrade a weak call reference to a strong Arc<Call> reference if possible
    pub fn upgrade(&self) -> Option<Arc<Call>> {
        // First check if we can upgrade the necessary components
        let state = match self.state.upgrade() {
            Some(state) => state,
            None => return None,
        };
        
        let remote_tag = match self.remote_tag.upgrade() {
            Some(remote_tag) => remote_tag,
            None => return None,
        };
        
        let connect_time = match self.connect_time.upgrade() {
            Some(connect_time) => connect_time,
            None => return None,
        };
        
        let end_time = match self.end_time.upgrade() {
            Some(end_time) => end_time,
            None => return None,
        };
        
        // Upgrade registry weak reference
        let registry_opt = self.registry.upgrade().and_then(|lock| {
            if let Ok(guard) = lock.try_read() {
                guard.clone()
            } else {
                None
            }
        });
        
        // Create a minimal dummy transaction manager
        // Instead of calling non-existent new_dummy method, create a default UDP transport
        // and initialize a real transaction manager (but not connected)
        let dummy_transport = Arc::new(rvoip_sip_transport::UdpTransport::default());
        let (_, dummy_rx) = tokio::sync::mpsc::channel(1);
        let dummy_transaction_manager = Arc::new(rvoip_transaction_core::TransactionManager::dummy(dummy_transport, dummy_rx));
        
        // Create a minimal Call with the available information
        let call = Call {
            id: self.id.clone(),
            direction: self.direction,
            config: CallConfig::default(), // Use default config
            sip_call_id: self.sip_call_id.clone(),
            local_tag: String::new(), // Empty local tag
            remote_tag,
            cseq: Arc::new(Mutex::new(1)), // Default CSeq
            local_uri: self.local_uri.clone(),
            remote_uri: self.remote_uri.clone(),
            remote_display_name: Arc::new(RwLock::new(None)),
            remote_addr: self.remote_addr,
            transaction_manager: dummy_transaction_manager,
            state,
            state_watcher: self.state_watcher.clone(),
            state_sender: Arc::new(watch::channel(CallState::Initial).0), // Dummy sender
            start_time: None, // Unknown start time
            connect_time,
            end_time,
            media_sessions: Arc::new(RwLock::new(Vec::new())),
            event_tx: mpsc::channel(10).0, // Dummy event channel
            local_sdp: Arc::new(RwLock::new(None)),
            remote_sdp: Arc::new(RwLock::new(None)),
            dialog: Arc::new(RwLock::new(None)),
            last_response: Arc::new(RwLock::new(None)),
            original_invite: Arc::new(RwLock::new(None)),
            invite_transaction_id: Arc::new(RwLock::new(None)),
            registry: Arc::new(RwLock::new(registry_opt)),
        };
        
        Some(Arc::new(call))
    }
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

    /// Get the dialog for this call
    pub async fn dialog(&self) -> Option<Dialog> {
        self.dialog.read().await.clone()
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
                        let dialog = Dialog {
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
    pub(crate) async fn handle_response(&self, response: &Response) -> Result<()> {
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
                        if let Some(dialog) = Dialog::from_2xx_response(&invite_req, response, true) {
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
        
        // Try looking up the transaction in the registry
        if !ack_sent {
            debug!("Attempting to look up transaction in call registry");
            if let Some(registry) = self.registry.read().await.as_ref() {
                // Try to get INVITE transaction from registry
                match registry.get_transactions(&self.id).await {
                    Ok(transactions) => {
                        // Find the INVITE transaction if any
                        if let Some(invite_tx) = transactions.iter().find(|tx| tx.transaction_type == "INVITE") {
                            debug!("Found INVITE transaction in registry: {}", invite_tx.transaction_id);
                            
                            if let Some(response) = response_opt.clone() {
                                // Try sending ACK using this transaction ID
                                match self.transaction_manager.send_2xx_ack(&invite_tx.transaction_id, &response).await {
                                    Ok(_) => {
                                        debug!("ACK sent successfully via registry transaction lookup");
                                        ack_sent = true;
                                    },
                                    Err(e) => {
                                        warn!("Failed to send ACK using registry transaction: {}", e);
                                        // Continue with fallbacks
                                    }
                                }
                            }
                        } else {
                            debug!("No INVITE transaction found in registry");
                        }
                    },
                    Err(e) => {
                        warn!("Failed to get transactions from registry: {}", e);
                    }
                }
            } else {
                debug!("No call registry available for transaction lookup");
            }
        }
        
        // Last resort fallback: Get transaction destination directly from registry
        if !ack_sent {
            debug!("Attempting to get transaction destination from registry");
            if let Some(registry) = self.registry.read().await.as_ref() {
                // Try to get INVITE transaction destination from registry
                match registry.get_transaction_destination(&self.id).await {
                    Ok(Some(destination)) => {
                        debug!("Found transaction destination in registry: {}", destination);
                        
                        // Send directly to this destination
                        match self.transaction_manager.transport().send_message(
                            Message::Request(request.clone()),
                            destination
                        ).await {
                            Ok(_) => {
                                debug!("ACK sent successfully via registry destination lookup");
                                ack_sent = true;
                            },
                            Err(e) => {
                                warn!("Failed to send ACK using registry destination: {}", e);
                                // Continue with fallbacks
                            }
                        }
                    },
                    Ok(None) => {
                        debug!("No transaction destination found in registry");
                    },
                    Err(e) => {
                        warn!("Failed to get transaction destination from registry: {}", e);
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

    /// Get a reference to the call registry if set
    pub fn registry(&self) -> Option<Arc<dyn CallRegistryInterface + Send + Sync>> {
        match self.registry.try_read() {
            Ok(guard) => guard.clone(),
            Err(_) => None
        }
    }
    
    /// Store the INVITE transaction ID for later use
    pub async fn store_invite_transaction_id(&self, transaction_id: String) -> Result<()> {
        let mut id_guard = self.invite_transaction_id.write().await;
        *id_guard = Some(transaction_id);
        Ok(())
    }
    
    /// Update the call state with proper handling of state transitions
    pub async fn update_state(&self, new_state: CallState) -> Result<()> {
        let current_state = *self.state.read().await;
        
        // Log the state change
        debug!("Call {} state change: {} -> {}", self.id, current_state, new_state);
        
        // Check if it's a valid transition
        if !is_valid_state_transition(current_state, new_state) {
            return Err(Error::Call(format!(
                "Invalid state transition from {} to {}", 
                current_state, new_state
            )));
        }
        
        // Update the actual state
        {
            let mut state_guard = self.state.write().await;
            let old_state = *state_guard;
            *state_guard = new_state;
            
            // Update state watcher (which notifies listeners)
            if self.state_sender.send(new_state).is_err() {
                error!("Failed to update state watcher for call {}", self.id);
            }
            
            // Send state change event
            let _ = self.event_tx.send(CallEvent::StateChanged {
                call: Arc::new(self.clone()),
                previous: old_state,
                current: new_state,
            }).await;
        }
        
        // Special handling for certain state changes
        match new_state {
            CallState::Established => {
                // Set connect time when transitioning to established
                let mut connect_time = self.connect_time.write().await;
                if connect_time.is_none() {
                    *connect_time = Some(Instant::now());
                    debug!("Call {} connected at {:?}", self.id, connect_time);
                }
            },
            CallState::Terminated | CallState::Failed => {
                // Set end time when terminating
                let mut end_time = self.end_time.write().await;
                if end_time.is_none() {
                    *end_time = Some(Instant::now());
                    debug!("Call {} ended at {:?}", self.id, end_time);
                }
                
                // Send terminated event
                let reason = if new_state == CallState::Failed {
                    "Call failed".to_string()
                } else {
                    "Call terminated normally".to_string()
                };
                
                let _ = self.event_tx.send(CallEvent::Terminated {
                    call: Arc::new(self.clone()),
                    reason,
                }).await;
            },
            _ => {},
        }
        
        Ok(())
    }
    
    /// Alias for update_state for readability
    pub async fn transition_to(&self, new_state: CallState) -> Result<()> {
        self.update_state(new_state).await
    }
    
    /// Store the original INVITE request for incoming calls
    pub async fn store_invite_request(&self, request: Request) -> Result<()> {
        let mut invite_guard = self.original_invite.write().await;
        *invite_guard = Some(request);
        Ok(())
    }
    
    /// Store the last response received for this call
    pub async fn store_last_response(&self, response: Response) -> Result<()> {
        let mut response_guard = self.last_response.write().await;
        *response_guard = Some(response);
        Ok(())
    }
    
    /// Set up media sessions based on SDP
    pub async fn setup_media_from_sdp(&self, sdp: &SessionDescription) -> Result<()> {
        debug!("Setting up media from SDP for call {}", self.id);
        
        // Store remote SDP
        {
            let mut sdp_guard = self.remote_sdp.write().await;
            *sdp_guard = Some(sdp.clone());
        }
        
        // Extract media information from SDP
        if let Some(media) = sdp.media.iter().find(|m| m.media_type == "audio") {
            let port = media.port;
            
            // Extract IP address from connection information
            let ip = if let Some(conn) = &sdp.connection {
                conn.connection_address
            } else {
                // If no connection info in SDP, use the remote address
                self.remote_addr.ip()
            };
            
            // Create socket address for remote RTP endpoint
            let remote_rtp_addr = SocketAddr::new(ip, port);
            debug!("Remote RTP endpoint: {}", remote_rtp_addr);
            
            // In a real implementation, this would create actual RTP sessions
            // For now, just log and send an event
            
            // Send MediaAdded event
            let _ = self.event_tx.send(CallEvent::MediaAdded {
                call: Arc::new(self.clone()),
                media_type: MediaType::Audio,
            }).await;
        }
        
        Ok(())
    }
    
    /// Set the remote tag for this call
    pub async fn set_remote_tag(&self, tag: String) {
        let mut tag_guard = self.remote_tag.write().await;
        *tag_guard = Some(tag.clone());
        debug!("Set remote tag for call {}: {}", self.id, tag);
    }
    
    /// Create a weak reference to this call
    pub fn weak_clone(&self) -> WeakCall {
        WeakCall {
            id: self.id.clone(),
            direction: self.direction,
            sip_call_id: self.sip_call_id.clone(),
            local_uri: self.local_uri.clone(),
            remote_uri: self.remote_uri.clone(),
            remote_addr: self.remote_addr,
            state_watcher: self.state_watcher.clone(),
            
            // Weak references to internal state
            remote_tag: Arc::downgrade(&self.remote_tag),
            state: Arc::downgrade(&self.state),
            connect_time: Arc::downgrade(&self.connect_time),
            end_time: Arc::downgrade(&self.end_time),
            
            // Registry reference
            registry: Arc::downgrade(&self.registry),
        }
    }
}

/// Check if a state transition is valid
fn is_valid_state_transition(from: CallState, to: CallState) -> bool {
    use CallState::*;
    
    match (from, to) {
        // Any state can transition to Failed or Terminated
        (_, Failed) | (_, Terminated) => true,
        
        // Initial can go to Ringing, Connecting or Established
        (Initial, Ringing) | (Initial, Connecting) | (Initial, Established) => true,
        
        // Ringing can go to Connecting or Established
        (Ringing, Connecting) | (Ringing, Established) => true,
        
        // Connecting can go to Established
        (Connecting, Established) => true,
        
        // Established can go to Terminating
        (Established, Terminating) => true,
        
        // All other transitions are invalid
        _ => false,
    }
} 