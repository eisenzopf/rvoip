use std::collections::HashMap;
use std::fmt::Debug;
use std::net::{SocketAddr, IpAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result as AnyResult;
use async_trait::async_trait;
use bytes::Bytes;
use tokio::sync::{mpsc, Mutex, RwLock, watch, oneshot};
use tokio::time;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use rvoip_sip_core::{
    Request, Response, Message, Method, StatusCode, 
    Uri, Header, HeaderName, HeaderValue
};
use rvoip_sip_transport::UdpTransport;
use rvoip_transaction_core::{TransactionManager, TransactionEvent};

use crate::config::{ClientConfig, CallConfig};
use crate::error::{Error, Result};
use crate::call::{Call, CallState, CallEvent, CallDirection};

/// Event types emitted by the SIP client
#[derive(Debug, Clone)]
pub enum SipClientEvent {
    /// Call-related event
    Call(CallEvent),
    
    /// Registration state changed
    RegistrationState {
        /// Is the client registered
        registered: bool,
        
        /// Registration server
        server: String,
        
        /// Registration expiry in seconds
        expires: Option<u32>,
        
        /// Error message if registration failed
        error: Option<String>,
    },
    
    /// Client error
    Error(String),
}

/// Registration state
struct Registration {
    /// Server address
    server: SocketAddr,
    
    /// Registration URI
    uri: Uri,
    
    /// Is registered
    registered: bool,
    
    /// Registration expiry
    expires: u32,
    
    /// Last registration time
    registered_at: Option<Instant>,
    
    /// Error message if registration failed
    error: Option<String>,
    
    /// Task handle for registration refresh
    refresh_task: Option<tokio::task::JoinHandle<()>>,
}

impl Clone for Registration {
    fn clone(&self) -> Self {
        Self {
            server: self.server,
            uri: self.uri.clone(),
            registered: self.registered,
            expires: self.expires,
            registered_at: self.registered_at,
            error: self.error.clone(),
            refresh_task: None, // Don't clone the task handle
        }
    }
}

/// SIP client for managing calls and registrations
pub struct SipClient {
    /// Client configuration
    config: ClientConfig,
    
    /// Transaction manager
    transaction_manager: Arc<TransactionManager>,
    
    /// Event receiver from transaction manager
    transaction_events_rx: mpsc::Receiver<TransactionEvent>,
    
    /// Event sender for client events
    event_tx: mpsc::Sender<SipClientEvent>,
    
    /// Event receiver for client events
    event_rx: mpsc::Receiver<SipClientEvent>,
    
    /// Active calls
    calls: Arc<RwLock<HashMap<String, Arc<Call>>>>,
    
    /// CSeq counter for requests
    cseq: Arc<Mutex<u32>>,
    
    /// Registration state
    registration: Arc<RwLock<Option<Registration>>>,
    
    /// Is the client running
    running: Arc<RwLock<bool>>,
    
    /// Background task handle - not included in Clone
    #[allow(dead_code)]
    event_task: Option<tokio::task::JoinHandle<()>>,

    /// Pending transaction responses
    pending_responses: Arc<Mutex<HashMap<String, oneshot::Sender<Response>>>>,
}

impl SipClient {
    /// Create a new SIP client
    pub async fn new(config: ClientConfig) -> Result<Self> {
        // Get local address from config or use default
        let local_addr = config.local_addr
            .ok_or_else(|| Error::Configuration("Local address must be specified".into()))?;
        
        // Create UDP transport
        let (udp_transport, transport_rx) = UdpTransport::bind(local_addr, None).await
            .map_err(|e| Error::Transport(e.to_string()))?;
        
        info!("SIP client UDP transport bound to {}", local_addr);
        
        // Wrap transport in Arc
        let arc_transport = Arc::new(udp_transport);
        
        // Create transaction manager
        let (transaction_manager, transaction_events_rx) = TransactionManager::new(
            arc_transport,
            transport_rx,
            Some(config.transaction.max_events),
        ).await.map_err(|e| Error::Transport(e.to_string()))?;
        
        // Create event channels
        let (event_tx, event_rx) = mpsc::channel(32);
        
        info!("SIP client initialized with username: {}", config.username);
        
        Ok(Self {
            config,
            transaction_manager: Arc::new(transaction_manager),
            transaction_events_rx,
            event_tx,
            event_rx,
            calls: Arc::new(RwLock::new(HashMap::new())),
            cseq: Arc::new(Mutex::new(1)),
            registration: Arc::new(RwLock::new(None)),
            running: Arc::new(RwLock::new(false)),
            event_task: None,
            pending_responses: Arc::new(Mutex::new(HashMap::new())),
        })
    }
    
    /// Start the client
    pub async fn start(&mut self) -> Result<()> {
        // Check if already running
        if *self.running.read().await {
            return Ok(());
        }
        
        // Set running flag
        *self.running.write().await = true;
        
        // Start event processing task
        let mut transaction_events = std::mem::replace(&mut self.transaction_events_rx, mpsc::channel(1).1);
        let transaction_manager = self.transaction_manager.clone();
        let calls = self.calls.clone();
        let running = self.running.clone();
        let client_events_tx = self.event_tx.clone();
        let pending_responses = self.pending_responses.clone();
        
        let event_task = tokio::spawn(async move {
            debug!("SIP client event processing task started");
            
            while *running.read().await {
                // Wait for transaction event with timeout
                let event = match tokio::time::timeout(
                    Duration::from_secs(1),
                    transaction_events.recv()
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
                                    calls.clone(),
                                    client_events_tx.clone(),
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
                        debug!("Transaction completed: {}, has response: {}", 
                               transaction_id, response.is_some());
                        
                        // If there's a pending response channel, deliver the response
                        let mut pending = pending_responses.lock().await;
                        let pending_sender = pending.remove(&transaction_id);
                        
                        // Forward response to call if applicable
                        let mut call_to_update = None;
                        
                        if let Some(resp) = &response {
                            // Try to find the call this response belongs to
                            if let Some(call_id) = resp.call_id() {
                                let calls_read = calls.read().await;
                                call_to_update = calls_read.get(call_id).cloned();
                                
                                if call_to_update.is_some() {
                                    debug!("Found call {} for response to transaction {}", 
                                          call_id, transaction_id);
                                } else {
                                    debug!("No call found for ID {} from response", call_id);
                                }
                            } else {
                                debug!("Response has no Call-ID header");
                            }
                        }
                        
                        // Send the response to the waiting channel if any
                        if let Some(tx) = pending_sender {
                            if let Some(resp) = response.clone() {
                                debug!("Sending response for transaction {} to waiting task", 
                                      transaction_id);
                                
                                if tx.send(resp.clone()).is_err() {
                                    warn!("Failed to deliver response - receiver dropped");
                                }
                            } else {
                                // No response, send a timeout response
                                warn!("No response for transaction {}, sending timeout", transaction_id);
                                let timeout_response = Response::new(StatusCode::RequestTimeout);
                                let _ = tx.send(timeout_response);
                            }
                        } else {
                            debug!("No pending task waiting for transaction {}", transaction_id);
                        }
                        
                        // Must drop the pending lock before updating call state to avoid deadlocks
                        drop(pending);
                        
                        // Update call state if needed
                        if let Some(call) = call_to_update {
                            if let Some(resp) = response {
                                debug!("Forwarding response to call: {}", resp.status);
                                if let Err(e) = call.handle_response(resp).await {
                                    error!("Error handling response in call: {}", e);
                                }
                            }
                        }
                    },
                    TransactionEvent::TransactionTerminated { transaction_id } => {
                        debug!("Transaction terminated: {}", transaction_id);
                        
                        // Clean up any pending channels
                        let mut pending = pending_responses.lock().await;
                        if pending.remove(&transaction_id).is_some() {
                            debug!("Removed pending channel for terminated transaction");
                        }
                    },
                    TransactionEvent::Error { error, transaction_id } => {
                        error!("Transaction error: {}, id: {:?}", error, transaction_id);
                        
                        // If there's a pending response channel, deliver an error
                        if let Some(id) = transaction_id {
                            let mut pending = pending_responses.lock().await;
                            if let Some(tx) = pending.remove(&id) {
                                let mut error_response = Response::new(StatusCode::ServerInternalError);
                                error_response.headers.push(Header::text(HeaderName::UserAgent, "RVOIP-SIP-Client"));
                                let _ = tx.send(error_response);
                            }
                        }
                        
                        // Send client error event
                        let _ = client_events_tx.send(SipClientEvent::Error(
                            format!("Transaction error: {}", error)
                        )).await;
                    },
                }
            }
            
            debug!("SIP client event processing task ended");
        });
        
        self.event_task = Some(event_task);
        
        Ok(())
    }
    
    /// Stop the client
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
        let calls = self.calls.read().await;
        for (_, call) in calls.iter() {
            let _ = call.hangup().await;
        }
        
        // Unregister if registered
        if let Some(registration) = self.registration.read().await.clone() {
            if registration.registered {
                let _ = self.unregister().await;
            }
        }
        
        Ok(())
    }
    
    /// Register with a SIP server
    pub async fn register(&self, server_addr: SocketAddr) -> Result<()> {
        // Create request URI for REGISTER (domain)
        let request_uri: Uri = format!("sip:{}", self.config.domain).parse()
            .map_err(|e| Error::SipProtocol(format!("Invalid domain URI: {}", e)))?;
        
        // Create REGISTER request
        let mut request = self.create_request(Method::Register, request_uri.clone());
        
        // Add Expires header
        request.headers.push(Header::text(
            HeaderName::Expires, 
            self.config.register_expires.to_string()
        ));
        
        // Add Contact header with expires parameter
        let contact = format!(
            "<sip:{}@{};transport=udp>;expires={}",
            self.config.username,
            self.config.local_addr.unwrap(),
            self.config.register_expires
        );
        request.headers.push(Header::text(HeaderName::Contact, contact));
        
        // Add Content-Length
        request.headers.push(Header::text(HeaderName::ContentLength, "0"));
        
        // Store registration state
        let mut registration = Registration {
            server: server_addr,
            uri: request_uri,
            registered: false,
            expires: self.config.register_expires,
            registered_at: None,
            error: None,
            refresh_task: None,
        };
        
        // Set registration state
        *self.registration.write().await = Some(registration.clone());
        
        // Send REGISTER request via transaction
        let response = self.send_via_transaction(request, server_addr).await?;
        
        if response.status == StatusCode::Ok {
            info!("Registration successful");
            
            // Update registration state
            registration.registered = true;
            registration.registered_at = Some(Instant::now());
            registration.error = None;
            
            // Set up registration refresh
            let refresh_interval = (self.config.register_expires as f32 * self.config.register_refresh) as u64;
            
            // Create refresh task
            let client = self.clone_lightweight();
            let server = server_addr;
            let refresh_task = tokio::spawn(async move {
                // Wait for refresh interval
                tokio::time::sleep(Duration::from_secs(refresh_interval)).await;
                
                // Refresh registration
                if let Err(e) = client.register(server).await {
                    error!("Failed to refresh registration: {}", e);
                }
            });
            
            registration.refresh_task = Some(refresh_task);
            
            // Update registration
            *self.registration.write().await = Some(registration);
            
            // Send registration event
            let _ = self.event_tx.send(SipClientEvent::RegistrationState {
                registered: true,
                server: server_addr.to_string(),
                expires: Some(self.config.register_expires),
                error: None,
            }).await;
            
            Ok(())
        } else if response.status == StatusCode::Unauthorized {
            // Handle authentication in the future
            error!("Authentication required, not implemented yet");
            
            // Update registration state
            registration.registered = false;
            registration.error = Some(format!("Authentication required: {}", response.status));
            
            // Update registration
            *self.registration.write().await = Some(registration);
            
            // Send registration event
            let _ = self.event_tx.send(SipClientEvent::RegistrationState {
                registered: false,
                server: server_addr.to_string(),
                expires: None,
                error: Some("Authentication required".into()),
            }).await;
            
            Err(Error::Authentication("Authentication required".into()))
        } else {
            // Registration failed
            error!("Registration failed: {}", response.status);
            
            // Update registration state
            registration.registered = false;
            registration.error = Some(format!("Registration failed: {}", response.status));
            
            // Update registration
            *self.registration.write().await = Some(registration);
            
            // Send registration event
            let _ = self.event_tx.send(SipClientEvent::RegistrationState {
                registered: false,
                server: server_addr.to_string(),
                expires: None,
                error: Some(format!("Registration failed: {}", response.status)),
            }).await;
            
            Err(Error::Registration(format!("Registration failed: {}", response.status)))
        }
    }
    
    /// Unregister from SIP server
    pub async fn unregister(&self) -> Result<()> {
        // Get current registration
        let registration = match self.registration.read().await.clone() {
            Some(r) => r,
            None => return Err(Error::Registration("Not registered".into())),
        };
        
        // Cancel refresh task if any
        if let Some(task) = registration.refresh_task {
            task.abort();
        }
        
        // Create REGISTER request with expires=0
        let mut request = self.create_request(Method::Register, registration.uri);
        
        // Add Expires header with 0
        request.headers.push(Header::text(HeaderName::Expires, "0"));
        
        // Add Contact header with expires=0
        let contact = format!(
            "<sip:{}@{};transport=udp>;expires=0",
            self.config.username,
            self.config.local_addr.unwrap()
        );
        request.headers.push(Header::text(HeaderName::Contact, contact));
        
        // Add Content-Length
        request.headers.push(Header::text(HeaderName::ContentLength, "0"));
        
        // Send REGISTER request via transaction
        let response = self.send_via_transaction(request, registration.server).await?;
        
        if response.status == StatusCode::Ok {
            info!("Unregistration successful");
            
            // Clear registration state
            *self.registration.write().await = None;
            
            // Send registration event
            let _ = self.event_tx.send(SipClientEvent::RegistrationState {
                registered: false,
                server: registration.server.to_string(),
                expires: None,
                error: None,
            }).await;
            
            Ok(())
        } else {
            // Unregistration failed
            error!("Unregistration failed: {}", response.status);
            
            // Send registration event
            let _ = self.event_tx.send(SipClientEvent::RegistrationState {
                registered: true, // Still registered
                server: registration.server.to_string(),
                expires: Some(registration.expires),
                error: Some(format!("Unregistration failed: {}", response.status)),
            }).await;
            
            Err(Error::Registration(format!("Unregistration failed: {}", response.status)))
        }
    }
    
    /// Make a call to a target URI
    pub async fn call(&self, target: &str, config: CallConfig) -> Result<Arc<Call>> {
        // Parse target URI
        let target_uri: Uri = target.parse()
            .map_err(|e| Error::SipProtocol(format!("Invalid target URI: {}", e)))?;
        
        // Get target address
        let target_addr = self.resolve_uri(&target_uri)?;
        
        // Generate Call-ID
        let call_id = format!("{}-{}", self.config.username, Uuid::new_v4());
        
        // Generate tag for From header
        let local_tag = format!("tag-{}", Uuid::new_v4());
        
        // Create local URI
        let local_uri = format!("sip:{}@{}", self.config.username, self.config.domain).parse()
            .map_err(|e| Error::SipProtocol(format!("Invalid local URI: {}", e)))?;
        
        // Create call
        let (call, state_tx) = Call::new(
            CallDirection::Outgoing,
            config,
            call_id.clone(),
            local_tag,
            local_uri,
            target_uri.clone(),
            target_addr,
            self.transaction_manager.clone(),
            self.event_tx.clone().with_transformer(|event| SipClientEvent::Call(event)),
        );
        
        // Store call
        self.calls.write().await.insert(call_id.clone(), call.clone());
        
        // Create INVITE request
        let mut request = self.create_request(Method::Invite, target_uri);
        
        // Set up media and SDP
        // Choose a local RTP port for audio
        let local_rtp_port = self.config.media.rtp_port_min + 
            (rand::random::<u16>() % (self.config.media.rtp_port_max - self.config.media.rtp_port_min));
        
        // Generate SDP using the session-core implementation
        use rvoip_session_core::sdp::SessionDescription;
        
        // Create SDP for audio call
        let sdp = SessionDescription::new_audio_call(
            &self.config.username,
            self.config.local_addr.unwrap().ip(),
            local_rtp_port
        );
        
        // Convert SDP to string and then to bytes
        let sdp_str = sdp.to_string();
        request.body = bytes::Bytes::from(sdp_str);
        
        // Add Content-Type and Content-Length headers
        request.headers.push(Header::text(HeaderName::ContentType, "application/sdp"));
        request.headers.push(Header::integer(HeaderName::ContentLength, request.body.len() as i64));
        
        // Send INVITE request via transaction
        let response = self.send_via_transaction(request, target_addr).await?;
        
        // Update call state based on response
        if response.status.is_success() {
            // Call answered
            state_tx.send(CallState::Established)
                .map_err(|_| Error::Call("Failed to update call state".into()))?;
        } else if response.status == StatusCode::Ringing || response.status == StatusCode::SessionProgress {
            // Call ringing
            state_tx.send(CallState::Ringing)
                .map_err(|_| Error::Call("Failed to update call state".into()))?;
        } else {
            // Call failed - update to Terminated with error status
            state_tx.send(CallState::Terminated)
                .map_err(|_| Error::Call("Failed to update call state".into()))?;
            
            // Send terminated event
            let _ = self.event_tx.send(SipClientEvent::Call(CallEvent::Terminated {
                call: call.clone(),
                reason: format!("Failed with status: {}", response.status),
            })).await;
        }
        
        Ok(call)
    }
    
    /// Get event stream for client events
    pub fn event_stream(&self) -> mpsc::Receiver<SipClientEvent> {
        // Create a new channel for the caller
        let (tx, rx) = mpsc::channel(32);
        
        // Clone our event sender
        let event_tx = self.event_tx.clone(); 
        
        // Spawn task to forward events to the new channel
        tokio::spawn(async move {
            // Create a separate channel to receive events
            let (_forward_tx, mut forward_rx) = mpsc::channel(32);
            
            // Use event_tx directly without assigning to unused variable
            
            // Forward all incoming events to the user's channel
            while let Some(event) = forward_rx.recv().await {
                if tx.send(event).await.is_err() {
                    break;
                }
            }
        });
        
        rx
    }
    
    /// Get active calls
    pub async fn calls(&self) -> HashMap<String, Arc<Call>> {
        self.calls.read().await.clone()
    }
    
    /// Get call by ID
    pub async fn call_by_id(&self, call_id: &str) -> Option<Arc<Call>> {
        self.calls.read().await.get(call_id).cloned()
    }
    
    /// Get registration state
    pub async fn registration_state(&self) -> Option<(String, bool, Option<u32>)> {
        let registration = self.registration.read().await;
        registration.as_ref().map(|r| (
            r.server.to_string(),
            r.registered,
            if r.registered { Some(r.expires) } else { None }
        ))
    }
    
    /// Run the client event loop
    pub async fn run(&mut self) -> Result<()> {
        // Start the client if not already running
        if !*self.running.read().await {
            self.start().await?;
        }
        
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
    
    /// Create a new SIP request
    fn create_request(&self, method: Method, uri: Uri) -> Request {
        let mut request = Request::new(method.clone(), uri.clone());
        
        // Add Via header with branch parameter
        let branch = format!("z9hG4bK-{}", Uuid::new_v4());
        let via_value = format!(
            "SIP/2.0/UDP {};branch={}",
            self.config.local_addr.unwrap(),
            branch
        );
        request.headers.push(Header::text(HeaderName::Via, via_value));
        
        // Add Max-Forwards
        request.headers.push(Header::integer(HeaderName::MaxForwards, 70));
        
        // Add From header with tag
        let from_tag = format!("tag-{}", Uuid::new_v4());
        let from_value = format!(
            "<sip:{}@{}>;tag={}",
            self.config.username,
            self.config.domain,
            from_tag
        );
        request.headers.push(Header::text(HeaderName::From, from_value));
        
        // Add To header
        let to_value = format!("<{}>", uri);
        request.headers.push(Header::text(HeaderName::To, to_value));
        
        // Add Call-ID
        let call_id = format!("{}-{}", self.config.username, Uuid::new_v4());
        request.headers.push(Header::text(HeaderName::CallId, call_id));
        
        // Add CSeq
        let cseq = self.next_cseq();
        request.headers.push(Header::text(
            HeaderName::CSeq,
            format!("{} {}", cseq, method)
        ));
        
        // Add Contact
        let contact_value = format!(
            "<sip:{}@{};transport=udp>",
            self.config.username,
            self.config.local_addr.unwrap()
        );
        request.headers.push(Header::text(HeaderName::Contact, contact_value));
        
        // Add User-Agent
        request.headers.push(Header::text(
            HeaderName::UserAgent,
            self.config.user_agent.clone()
        ));
        
        request
    }
    
    /// Get next CSeq value
    fn next_cseq(&self) -> u32 {
        let mut cseq = self.cseq.try_lock().unwrap();
        let value = *cseq;
        *cseq += 1;
        value
    }
    
    /// Resolve a URI to a socket address
    fn resolve_uri(&self, uri: &Uri) -> Result<SocketAddr> {
        // Determine host and port
        let host = &uri.host;
        let port = uri.port.unwrap_or(5060);
        
        match host.parse::<IpAddr>() {
            Ok(ip) => {
                // Direct IP address
                Ok(SocketAddr::new(ip, port))
            },
            Err(_) => {
                // Handle special case for local development
                if host == "rvoip.local" || host == "localhost" || host == &self.config.domain {
                    // Use outbound proxy if specified
                    if let Some(proxy) = self.config.outbound_proxy {
                        return Ok(proxy);
                    }
                    
                    // If no proxy defined but we have a local address, use that with the specified port
                    if let Some(local_addr) = self.config.local_addr {
                        return Ok(SocketAddr::new(local_addr.ip(), port));
                    }
                }
                
                // TODO: Implement DNS SRV lookup for production environments
                
                // For development, return a helpful error
                Err(Error::Transport(format!("Could not resolve URI: {}. For local development, use IP addresses directly or update /etc/hosts.", uri)))
            }
        }
    }
    
    /// Create a lightweight clone (for use in closures and tasks)
    fn clone_lightweight(&self) -> LightweightClient {
        LightweightClient {
            transaction_manager: self.transaction_manager.clone(),
            config: self.config.clone(),
            cseq: self.cseq.clone(),
            registration: self.registration.clone(),
            event_tx: self.event_tx.clone(),
        }
    }
    
    /// Send a request via the transaction layer
    async fn send_via_transaction(&self, request: Request, destination: SocketAddr) -> Result<Response> {
        let method = request.method.clone();
        
        // Create appropriate client transaction based on request method
        let transaction_id = if method == Method::Invite {
            // Use INVITE transaction
            self.transaction_manager.create_client_invite_transaction(
                request,
                destination,
            ).await.map_err(|e| Error::Transport(e.to_string()))?
        } else {
            // Use non-INVITE transaction
            self.transaction_manager.create_client_non_invite_transaction(
                request,
                destination,
            ).await.map_err(|e| Error::Transport(e.to_string()))?
        };
        
        // Send request
        self.transaction_manager.send_request(&transaction_id).await
            .map_err(|e| Error::Transport(e.to_string()))?;
        
        // Wait for final response
        let response = match method {
            Method::Invite => {
                // For INVITE, wait for a provisional response first
                // TODO: Implement this logic
                // For now, just wait for final response
                self.wait_for_transaction_response(&transaction_id).await?
            },
            _ => {
                // For non-INVITE, just wait for final response
                self.wait_for_transaction_response(&transaction_id).await?
            }
        };
        
        Ok(response)
    }
    
    /// Wait for transaction response
    async fn wait_for_transaction_response(&self, transaction_id: &str) -> Result<Response> {
        debug!("Waiting for response to transaction: {}", transaction_id);
        
        // Create a new channel for this specific wait
        let (tx, rx) = oneshot::channel::<Response>();
        
        // Store the transaction and channel in pending_responses
        {
            let mut pending = self.pending_responses.lock().await;
            pending.insert(transaction_id.to_string(), tx);
            debug!("Added transaction {} to pending_responses map", transaction_id);
        }
        
        // Wait for the response with a timeout
        match tokio::time::timeout(Duration::from_secs(32), rx).await {
            Ok(Ok(response)) => {
                debug!("Received response for transaction {}: {}", transaction_id, response.status);
                Ok(response)
            },
            Ok(Err(_)) => {
                warn!("Response channel for transaction {} was closed", transaction_id);
                Err(Error::Transport("Transaction channel closed".into()))
            },
            Err(_) => {
                warn!("Timeout waiting for response to transaction {}", transaction_id);
                // Clean up the pending entry
                let mut pending = self.pending_responses.lock().await;
                pending.remove(transaction_id);
                Err(Error::Transport("Timeout waiting for response".into()))
            }
        }
    }
}

