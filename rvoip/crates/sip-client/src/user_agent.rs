use std::collections::HashMap;
use std::net::{SocketAddr, IpAddr};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock, Mutex};
use tokio::time::Instant;
use tracing::{debug, error, info, warn, trace};
use uuid::Uuid;

use rvoip_sip_core::{
    Request, Response, Message, Method, StatusCode, 
    Uri, Header, HeaderName, HeaderValue
};
use rvoip_sip_transport::UdpTransport;
use rvoip_transaction_core::{TransactionManager, TransactionEvent};
use rvoip_session_core::sdp::{SessionDescription, extract_rtp_port_from_sdp};

use crate::config::{ClientConfig, CallConfig};
use crate::error::{Error, Result};
use crate::call::{Call, CallState, CallEvent, CallDirection};
use crate::media::MediaSession;

/// User agent for receiving SIP calls
pub struct UserAgent {
    /// Client configuration
    config: ClientConfig,
    
    /// Local address
    local_addr: SocketAddr,
    
    /// Username
    username: String,
    
    /// Domain
    domain: String,
    
    /// Transaction manager
    transaction_manager: Arc<TransactionManager>,
    
    /// Event receiver
    events_rx: mpsc::Receiver<TransactionEvent>,
    
    /// Active calls
    active_calls: Arc<RwLock<HashMap<String, Arc<Call>>>>,
    
    /// Event sender for client events
    event_tx: mpsc::Sender<CallEvent>,
    
    /// Event receiver for client events
    event_rx: mpsc::Receiver<CallEvent>,
    
    /// Is the UA running
    running: Arc<RwLock<bool>>,
    
    /// Background task handle
    event_task: Option<tokio::task::JoinHandle<()>>,
}

impl UserAgent {
    /// Create a new user agent
    pub async fn new(config: ClientConfig) -> Result<Self> {
        // Get local address from config or use default
        let local_addr = config.local_addr
            .ok_or_else(|| Error::Configuration("Local address must be specified".into()))?;
        
        // Create UDP transport
        let (udp_transport, transport_rx) = UdpTransport::bind(local_addr, None).await
            .map_err(|e| Error::Transport(e.to_string()))?;
        
        info!("SIP user agent UDP transport bound to {}", local_addr);
        
        // Wrap transport in Arc
        let arc_transport = Arc::new(udp_transport);
        
        // Create transaction manager
        let (transaction_manager, events_rx) = TransactionManager::new(
            arc_transport,
            transport_rx,
            Some(config.transaction.max_events),
        ).await.map_err(|e| Error::Transport(e.to_string()))?;
        
        // Create event channel (keep both sender and receiver)
        let (event_tx, event_rx) = mpsc::channel(32);
        
        info!("SIP user agent initialized with username: {}", config.username);
        
        Ok(Self {
            config: config.clone(),
            local_addr,
            username: config.username.clone(),
            domain: config.domain.clone(),
            transaction_manager: Arc::new(transaction_manager),
            events_rx,
            active_calls: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
            event_rx,
            running: Arc::new(RwLock::new(false)),
            event_task: None,
        })
    }
    
    /// Generate a new branch parameter
    fn new_branch(&self) -> String {
        format!("z9hG4bK-{}", Uuid::new_v4())
    }
    
