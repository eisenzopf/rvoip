use std::collections::HashMap;
use std::net::{SocketAddr, IpAddr};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock, Mutex, broadcast};
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
use rvoip_session_core::dialog::DialogState;

use crate::config::{ClientConfig, CallConfig};
use crate::error::{Error, Result};
use crate::call::{Call, CallState, CallEvent, CallDirection};
use crate::media::MediaSession;
use crate::call_registry::{CallRegistry, CallRecord};

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
    
    /// Call registry for storing call history
    call_registry: Arc<CallRegistry>,
    
    /// Event sender for client events
    event_tx: mpsc::Sender<CallEvent>,
    
    /// Event broadcast for client events
    event_broadcast: broadcast::Sender<CallEvent>,
    
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
        
        // Create event channels
        let (event_tx, mut event_rx) = mpsc::channel(32);
        let (event_broadcast, _) = broadcast::channel(32);
        
        // Create call registry with max history size
        let call_registry = Arc::new(CallRegistry::new(config.max_call_history.unwrap_or(100)));
        
        // Create a separate task to forward events from mpsc to broadcast
        let broadcast_tx = event_broadcast.clone();
        let registry_clone = call_registry.clone();
        tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                // Process event to update call registry
                match &event {
                    CallEvent::IncomingCall(call) => {
                        debug!("Registering incoming call {} in registry", call.id());
                        if let Err(e) = registry_clone.register_call(call.clone()).await {
                            error!("Failed to register incoming call: {}", e);
                        }
                    },
                    CallEvent::StateChanged { call, previous, current } => {
                        debug!("Updating call state in registry: {} {} -> {}", call.id(), previous, current);
                        if let Err(e) = registry_clone.update_call_state(call.id(), *previous, *current).await {
                            error!("Failed to update call state in registry: {}", e);
                        }
                    },
                    CallEvent::Terminated { call, .. } => {
                        // Ensure termination is recorded in call history
                        let current_state = match call.state().await {
                            CallState::Terminated => CallState::Terminated,
                            _ => CallState::Terminated, // Force to terminated state
                        };
                        
                        if let Err(e) = registry_clone.update_call_state(
                            call.id(), CallState::Terminating, current_state
                        ).await {
                            if !e.to_string().contains("not found") {
                                error!("Failed to record call termination: {}", e);
                            }
                        }
                        
                        // Clean up old call history if needed
                        registry_clone.cleanup_history().await;
                    },
                    _ => {},
                }
                
                // Forward to broadcast
                let _ = broadcast_tx.send(event);
            }
        });
        
        info!("SIP user agent initialized with username: {}", config.username);
        
        Ok(Self {
            config: config.clone(),
            local_addr,
            username: config.username.clone(),
            domain: config.domain.clone(),
            transaction_manager: Arc::new(transaction_manager),
            events_rx,
            active_calls: Arc::new(RwLock::new(HashMap::new())),
            call_registry,
            event_tx,
            event_broadcast,
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
        let call_registry = self.call_registry.clone();
        
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
                                    call_registry.clone(),
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
                    TransactionEvent::ResponseReceived { message, source: _, transaction_id } => {
                        if let Message::Response(response) = message {
                            debug!("Received response for transaction {}: {}", transaction_id, response.status);
                            
                            // Extract call ID
                            if let Some(call_id) = response.call_id() {
                                // Find the call
                                let calls_read = active_calls.read().await;
                                if let Some(call) = calls_read.get(call_id) {
                                    debug!("Found call {} for response, handling", call_id);
                                    
                                    // Handle response
                                    if let Err(e) = call.handle_response(response).await {
                                        error!("Failed to handle response: {}", e);
                                    }
                                    
                                    // For 2xx responses to INVITE, store transaction ID
                                    if response.status.is_success() {
                                        if let Some((_, method)) = rvoip_transaction_core::utils::extract_cseq(&Message::Response(response)) {
                                            if method == Method::Invite {
                                                debug!("Storing transaction ID {} for 2xx response to INVITE", transaction_id);
                                                if let Err(e) = call.store_invite_transaction_id(transaction_id).await {
                                                    error!("Failed to store transaction ID: {}", e);
                                                }
                                            }
                                        }
                                    }
                                } else {
                                    debug!("No call found for call-ID {}", call_id);
                                }
                            } else {
                                debug!("Response missing Call-ID header");
                            }
                        }
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
    
    /// Create an event stream for call events
    pub fn event_stream(&self) -> mpsc::Receiver<CallEvent> {
        let (tx, rx) = mpsc::channel(100);
        
        // Subscribe to event broadcast
        let mut event_rx = self.event_broadcast.subscribe();
        
        // Forward events from the broadcast channel to the mpsc channel
        let tx_clone = tx.clone();
        tokio::spawn(async move {
            while let Ok(event) = event_rx.recv().await {
                if let Err(_) = tx_clone.send(event).await {
                    break; // Receiver dropped, stop forwarding events
                }
            }
        });
        
        // Send a Ready event to initialize the stream
        let tx1 = tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            
            if let Err(_) = tx1.send(CallEvent::Ready).await {
                // Channel closed, can ignore
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
    
    /// Get the call registry
    pub fn registry(&self) -> Arc<CallRegistry> {
        self.call_registry.clone()
    }
    
    /// Get call history
    pub async fn call_history(&self) -> HashMap<String, CallRecord> {
        self.call_registry.call_history().await
    }
    
    /// Get active calls
    pub async fn calls(&self) -> HashMap<String, Arc<Call>> {
        self.call_registry.active_calls().await
    }
    
    /// Get call by ID
    pub async fn call_by_id(&self, call_id: &str) -> Option<Arc<Call>> {
        self.call_registry.get_active_call(call_id).await
    }
    
    /// Find a call by ID in both active calls and history
    /// 
    /// This is a convenience method that delegates to the call registry's `find_call_by_id` method.
    /// It returns information about the call if found, including a record of the call and
    /// any available references to the actual call object.
    /// 
    /// # Parameters
    /// * `call_id` - The ID of the call to find
    /// 
    /// # Returns
    /// * `Some(CallLookupResult)` - If the call was found
    /// * `None` - If no call with the given ID exists
    pub async fn find_call(&self, call_id: &str) -> Option<crate::call_registry::CallLookupResult> {
        self.call_registry.find_call_by_id(call_id).await
    }
    
    /// Find a call by ID for API use, returning a serializable result
    /// 
    /// This is a convenience method for API endpoints that need to return serializable data.
    /// It works like `find_call` but returns a version that can be safely serialized.
    /// 
    /// # Parameters
    /// * `call_id` - The ID of the call to find
    /// 
    /// # Returns
    /// * `Some(SerializableCallLookupResult)` - If the call was found
    /// * `None` - If no call with the given ID exists
    pub async fn find_call_for_api(&self, call_id: &str) -> Option<crate::call_registry::SerializableCallLookupResult> {
        self.call_registry.find_call_by_id(call_id).await.map(Into::into)
    }
    
    /// Create a new outgoing call (the implementation will be filled in later)
    pub async fn create_call(&self, _remote_uri: &str) -> Result<Arc<Call>> {
        // Placeholder for creating a new outgoing call
        Err(Error::Call("Not implemented yet".into()))
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
    call_registry: Arc<CallRegistry>,
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

        // Extract From tag (IMPORTANT - extract it from the header for dialog setup)
        let from_tag = match request.headers.iter()
            .find(|h| h.name == HeaderName::From)
            .and_then(|h| h.value.as_text())
            .and_then(|v| rvoip_session_core::dialog::extract_tag(v)) {
            Some(tag) => tag,
            None => return Err(Error::SipProtocol("Missing From tag".into())),
        };

        debug!("Extracted From tag: {}", from_tag);

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

        // Set the remote tag extracted from the From header
        call.set_remote_tag(from_tag).await;

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

        // Update call state to ringing using proper transition method
        if let Err(e) = call.transition_to(CallState::Ringing).await {
            error!("Failed to update call state to Ringing: {}", e);
        } else {
            debug!("Call {} state updated to Ringing", call_id);
        }

        // Store call - important that we register the call before sending events
        // First add to active calls
        active_calls.write().await.insert(call_id.clone(), call.clone());

        // Before sending the IncomingCall event, manually register with call registry to avoid race conditions
        debug!("Registering call {} directly with registry to avoid race conditions", call_id);
        if let Err(e) = call_registry.register_call(call.clone()).await {
            error!("Failed to register call in registry: {}", e);
        }

        // Store the original INVITE request for later answering
        if let Err(e) = call.store_invite_request(request.clone()).await {
            warn!("Failed to store INVITE request: {}", e);
        } else {
            debug!("Stored original INVITE request for later answering");
        }

        // Send event - this will trigger registry update via event handler
        if let Err(e) = event_tx.send(CallEvent::IncomingCall(call.clone())).await
            .map_err(|_| Error::Call("Failed to send call event".into())) {
            error!("Failed to send IncomingCall event: {}", e);
        } else {
            debug!("Storing weak reference to call {}", call_id);
            let weak_call = call.weak_clone();
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
            
            // Store the call info for later - we don't want to answer until the IncomingCall event is processed
            // Instead of handling auto-answer here directly, let the application handle it
            // based on the IncomingCall event and the current call state
            
            return Ok(());
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
            info!("Received ACK for call {} in state {}", call_id, current_state);
            
            if current_state == CallState::Connecting {
                info!("Transitioning call {} from Connecting to Established after ACK", call_id);
                
                // Directly update the call's state to Established
                if let Err(e) = call.transition_to(CallState::Established).await {
                    warn!("Failed to update call state to Established: {}", e);
                } else {
                    info!("Call {} established successfully after ACK", call_id);
                    
                    // Check if dialog state is properly updated
                    if let Some(dialog) = call.dialog().await {
                        info!("Dialog state after ACK: {}", dialog.state);
                        if dialog.state != DialogState::Confirmed {
                            warn!("Dialog state not updated to Confirmed after ACK!");
                        }
                    } else {
                        warn!("No dialog found after ACK processing!");
                    }
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