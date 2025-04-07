use std::collections::HashMap;
use std::fmt::Debug;
use std::net::{SocketAddr, IpAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result as AnyResult;
use async_trait::async_trait;
use bytes::Bytes;
use tokio::sync::{mpsc, Mutex, RwLock, watch, oneshot, broadcast};
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
    
    /// Broadcast channel for client events
    event_broadcast: broadcast::Sender<SipClientEvent>,
    
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
        let (broadcast_tx, _) = broadcast::channel(32);  // Create broadcast channel
        
        info!("SIP client initialized with username: {}", config.username);
        
        Ok(Self {
            config,
            transaction_manager: Arc::new(transaction_manager),
            transaction_events_rx,
            event_tx,
            event_rx,
            event_broadcast: broadcast_tx,  // Add this field
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

        // Set up event processing task
        let mut transaction_events = std::mem::replace(
            &mut self.transaction_events_rx,
            mpsc::channel(1).1
        );
        let running = self.running.clone();
        let transaction_manager = self.transaction_manager.clone();
        let calls = self.calls.clone();
        let event_tx = self.event_tx.clone();
        let pending_responses = self.pending_responses.clone();
        let client_events_tx = self.event_tx.clone();

        // Create a client events channel
        let (client_events_tx, mut client_events_rx) = mpsc::channel(32);
        let broadcast_tx = self.event_broadcast.clone();

        // Create a task to forward events from client_events to broadcast
        tokio::spawn(async move {
            while let Some(event) = client_events_rx.recv().await {
                // Forward to broadcast channel
                let _ = broadcast_tx.send(event);
            }
        });

        // Process transaction events in a background task
        let event_task = tokio::spawn(async move {
            info!("SIP client event processor started");
            
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
                handle_transaction_event(
                    event,
                    transaction_manager.clone(),
                    calls.clone(),
                    event_tx.clone(),
                    pending_responses.clone(),
                ).await;
            }
            
            info!("SIP client event processor stopped");
        });
        
        self.event_task = Some(event_task);
        
        info!("SIP client started with username: {}", self.config.username);
        
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
        debug!("Making call to target URI: {}", target);
        
        // Parse target URI
        let target_uri: Uri = target.parse()
            .map_err(|e| Error::SipProtocol(format!("Invalid target URI: {}", e)))?;
        
        // Get target address
        let target_addr = self.resolve_uri(&target_uri)?;
        debug!("Resolved target address: {}", target_addr);
        
        // Generate Call-ID
        let call_id = format!("{}-{}", self.config.username, Uuid::new_v4());
        debug!("Generated Call-ID: {}", call_id);
        
        // Generate tag for From header
        let local_tag = format!("tag-{}", Uuid::new_v4());
        debug!("Generated local tag: {}", local_tag);
        
        // Create local URI
        let local_uri = format!("sip:{}@{}", self.config.username, self.config.domain).parse()
            .map_err(|e| Error::SipProtocol(format!("Invalid local URI: {}", e)))?;
        debug!("Local URI: {}", local_uri);
        
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
        debug!("Created new call with ID: {}", call.id());
        
        // Keep track of the call
        {
            let mut calls = self.calls.write().await;
            calls.insert(call.id().to_string(), call.clone());
        }
        
        // Create and send the INVITE request to initiate the call
        debug!("Creating INVITE request for call {}", call.id());
        let invite_request = call.create_invite_request().await?;
        
        // Create a transaction for the INVITE
        debug!("Creating transaction for INVITE");
        let transaction_id = self.transaction_manager.create_client_invite_transaction(
            invite_request,
            target_addr
        ).await.map_err(|e| Error::Transport(e.to_string()))?;
        
        // Send the INVITE
        debug!("Sending INVITE request for call {}", call.id());
        self.transaction_manager.send_request(&transaction_id).await
            .map_err(|e| Error::Transport(e.to_string()))?;
        
        // Update call state to Ringing
        call.update_state(CallState::Ringing).await?;
        
        Ok(call)
    }
    
    /// Get event stream for client events
    pub fn event_stream(&self) -> broadcast::Receiver<SipClientEvent> {
        // Return a new receiver subscribed to the broadcast channel
        self.event_broadcast.subscribe()
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
        use tracing::{info, debug};

        // Log the outgoing request for debugging
        debug!("Sending {} request to {}", request.method, destination);
        info!("Sending {} request to {}", request.method, destination);
        
        // Create a transaction
        let transaction_id = if request.method == Method::Invite {
            self.transaction_manager.create_client_invite_transaction(
                request,
                destination,
            ).await.map_err(|e| Error::Transport(e.to_string()))?
        } else {
            self.transaction_manager.create_client_non_invite_transaction(
                request,
                destination,
            ).await.map_err(|e| Error::Transport(e.to_string()))?
        };
        
        debug!("Created transaction: {}", transaction_id);
        
        // Create a oneshot channel to receive the response
        let (tx, rx) = oneshot::channel();
        
        // Store the channel for this transaction
        {
            let mut pending = self.pending_responses.lock().await;
            pending.insert(transaction_id.clone(), tx);
        }
        
        // Send the request
        self.transaction_manager.send_request(&transaction_id).await
            .map_err(|e| Error::Transport(e.to_string()))?;
        
        debug!("Request sent, waiting for response: {}", transaction_id);
        
        // Wait for the response with a timeout
        let timeout = tokio::time::timeout(
            Duration::from_secs(30), // Use a longer timeout for INVITE
            self.wait_for_transaction_response(&transaction_id)
        ).await;
        
        match timeout {
            Ok(result) => result,
            Err(_) => {
                error!("Transaction timed out: {}", transaction_id);
                Err(Error::Timeout("Transaction timed out".into()))
            }
        }
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
            event_broadcast: self.event_broadcast.clone(),
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
    use tracing::{debug, info, warn, error};
    
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
        info!("New incoming call from {}", source);
        
        // Create temporary response
        let mut response = Response::new(StatusCode::Trying);
        add_response_headers(&request, &mut response);
        
        // Send 100 Trying response
        if let Err(e) = transaction_manager.transport().send_message(
            Message::Response(response.clone()),
            source
        ).await {
            warn!("Failed to send 100 Trying: {}", e);
        }
        
        // Get From and To headers
        let from = request.header(&HeaderName::From)
            .ok_or_else(|| Error::SipProtocol("Missing From header".into()))?;
        
        let to = request.header(&HeaderName::To)
            .ok_or_else(|| Error::SipProtocol("Missing To header".into()))?;
        
        // Convert headers to string for extraction functions
        let from_str = from.value.to_string();
        let to_str = to.value.to_string();
        
        // Extract local and remote URIs
        let remote_uri = rvoip_session_core::dialog::extract_uri(&from_str)
            .ok_or_else(|| Error::SipProtocol("Invalid From URI".into()))?;
        
        let local_uri = rvoip_session_core::dialog::extract_uri(&to_str)
            .ok_or_else(|| Error::SipProtocol("Invalid To URI".into()))?;
            
        // Extract remote tag
        let remote_tag = rvoip_session_core::dialog::extract_tag(&from_str)
            .ok_or_else(|| Error::SipProtocol("Missing From tag".into()))?;
            
        // Generate a local tag
        let local_tag = format!("tag-{}", uuid::Uuid::new_v4());
        
        // Create a default call config
        let config = CallConfig::new().with_audio(true);
        
        // Create the call
        let (call, _state_tx) = Call::new(
            CallDirection::Incoming,
            config,
            call_id.clone(),
            local_tag.clone(),
            local_uri,
            remote_uri,
            source,
            transaction_manager.clone(),
            event_tx.clone().with_transformer(|event| SipClientEvent::Call(event)),
        );
        
        // Add the incoming request body as a dummy response so it can be used by answer()
        let dummy_ok = Response::new(StatusCode::Ok);
        call.store_last_response(dummy_ok).await;
        
        // Store call and extract SDP if present
        {
            let mut calls_write = calls.write().await;
            calls_write.insert(call.id().to_string(), call.clone());
            
            // Process SDP if present
            if !request.body.is_empty() {
                if let Ok(sdp_str) = std::str::from_utf8(&request.body) {
                    if let Ok(sdp) = rvoip_session_core::sdp::SessionDescription::parse(sdp_str) {
                        // Set up media session by storing the SDP
                        if let Err(e) = call.setup_media_from_sdp(&sdp).await {
                            error!("Failed to setup media: {}", e);
                        }
                    } else {
                        warn!("Failed to parse SDP in INVITE");
                    }
                } else {
                    warn!("Invalid UTF-8 in SDP body");
                }
            }
        }
        
        // Set the remote tag
        call.set_remote_tag(remote_tag).await;
        
        // Change call state to Ringing
        call.update_state(CallState::Ringing).await?;
        
        // Send 180 Ringing
        let mut ringing_response = Response::new(StatusCode::Ringing);
        add_response_headers(&request, &mut ringing_response);
        
        // Add To header with tag
        let to_with_tag = format!("{};tag={}", to, local_tag);
        ringing_response.headers.push(Header::text(HeaderName::To, to_with_tag));
        
        // Send 180 Ringing response
        if let Err(e) = transaction_manager.transport().send_message(
            Message::Response(ringing_response),
            source
        ).await {
            warn!("Failed to send 180 Ringing: {}", e);
        }
        
        // Send IncomingCall event
        info!("Sending IncomingCall event for call {}", call.id());
        event_tx.send(SipClientEvent::Call(CallEvent::IncomingCall(call.clone()))).await
            .map_err(|e| Error::Other(format!("Failed to send IncomingCall event: {}", e)))?;
        
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

// Handle a transaction event
async fn handle_transaction_event(
    event: TransactionEvent,
    transaction_manager: Arc<TransactionManager>,
    calls: Arc<RwLock<HashMap<String, Arc<Call>>>>,
    event_tx: mpsc::Sender<SipClientEvent>,
    pending_responses: Arc<Mutex<HashMap<String, oneshot::Sender<Response>>>>,
) {
    use tracing::{info, debug, error, warn};
    
    match event {
        TransactionEvent::TransactionCreated { transaction_id } => {
            debug!("Transaction created: {}", transaction_id);
        },
        TransactionEvent::TransactionCompleted { transaction_id, response } => {
            if let Some(response) = response {
                debug!("Transaction {} completed with response {} ({:?})", 
                    transaction_id, response.status, response.status);
                
                // Extract Call-ID to find the call
                if let Some(call_id) = response.call_id() {
                    debug!("Found Call-ID {} in response", call_id);
                    
                    // Look up the call
                    let calls_read = calls.read().await;
                    if let Some(call) = calls_read.get(call_id) {
                        debug!("Found call for Call-ID {}, handling response", call_id);
                        
                        // Handle the response
                        if let Err(e) = call.handle_response(response.clone()).await {
                            error!("Error handling response in call {}: {}", call_id, e);
                        } else {
                            debug!("Response handled successfully for call {}", call_id);
                        }
                    } else {
                        warn!("Call not found for Call-ID {}", call_id);
                    }
                    drop(calls_read);  // Explicitly release lock
                }
                
                // Also handle pending response channels
                if let Some(sender) = {
                    let mut pending = pending_responses.lock().await;
                    pending.remove(&transaction_id)
                } {
                    debug!("Sending response to pending channel for transaction {}", transaction_id);
                    let _ = sender.send(response.clone()); // Ignore errors from closed channels
                }
            } else {
                debug!("Transaction {} completed with no response", transaction_id);
            }
        },
        TransactionEvent::TransactionTerminated { transaction_id } => {
            debug!("Transaction terminated: {}", transaction_id);
        },
        TransactionEvent::UnmatchedMessage { message, source } => {
            match message {
                Message::Request(request) => {
                    debug!("Handling unmatched request: {}", request.method);
                    
                    // Extract Call-ID to find the call
                    if let Some(call_id) = request.call_id() {
                        let call_opt = {
                            let calls_read = calls.read().await;
                            calls_read.get(call_id).cloned()
                        };
                        
                        if let Some(call) = call_opt {
                            info!("Found call for unmatched request: {}", call_id);
                            
                            // Let the call handle the request
                            match call.handle_request(request).await {
                                Ok(Some(response)) => {
                                    info!("Sending response for unmatched request: {}", response.status);
                                    
                                    // Send response
                                    if let Err(e) = transaction_manager.transport().send_message(
                                        Message::Response(response),
                                        source
                                    ).await {
                                        error!("Failed to send response: {}", e);
                                    }
                                },
                                Ok(None) => {
                                    debug!("No response needed for unmatched request");
                                },
                                Err(e) => {
                                    error!("Error handling unmatched request: {}", e);
                                }
                            }
                        } else {
                            warn!("Call not found for unmatched request with Call-ID: {}", call_id);
                        }
                    } else {
                        warn!("Unmatched request has no Call-ID");
                    }
                },
                Message::Response(response) => {
                    debug!("Unmatched response: {:?}", response);
                }
            }
        },
        TransactionEvent::Error { error, transaction_id } => {
            error!("Transaction error: {}, transaction ID: {:?}", error, transaction_id);
            
            // Forward error to client events
            let _ = event_tx.send(SipClientEvent::Error(error.to_string())).await;
        }
    }
} 