    /// Start the user agent
    pub async fn start(&mut self) -> Result<()> {
        // Check if already running
        if *self.running.read().await {
            return Ok(());
        }
        
        // Set running flag
        *self.running.write().await = true;
        
        // Start event processing task
        let mut events_rx = std::mem::replace(&mut self.events_rx, mpsc::channel(1).1);
        let transaction_manager = self.transaction_manager.clone();
        let active_calls = self.active_calls.clone();
        let running = self.running.clone();
        let event_tx = self.event_tx.clone();
        let config = self.config.clone();
        
        let event_task = tokio::spawn(async move {
            debug!("SIP user agent event processing task started");
            
            while *running.read().await {
                // Wait for transaction event with timeout
                let event = match tokio::time::timeout(
                    Duration::from_secs(1),
                    events_rx.recv()
                ).await {
                    Ok(Some(event)) => event,
                    Ok(None) => {
                        error!("Transaction event channel closed");
                        break;
                    },
                    Err(_) => {
                        // Timeout, continue
                        continue;
                    }
                };
                
                // Process transaction event
                match event {
                    TransactionEvent::UnmatchedMessage { message, source } => {
                        // Handle unmatched message (typically requests)
                        match message {
                            Message::Request(request) => {
                                debug!("Received {} request from {}", request.method, source);
                                
                                // Handle incoming request
                                if let Err(e) = handle_incoming_request(
                                    request,
                                    source,
                                    transaction_manager.clone(),
                                    active_calls.clone(),
                                    event_tx.clone(),
                                    &config,
                                ).await {
                                    error!("Error handling incoming request: {}", e);
                                }
                            },
                            Message::Response(response) => {
                                debug!("Received unmatched response: {:?} from {}", response.status, source);
                            }
                        }
                    },
                    TransactionEvent::TransactionCreated { transaction_id } => {
                        debug!("Transaction created: {}", transaction_id);
                    },
                    TransactionEvent::TransactionCompleted { transaction_id, response } => {
                        debug!("Transaction completed: {}", transaction_id);
                        
                        // Forward to any active calls that might be interested
                        if let Some(response) = response {
                            // Extract Call-ID to find the call
                            if let Some(call_id) = response.call_id() {
                                let calls_read = active_calls.read().await;
                                if let Some(call) = calls_read.get(call_id) {
                                    // Let the call handle the response (now works with immutable reference)
                                    if let Err(e) = call.handle_response(response.clone()).await {
                                        error!("Error handling response in call: {}", e);
                                    }
                                }
                            }
                        }
                    },
                    TransactionEvent::TransactionTerminated { transaction_id } => {
                        debug!("Transaction terminated: {}", transaction_id);
                    },
                    TransactionEvent::Error { error, transaction_id } => {
                        error!("Transaction error: {}, id: {:?}", error, transaction_id);
                    },
                }
            }
            
            debug!("SIP user agent event processing task ended");
        });
        
        self.event_task = Some(event_task);
        
        Ok(())
    }
    
    /// Stop the user agent
    pub async fn stop(&mut self) -> Result<()> {
        // Check if not running
        if !*self.running.read().await {
            return Ok(());
        }
        
        // Set running flag to false
        *self.running.write().await = false;
        
        // Wait for event task to end
        if let Some(task) = self.event_task.take() {
            task.abort();
            let _ = tokio::time::timeout(Duration::from_millis(100), task).await;
        }
        
        // Hang up all active calls
        let calls = self.active_calls.read().await;
        for (_, call) in calls.iter() {
            let _ = call.hangup().await;
        }
        
        Ok(())
    }
    