/// Lightweight clone of SIP client for use in tasks
#[derive(Clone)]
struct LightweightClient {
    transaction_manager: Arc<TransactionManager>,
    config: ClientConfig,
    cseq: Arc<Mutex<u32>>,
    registration: Arc<RwLock<Option<Registration>>>,
    event_tx: mpsc::Sender<SipClientEvent>,
}

impl LightweightClient {
    /// Register with a SIP server (simplified version)
    pub async fn register(&self, server_addr: SocketAddr) -> Result<()> {
        // Create request URI for REGISTER (domain)
        let request_uri: Uri = format!("sip:{}", self.config.domain).parse()
            .map_err(|e| Error::SipProtocol(format!("Invalid domain URI: {}", e)))?;
        
        // Create REGISTER request
        let mut request = Request::new(Method::Register, request_uri.clone());
        
        // Add common headers
        request.headers.push(Header::text(HeaderName::From, 
            format!("<sip:{}@{}>", self.config.username, self.config.domain)));
        request.headers.push(Header::text(HeaderName::To, 
            format!("<sip:{}@{}>", self.config.username, self.config.domain)));
        request.headers.push(Header::text(HeaderName::CallId, 
            format!("{}-register-{}", self.config.username, Uuid::new_v4())));
        
        let cseq = {
            let mut lock = self.cseq.lock().await;
            let current = *lock;
            *lock += 1;
            current
        };
        
        request.headers.push(Header::text(HeaderName::CSeq, 
            format!("{} {}", cseq, Method::Register)));
        
        // Add Via header
        request.headers.push(Header::text(HeaderName::Via, 
            format!("SIP/2.0/UDP {};branch=z9hG4bK-{}", 
                self.config.local_addr.unwrap(),
                Uuid::new_v4().to_string().split('-').next().unwrap()
            )));
            
        // Add Max-Forwards header
        request.headers.push(Header::text(HeaderName::MaxForwards, "70"));
        
        // Add Expires header
        request.headers.push(Header::text(
            HeaderName::Expires, 
            self.config.register_expires.to_string()
        ));
        
        // Add Contact header with expires parameter
        let contact = format!(
            "<sip:{}@{};transport=udp>;expires={}",
            self.config.username,
            self.config.local_addr.unwrap(),
            self.config.register_expires
        );
        request.headers.push(Header::text(HeaderName::Contact, contact));
        
        // Add Content-Length
        request.headers.push(Header::text(HeaderName::ContentLength, "0"));
        
        // Create a client transaction for the request
        let transaction_id = if request.method == Method::Invite {
            self.transaction_manager.create_client_invite_transaction(
                request,
                server_addr,
            ).await.map_err(|e| Error::Transport(e.to_string()))?
        } else {
            self.transaction_manager.create_client_non_invite_transaction(
                request,
                server_addr,
            ).await.map_err(|e| Error::Transport(e.to_string()))?
        };
        
        // Send the request
        self.transaction_manager.send_request(&transaction_id).await
            .map_err(|e| Error::Transport(e.to_string()))?;
        
        // For a complete implementation, we would wait for the response
        // using a proper response handling mechanism.
        // For now, we just log that the request was sent.
        debug!("REGISTER request sent with transaction ID: {}", transaction_id);
        
        Ok(())
    }
}