    /// Get an event stream for call events
    pub fn event_stream(&self) -> mpsc::Receiver<CallEvent> {
        // Create a new forwarding channel
        let (tx, rx) = mpsc::channel(32);
        
        // Create a clone for the first task
        let tx1 = tx.clone();
        
        tokio::spawn(async move {
            // Periodically send events to keep the channel active
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        // Just a keep-alive tick
                    },
                    // If the channel closes, exit the task
                    _ = tx1.closed() => break,
                }
                
                // Simple poll event to keep the connection alive
                let dummy_event = CallEvent::StateChanged {
                    call: Arc::new(Call::dummy()),
                    previous: CallState::Initial,
                    current: CallState::Initial,
                };
                
                if tx1.send(dummy_event).await.is_err() {
                    break;
                }
            }
        });
        
        // Also periodically check call states to ensure state change events are sent
        let active_calls = self.active_calls.clone();
        let tx2 = tx.clone();
        
        tokio::spawn(async move {
            // Create a periodic timer (every 500ms)
            let mut interval = tokio::time::interval(Duration::from_millis(500));
            
            // Keep track of last known states for each call
            let mut last_states = HashMap::new();
            
            // Keep running until receiver is closed
            while !tx2.is_closed() {
                interval.tick().await;
                
                // Get all active calls
                let calls = active_calls.read().await;
                
                // For each active call, check if state has changed
                for (call_id, call) in calls.iter() {
                    let current_state = call.state().await;
                    let previous_state = last_states.get(call_id).copied().unwrap_or(CallState::Initial);
                    
                    // Only send if state changed
                    if current_state != previous_state {
                        info!("Call {} state changed: {} -> {}", call_id, previous_state, current_state);
                        
                        // Send the state change event
                        let event = CallEvent::StateChanged {
                            call: call.clone(),
                            previous: previous_state,
                            current: current_state,
                        };
                        
                        if tx2.send(event).await.is_err() {
                            break;
                        }
                        
                        // Update the last known state
                        last_states.insert(call_id.clone(), current_state);
                    }
                }
                
                // Cleanup removed calls
                last_states.retain(|call_id, _| calls.contains_key(call_id));
            }
        });
        
        rx
    }
    
    /// Run the user agent event loop
    pub async fn run(&mut self) -> Result<()> {
        // Start the user agent if not already running
        if !*self.running.read().await {
            self.start().await?;
        }
        
        info!("SIP user agent {} started, waiting for requests on {}...", self.username, self.local_addr);
        
        // Wait for termination signal
        match tokio::signal::ctrl_c().await {
            Ok(()) => {
                info!("Received termination signal, shutting down");
                self.stop().await?;
                Ok(())
            },
            Err(e) => {
                error!("Error waiting for termination signal: {}", e);
                self.stop().await?;
                Err(Error::Other(format!("Error waiting for termination signal: {}", e)))
            }
        }
    }
    
    /// Get active calls
    pub async fn calls(&self) -> HashMap<String, Arc<Call>> {
        self.active_calls.read().await.clone()
    }
    
    /// Get call by ID
    pub async fn call_by_id(&self, call_id: &str) -> Option<Arc<Call>> {
        self.active_calls.read().await.get(call_id).cloned()
    }
}

/// Add common headers to a response based on a request
fn add_response_headers(request: &Request, response: &mut Response) {
    // Copy headers from request
    for header in &request.headers {
        match header.name {
            HeaderName::Via | HeaderName::From | HeaderName::CallId | HeaderName::CSeq => {
                response.headers.push(header.clone());
            },
            _ => {},
        }
    }
    
    // Add Content-Length if not present
    if !response.headers.iter().any(|h| h.name == HeaderName::ContentLength) {
        response.headers.push(Header::text(HeaderName::ContentLength, "0"));
    }
}

/// Handle an incoming SIP request
async fn handle_incoming_request(
    request: Request,
    source: SocketAddr,
    transaction_manager: Arc<TransactionManager>,
    active_calls: Arc<RwLock<HashMap<String, Arc<Call>>>>,
    event_tx: mpsc::Sender<CallEvent>,
    config: &ClientConfig,
) -> Result<()> {
    debug!("Handling incoming {} request from {}", request.method, source);
    
    // Extract Call-ID
    let call_id = match request.call_id() {
        Some(id) => id.to_string(),
        None => return Err(Error::SipProtocol("Request missing Call-ID".into())),
    };
    
    // Log message receipt for debugging
    debug!("Received {} for call {}: {:?}", request.method, call_id, request);
    
    // Check for existing call
    let calls_read = active_calls.read().await;
    let existing_call = calls_read.get(&call_id).cloned();
    drop(calls_read);
    
    // Handling INVITE requests
    if request.method == Method::Invite && existing_call.is_none() {
        debug!("Processing new INVITE request from {}", source);

        // Extract From URI for caller identification
        let from_uri = match extract_uri_from_header(&request, HeaderName::From) {
            Some(uri) => uri,
            None => return Err(Error::SipProtocol("Missing From URI".into())),
        };

        // Extract To URI
        let to_uri = match extract_uri_from_header(&request, HeaderName::To) {
            Some(uri) => uri,
            None => return Err(Error::SipProtocol("Missing To URI".into())),
        };

        // Get To header value for tag
        let to_header_value = match request.headers.iter()
            .find(|h| h.name == HeaderName::To)
            .and_then(|h| h.value.as_text()) {
            Some(value) => value.to_string(),
            None => return Err(Error::SipProtocol("Missing To header".into())),
        };

        // Generate tag for To header
        let to_tag = format!("tag-{}", Uuid::new_v4());
        let to_with_tag = format!("{};tag={}", to_header_value, to_tag);

        info!("Processing INVITE for call {}", call_id);

        // Debug SDP content if present
        if !request.body.is_empty() {
            if let Ok(sdp_str) = std::str::from_utf8(&request.body) {
                debug!("Received SDP in INVITE:\n{}", sdp_str);
            } else {
                warn!("INVITE contains body but it's not valid UTF-8");
            }
        } else {
            warn!("INVITE request has no SDP body");
        }

        // Create call config from client config
        let call_config = CallConfig {
            audio_enabled: config.media.rtp_enabled,
            video_enabled: false,
            dtmf_enabled: true,
            auto_answer: config.media.rtp_enabled,
            auto_answer_delay: Duration::from_secs(0),
            call_timeout: Duration::from_secs(60),
            media: None,
            auth_username: None,
            auth_password: None,
            display_name: None,
        };

        // Create call with auto-generated ID
        let (call, state_tx) = Call::new(
            CallDirection::Incoming,
            call_config,
            call_id.clone(),
            to_tag,
            to_uri,
            from_uri.clone(),
            source,
            transaction_manager.clone(),
            event_tx.clone(),
        );

        // Send a ringing response
        let mut ringing_response = Response::new(StatusCode::Ringing);
        add_response_headers(&request, &mut ringing_response);

        // Add To header with tag
        ringing_response.headers.push(Header::text(HeaderName::To, to_with_tag.clone()));

        debug!("Sending 180 Ringing for call {}", call_id);

        // Send 180 Ringing
        if let Err(e) = transaction_manager.transport().send_message(
            Message::Response(ringing_response),
            source
        ).await {
            warn!("Failed to send 180 Ringing: {}", e);
        }

        // Update call state to ringing
        if let Err(e) = state_tx.send(CallState::Ringing)
            .map_err(|_| Error::Call("Failed to update call state".into())) {
            error!("Failed to update call state to Ringing: {}", e);
        } else {
            debug!("Call {} state updated to Ringing", call_id);
        }

        // Store call
        active_calls.write().await.insert(call_id.clone(), call.clone());

        // Send event
        if let Err(e) = event_tx.send(CallEvent::IncomingCall(call.clone())).await
            .map_err(|_| Error::Call("Failed to send call event".into())) {
            error!("Failed to send IncomingCall event: {}", e);
        } else {
            debug!("Sent IncomingCall event for call {}", call_id);
        }

        // If auto-answer is enabled, answer the call
        if config.media.rtp_enabled {
            debug!("Auto-answer is enabled, proceeding to answer call {}", call_id);
            
            // Extract remote SDP
            if !request.body.is_empty() {
                match std::str::from_utf8(&request.body)
                    .map_err(|_| Error::SipProtocol("Invalid UTF-8 in SDP".into()))
                    .and_then(|sdp_str| SessionDescription::parse(sdp_str)
                        .map_err(|e| Error::SipProtocol(format!("Invalid SDP: {}", e))))
                {
                    Ok(remote_sdp) => {
                        debug!("Successfully parsed SDP from INVITE");
                        
                        // Store SDP in the call for media setup
                        if let Err(e) = call.setup_media_from_sdp(&remote_sdp).await {
                            warn!("Error setting up media from SDP: {}", e);
                        }
                    },
                    Err(e) => {
                        warn!("Failed to parse SDP: {}", e);
                        debug!("SDP content that failed to parse: {:?}", 
                            String::from_utf8_lossy(&request.body));
                    }
                }
            } else {
                debug!("No SDP body in INVITE, skipping SDP parsing");
            }
            
            // Extract remote RTP port from SDP
            let remote_rtp_port = if !request.body.is_empty() {
                extract_rtp_port_from_sdp(&request.body)
            } else {
                None
            };
            
            if remote_rtp_port.is_none() {
                warn!("Could not extract RTP port from INVITE SDP");
                
                // Store the failed body for troubleshooting
                if !request.body.is_empty() {
                    let body_text = String::from_utf8_lossy(&request.body);
                    debug!("SDP body that failed to extract RTP port:\n{}", body_text);
                }
            } else {
                info!("Remote endpoint RTP port is {}", remote_rtp_port.unwrap());
                
                // Create OK response
                let mut ok_response = Response::new(StatusCode::Ok);
                add_response_headers(&request, &mut ok_response);
                
                // Add To header with tag
                ok_response.headers.push(Header::text(HeaderName::To, to_with_tag.clone()));
                
                // Add Contact header
                let contact = format!("<sip:{}@{}>", config.username, config.local_addr.unwrap());
                ok_response.headers.push(Header::text(HeaderName::Contact, contact));
                
                // Create SDP answer
                let local_rtp_port = config.media.rtp_port_min;
                let sdp = SessionDescription::new_audio_call(
                    &config.username,
                    config.local_addr.unwrap().ip(),
                    local_rtp_port
                );
                
                let sdp_str = sdp.to_string();
                debug!("Created SDP answer:\n{}", sdp_str);
                
                // Store SDP in the call for later use
                if let Err(e) = call.setup_local_sdp().await {
                    warn!("Failed to set up local SDP: {}", e);
                }
                
                // Add Content-Type and Content-Length
                ok_response.headers.push(Header::text(HeaderName::ContentType, "application/sdp"));
                ok_response.headers.push(Header::text(
                    HeaderName::ContentLength, 
                    sdp_str.len().to_string()
                ));
                
                // Add SDP body
                ok_response.body = sdp_str.into();
                
                debug!("Sending 200 OK with SDP answer for call {}", call_id);
                
                // Update call state to connecting BEFORE sending response
                if let Err(e) = call.update_state(CallState::Connecting).await {
                    error!("Failed to update call state to Connecting: {}", e);
                } else {
                    info!("Call {} transitioned from Ringing to Connecting", call_id);
                }
                
                // Send 200 OK
                if let Err(e) = transaction_manager.transport().send_message(
                    Message::Response(ok_response),
                    source
                ).await {
                    warn!("Failed to send 200 OK: {}", e);
                }
            }
        }
        
        return Ok(());
    }
    
    // Handle request for existing call
    if let Some(call) = existing_call {
        debug!("Processing {} request for existing call {}", request.method, call_id);
        
        // Handle ACK to 200 OK specially for state transition
        if request.method == Method::Ack {
            // When we receive an ACK after sending 200 OK, the call is now established
            let current_state = call.state().await;
            if current_state == CallState::Connecting {
                debug!("Received ACK for call {} in Connecting state, transitioning to Established", call_id);
                
                // Directly update the call's state to Established
                if let Err(e) = call.transition_to(CallState::Established).await {
                    warn!("Failed to update call state to Established: {}", e);
                } else {
                    info!("Call {} established successfully after ACK", call_id);
                }
                
                return Ok(());
            } else {
                debug!("Received ACK for call {} in state {}, not transitioning", call_id, current_state);
            }
        }
        
        // Let the call handle other requests
        return match call.handle_request(request).await? {
            Some(response) => {
                debug!("Sending response {} for call {}", response.status, call_id);
                
                // Send response
                transaction_manager.transport().send_message(
                    Message::Response(response),
                    source
                ).await.map_err(|e| Error::Transport(e.to_string()))?;
                
                Ok(())
            },
            None => Ok(()),
        };
    }
    
    debug!("No matching call for {} request with Call-ID {}", request.method, call_id);
    
    // No matching call, reject with 481 Call/Transaction Does Not Exist
    if request.method != Method::Ack {
        let mut response = Response::new(StatusCode::CallOrTransactionDoesNotExist);
        add_response_headers(&request, &mut response);
        
        debug!("Sending 481 Call/Transaction Does Not Exist for {}", call_id);
        
        // Send response
        transaction_manager.transport().send_message(
            Message::Response(response),
            source
        ).await.map_err(|e| Error::Transport(e.to_string()))?;
    }
    
    Ok(())
}

/// Helper function to extract URI from a SIP header
fn extract_uri_from_header(request: &Request, header_name: HeaderName) -> Option<Uri> {
    let header = request.headers.iter()
        .find(|h| h.name == header_name)?;
    
    let value = header.value.as_text()?;
    
    // Extract URI from the header value
    let uri_str = if let Some(uri_end) = value.find('>') {
        if let Some(uri_start) = value.find('<') {
            &value[uri_start + 1..uri_end]
        } else {
            value
        }
    } else {
        value
    };
    
    // Parse the URI
    match uri_str.parse() {
        Ok(uri) => Some(uri),
        Err(_) => None,
    }
} 