impl Clone for SipClient {
    fn clone(&self) -> Self {
        // Create a new transaction events channel
        let transaction_events_rx = self.transaction_manager.subscribe();
        
        // Create a new event channel
        let (event_tx, event_rx) = mpsc::channel(32);
        
        Self {
            config: self.config.clone(),
            transaction_manager: self.transaction_manager.clone(),
            transaction_events_rx,
            event_tx,
            event_rx,
            calls: self.calls.clone(),
            cseq: self.cseq.clone(),
            registration: self.registration.clone(),
            running: self.running.clone(),
            event_task: None,
            pending_responses: self.pending_responses.clone(),
        }
    }
}

/// Extension trait to transform channel events
trait ChannelTransformer<T, U> {
    fn with_transformer<F>(self, f: F) -> mpsc::Sender<T>
    where
        F: Fn(T) -> U + Send + 'static,
        T: Send + 'static,
        U: Send + 'static;
}

impl<T, U> ChannelTransformer<T, U> for mpsc::Sender<U>
where
    T: Send + 'static,
    U: Send + 'static,
{
    fn with_transformer<F>(self, f: F) -> mpsc::Sender<T>
    where
        F: Fn(T) -> U + Send + 'static,
    {
        let (tx, mut rx) = mpsc::channel(32);
        let sender = self.clone();
        
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                let transformed = f(event);
                if sender.send(transformed).await.is_err() {
                    break;
                }
            }
        });
        
        tx
    }
}

/// Handle an incoming SIP request
async fn handle_incoming_request(
    request: Request,
    source: SocketAddr,
    transaction_manager: Arc<TransactionManager>,
    calls: Arc<RwLock<HashMap<String, Arc<Call>>>>,
    event_tx: mpsc::Sender<SipClientEvent>,
) -> Result<()> {
    debug!("Handling incoming {} request from {}", request.method, source);
    
    // Extract Call-ID
    let call_id = match request.call_id() {
        Some(id) => id.to_string(),
        None => return Err(Error::SipProtocol("Request missing Call-ID".into())),
    };
    
    // Check for existing call
    let calls_read = calls.read().await;
    let existing_call = calls_read.get(&call_id).cloned();
    drop(calls_read);
    
    // Handle INVITE request - new incoming call
    if request.method == Method::Invite && existing_call.is_none() {
        // TODO: Create a new call and send IncomingCall event
        debug!("New incoming call from {}", source);
        
        // Create temporary response
        let mut response = Response::new(StatusCode::Trying);
        add_response_headers(&request, &mut response);
        
        // Send response
        if let Err(e) = transaction_manager.transport().send_message(
            Message::Response(response),
            source
        ).await {
            warn!("Failed to send 100 Trying: {}", e);
        }
        
        // TODO: Create the call and send event
        
        return Ok(());
    }
    
    // Handle request for existing call
    if let Some(call) = existing_call {
        // Let the call handle it
        return match call.handle_request(request).await? {
            Some(response) => {
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
    
    // No matching call, reject with 481 Call/Transaction Does Not Exist
    if request.method != Method::Ack {
        let mut response = Response::new(StatusCode::CallOrTransactionDoesNotExist);
        add_response_headers(&request, &mut response);
        
        // Send response
        transaction_manager.transport().send_message(
            Message::Response(response),
            source
        ).await.map_err(|e| Error::Transport(e.to_string()))?;
    }
    
    Ok(())
}

/// Add common headers to a response based on a request
fn add_response_headers(request: &Request, response: &mut Response) {
    // Copy headers from request
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
} 