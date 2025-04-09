// WARNING: This file is deprecated and will be removed in a future version.
// The code has been restructured to more manageable modules in the client/ directory.
// Please update your imports to use the new module structure.

// Re-export from the new module structure for backward compatibility

pub use crate::client::SipClientEvent;
pub use crate::client::SipClient;

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
use tracing::{debug, error, info, warn, trace};
use uuid::Uuid;

use rvoip_sip_core::{
    Request, Response, Message, Method, StatusCode, 
    Uri, Header, HeaderName, HeaderValue
};
use rvoip_sip_transport::{Transport, UdpTransport};
use rvoip_transaction_core::{TransactionManager, TransactionEvent};
use rvoip_session_core::sdp::SessionDescription;

use crate::config::{ClientConfig, CallConfig};
use crate::error::{Error, Result};
use crate::call::{Call, CallState, CallEvent, CallDirection};
use crate::call_registry;

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
    /// Configuration for the client
    config: ClientConfig,
    
    /// Transport to use
    transport: Arc<dyn Transport>,
    
    /// Transaction manager
    transaction_manager: Arc<TransactionManager>,
    
    /// Transaction events receiver
    transaction_events_rx: mpsc::Receiver<TransactionEvent>,
    
    /// Active calls
    calls: Mutex<HashMap<String, Arc<RwLock<Call>>>>,
    
    /// Event sender
    event_tx: mpsc::Sender<SipClientEvent>,
    
    /// Event receiver
    event_rx: Option<mpsc::Receiver<SipClientEvent>>,
    
    /// Event broadcast channel
    event_broadcast: broadcast::Sender<SipClientEvent>,
    
    /// CSeq counter for requests
    cseq: Arc<Mutex<u32>>,
    
    /// Running flag
    running: Arc<RwLock<bool>>,
    
    /// Registration state
    registration: Arc<RwLock<Option<Registration>>>,
    
    /// Event processing task
    event_task: Option<tokio::task::JoinHandle<()>>,
    
    /// Pending response handlers
    pending_responses: Arc<Mutex<HashMap<String, oneshot::Sender<Response>>>>,
    
    /// Call registry for persistence
    registry: Option<Arc<call_registry::CallRegistry>>,
}

impl SipClient {
    /// Create a new SIP client
    pub async fn new(config: ClientConfig) -> Result<Self> {
        // Get local address from config or use default
        let local_addr = config.local_addr
            .ok_or_else(|| Error::Configuration("Local address must be specified".into()))?;
        
        let (udp_transport, transport_rx) = UdpTransport::bind(local_addr, None).await
            .map_err(|e| Error::Transport(e.to_string()))?;
        
        info!("SIP client UDP transport bound to {}", local_addr);
        
        // Create transaction manager
        let arc_transport = Arc::new(udp_transport as UdpTransport);
        
        // Create transaction manager
        let (transaction_manager, transaction_events_rx) = TransactionManager::new(
            arc_transport.clone(),
            transport_rx,
            Some(config.transaction.max_events),
        ).await.map_err(|e| Error::Transport(e.to_string()))?;
        
        // Create event channels
        let (event_tx, event_rx) = mpsc::channel(32);
        let (broadcast_tx, _) = broadcast::channel(32);  // Create broadcast channel
        
        info!("SIP client initialized with username: {}", config.username);
        
        Ok(Self {
            config,
            transport: arc_transport,
            transaction_manager: Arc::new(transaction_manager),
            transaction_events_rx,
            event_tx,
            event_rx: Some(event_rx),
            event_broadcast: broadcast_tx,
            calls: Mutex::new(HashMap::new()),
            cseq: Arc::new(Mutex::new(1)),
            registration: Arc::new(RwLock::new(None)),
            running: Arc::new(RwLock::new(false)),
            event_task: None,
            pending_responses: Arc::new(Mutex::new(HashMap::new())),
            registry: None,
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
        
        // Store references for event handling
        let transaction_manager = self.transaction_manager.clone();
        let mut transaction_events_rx = std::mem::replace(&mut self.transaction_events_rx, self.transaction_manager.subscribe());
        let event_tx = self.event_tx.clone();
        let running = self.running.clone();
        
        // Spawn event processor task
        let event_task = tokio::spawn(async move {
            info!("SIP client event processor started");
            
            while *running.read().await {
                // Wait for next transaction event
                let event = match tokio::time::timeout(
                    Duration::from_secs(1),
                    transaction_events_rx.recv()
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
                
                // Process transaction event without passing calls and pending_responses
                handle_transaction_event(event, transaction_manager.clone(), event_tx.clone()).await;
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
        
        // Cancel event task if running
        if let Some(task) = self.event_task.take() {
            task.abort();
        }
        
        // Hangup all active calls
        let calls = self.calls.lock().await;
        for (_, call_lock) in calls.iter() {
            let call_lock_clone = call_lock.clone();
            // Need to get a write lock on the Call to use hangup
            tokio::spawn(async move {
                let mut call = call_lock_clone.write().await;
                if let Err(e) = call.hangup().await {
                    error!("Error hanging up call: {}", e);
                }
            });
        }
        
        info!("SIP client stopped");
        
        Ok(())
    }
    
    /// Register with a SIP server
    pub async fn register(&self, server_addr: SocketAddr) -> Result<()> {
        // Create request URI for REGISTER (domain)
        let request_uri: Uri = format!("sip:{}", self.config.domain).parse()
            .map_err(|e| Error::SipProtocol(format!("Invalid domain URI: {}", e)))?;
        
        // Create REGISTER request
        let mut request = self.create_request(Method::Register, request_uri.clone()).await?;
        
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
        let registration = self.registration.read().await.clone()
            .ok_or_else(|| Error::Registration("No active registration".into()))?;
        
        // Cancel refresh task if any
        if let Some(task) = &registration.refresh_task {
            task.abort();
        }
        
        // Create REGISTER request with expires=0
        let mut request = self.create_request(Method::Register, registration.uri).await?;
        
        // Add zero Expires header
        request.headers.push(Header::text(HeaderName::Expires, "0"));
        
        // Send request via transaction
        let transaction_id = self.transaction_manager.create_client_transaction(
            request,
            registration.server,
        ).await.map_err(|e| Error::Transport(e.to_string()))?;
        
        debug!("Created unregister transaction: {}", transaction_id);
        
        // Send via transaction manager
        self.transaction_manager.send_request(&transaction_id).await
            .map_err(|e| Error::Transport(e.to_string()))?;
        
        // Clear registration
        let mut reg_write = self.registration.write().await;
        *reg_write = None;
        
        info!("Unregistered from {} successfully", registration.server);
        
        Ok(())
    }
    
    /// Make an outgoing call to a target URI
    pub async fn call_to(&self, target_uri: Uri, remote_addr: SocketAddr) -> Result<Arc<RwLock<Call>>> {
        info!("Call to {} is not fully implemented yet", target_uri);
        
        // Create a proper error to inform the user
        Err(Error::Client("The call_to method is not fully implemented yet. A more complete implementation will be provided in a future update.".into()))
    }
    
    /// Get event stream for client events
    pub fn event_stream(&self) -> broadcast::Receiver<SipClientEvent> {
        // Return a new receiver subscribed to the broadcast channel
        self.event_broadcast.subscribe()
    }
    
    /// Get all active calls
    pub async fn calls(&self) -> HashMap<String, Arc<RwLock<Call>>> {
        self.calls.lock().await.clone()
    }
    
    /// Get call by ID
    pub async fn call_by_id(&self, call_id: &str) -> Option<Arc<RwLock<Call>>> {
        self.calls.lock().await.get(call_id).cloned()
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
    
    /// Create a SIP request with common headers
    pub async fn create_request(&self, method: Method, target_uri: Uri) -> Result<Request> {
        debug!("Creating {} request to {}", method, target_uri);
        
        // Get next CSeq value
        let cseq_value = self.next_cseq().await;
        
        // Create request with basic headers
        let method_clone = method.clone();
        let target_uri_clone = target_uri.clone();
        let mut request = Request::new(method_clone, target_uri_clone);
        
        // Add Via header with branch parameter
        let branch = format!("z9hG4bK-{}", Uuid::new_v4().to_string());
        let via_value = format!("SIP/2.0/UDP {};branch={}", self.config.local_addr.unwrap(), branch);
        request.headers.push(Header::text(HeaderName::Via, via_value));
        
        // Add From header with tag
        let tag = format!("{}", Uuid::new_v4().to_string().split_at(8).0);
        let from_value = format!("<sip:{}@{}>", self.config.username, self.config.domain);
        let from_value_tagged = format!("{};tag={}", from_value, tag);
        request.headers.push(Header::text(HeaderName::From, from_value_tagged));
        
        // Add To header
        let to_value = format!("<{}>", target_uri);
        request.headers.push(Header::text(HeaderName::To, to_value));
        
        // Add Call-ID header
        let call_id = format!("{}", Uuid::new_v4().to_string());
        request.headers.push(Header::text(HeaderName::CallId, call_id));
        
        // Add CSeq
        request.headers.push(Header::text(
            HeaderName::CSeq,
            format!("{} {}", cseq_value, method)
        ));
        
        // Add Max-Forwards
        request.headers.push(Header::text(HeaderName::MaxForwards, "70"));
        
        // Add User-Agent
        request.headers.push(Header::text(
            HeaderName::UserAgent,
            self.config.user_agent.clone()
        ));
        
        // Add Content-Length
        request.headers.push(Header::text(HeaderName::ContentLength, "0"));
        
        Ok(request)
    }
    
    /// Get next CSeq value
    async fn next_cseq(&self) -> u32 {
        let mut cseq = self.cseq.lock().await;
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
        
        // Create a transaction using the unified method
        let transaction_id = self.transaction_manager.create_client_transaction(
            request,
            destination,
        ).await.map_err(|e| Error::Transport(e.to_string()))?;
        
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

    /// Set the call registry for call persistence
    pub async fn set_call_registry(&mut self, registry: call_registry::CallRegistry) {
        // Create an Arc for the registry
        let arc_registry = Arc::new(registry);
        
        // Check if we're replacing an existing registry
        let replacing = self.registry.is_some();
        if replacing {
            debug!("Replacing existing call registry");
        } else {
            debug!("Setting new call registry");
        }
        
        // Set the registry
        self.registry = Some(arc_registry.clone());
        
        // Update all existing calls
        let calls = self.calls.lock().await;
        for (_, call) in calls.iter() {
            let call_clone = call.clone();
            let registry_clone = arc_registry.clone();
            tokio::spawn(async move {
                let mut call_lock = call_clone.write().await;
                call_lock.set_registry(registry_clone).await;
            });
        }
        
        info!("Call registry set for SIP client");
    }

    /// Get the client's event broadcast channel
    pub fn subscribe(&self) -> broadcast::Receiver<SipClientEvent> {
        self.event_broadcast.subscribe()
    }

    /// Make a call to a specific URI with given configuration
    pub async fn call(&self, target_uri: &str, config: CallConfig) -> Result<Arc<Call>> {
        use tracing::{info, debug};
        
        debug!("Making call to {}", target_uri);
        
        // Parse target URI
        let target_uri = Uri::from_str(target_uri)
            .map_err(|e| Error::Client(format!("Invalid target URI: {}", e)))?;
        
        // Resolve target URI to address
        let remote_addr = self.resolve_uri(&target_uri)?;
        
        // Make the call
        let call = self.call_to(target_uri, remote_addr, config).await?;
        Ok(call)
    }

    /// Make a call to a specific URI and address with given configuration
    pub async fn call_to(&self, target_uri: Uri, remote_addr: SocketAddr, config: CallConfig) -> Result<Arc<Call>> {
        use tracing::{info, debug};
        
        debug!("Making call to {} at {}", target_uri, remote_addr);
        
        // Generate a unique ID for this call
        let call_id = format!("call-{}", Uuid::new_v4());
        
        // Generate a random tag for this call
        let local_tag = format!("tag-{}", Uuid::new_v4());
        
        // Get local URI
        let local_uri = Uri::from_str(&format!("sip:{}@{}", 
            self.config.username, self.config.domain))
            .map_err(|e| Error::Client(format!("Failed to create local URI: {}", e)))?;
        
        // Create a new call
        let call = Call::new(
            CallDirection::Outgoing,
            config.clone(),
            call_id.clone(),
            local_tag,
            local_uri.clone(),
            target_uri.clone(),
            remote_addr,
            self.transaction_manager.clone(),
            self.event_tx.clone(),
        ).0;
        
        // Store the call
        let mut calls = self.calls.lock().await;
        calls.insert(call_id.clone(), Arc::new(RwLock::new(call.clone())));
        drop(calls);
        
        // If we have a registry, associate it with the call
        if let Some(registry) = &self.registry {
            call.set_registry(registry.clone()).await;
        }
        
        // Start the call
        let mut invite_req = call.create_invite_request().await?;
        
        // Create INVITE transaction
        let transaction_id = uuid::Uuid::new_v4().to_string();
        
        // Log this transaction to the call log
        log_transaction(
            &call_id,
            &transaction_id,
            "invite",
            CallDirection::Outgoing,
            "created",
            Some("Initial INVITE created".to_string()),
            &self.calls
        ).await;
        
        // Send via transaction
        let response = match self.send_via_transaction(invite_req.clone(), remote_addr).await {
            Ok(response) => response,
            Err(e) => {
                error!("Failed to send INVITE: {}", e);
                call.update_state(CallState::Failed).await?;
                return Err(Error::Call(format!("Failed to send INVITE: {}", e)));
            }
        };
        
        info!("INVITE sent, initial response: {}", response.status);
        
        // Store original INVITE for later use (ACK, etc.)
        call.store_invite_request(invite_req).await?;
        
        // Store the last response
        call.store_last_response(response.clone()).await?;
        
        // Process the response
        call.handle_response(&response).await?;
        
        Ok(call)
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
        
        // Create a client transaction for the request using the unified method
        let transaction_id = self.transaction_manager.create_client_transaction(
            request,
            server_addr,
        ).await.map_err(|e| Error::Transport(e.to_string()))?;
        
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
            transport: self.transport.clone(),
            transaction_manager: self.transaction_manager.clone(),
            transaction_events_rx,
            event_tx,
            event_rx: Some(event_rx),
            event_broadcast: self.event_broadcast.clone(),
            calls: Mutex::new(HashMap::new()),
            cseq: Arc::new(Mutex::new(1)),
            registration: self.registration.clone(),
            running: self.running.clone(),
            event_task: None,
            pending_responses: Arc::new(Mutex::new(HashMap::new())),
            registry: self.registry.clone(),
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

/// Handle a transaction event
async fn handle_transaction_event(
    event: TransactionEvent,
    transaction_manager: Arc<TransactionManager>,
    event_tx: mpsc::Sender<SipClientEvent>,
) -> Result<()> {
    use tracing::{info, debug, error, warn};
    
    // Currently, this is a simplified placeholder implementation
    match event {
        TransactionEvent::Completed { transaction_id, .. } => {
            debug!("Transaction completed: {}", transaction_id);
        },
        TransactionEvent::Terminated { transaction_id, .. } => {
            debug!("Transaction terminated: {}", transaction_id);
        },
        TransactionEvent::Error { transaction_id, error } => {
            error!("Transaction error: {}: {}", transaction_id, error);
            
            // Send error event to client
            let _ = event_tx.send(SipClientEvent::Error(
                format!("Transaction error: {}: {}", transaction_id, error)
            )).await;
        },
        TransactionEvent::NewClientTransaction { .. } => {
            debug!("New client transaction");
        },
        TransactionEvent::NewServerTransaction { .. } => {
            debug!("New server transaction");
        },
        _ => {
            debug!("Unhandled transaction event: {:?}", event);
        }
    }
    
    Ok(())
}

/// Handle a new incoming call from an INVITE request
async fn handle_new_call(
    request: Request,
    source: SocketAddr,
    transaction_manager: Arc<TransactionManager>,
    calls: Arc<Mutex<HashMap<String, Arc<Call>>>>,
    event_tx: mpsc::Sender<SipClientEvent>,
) -> Result<()> {
    use tracing::{debug, info, warn, error};
    
    debug!("Handling new incoming call from {}", source);
    
    // Extract Call-ID
    let call_id = match request.call_id() {
        Some(id) => id.to_string(),
        None => return Err(Error::SipProtocol("Request missing Call-ID".into())),
    };
    
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
    
    // Store the original INVITE request for later answering
    if let Err(e) = call.store_invite_request(request.clone()).await {
        error!("Failed to store INVITE request: {}", e);
    }
    
    // Add the incoming request body as a dummy response so it can be used by answer()
    let dummy_ok = Response::new(StatusCode::Ok);
    call.store_last_response(dummy_ok).await;
    
    // Store call and extract SDP if present
    {
        let mut calls_write = calls.lock().await;
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
    let to_with_tag = format!("{};tag={}", to_str, local_tag);
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
    if let Err(e) = event_tx.send(SipClientEvent::Call(CallEvent::IncomingCall(call.clone()))).await {
        error!("Failed to send IncomingCall event: {}", e);
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

/// Log a transaction to the call registry
async fn log_transaction(
    call_id: &str, 
    transaction_id: &str,
    transaction_type: &str,
    direction: CallDirection,
    status: &str,
    info: Option<String>,
    calls: &Arc<Mutex<HashMap<String, Arc<Call>>>>,
) {
    use tracing::{debug, error};
    
    debug!("Logging transaction {} for call {}", transaction_id, call_id);
    
    // Create transaction record
    let transaction = crate::call_registry::TransactionRecord {
        transaction_id: transaction_id.to_string(),
        transaction_type: transaction_type.to_string(),
        timestamp: std::time::SystemTime::now(),
        direction,
        status: status.to_string(),
        info,
        destination: None,
    };
    
    // Find the call
    let calls_read = calls.lock().await;
    if let Some(call) = calls_read.get(call_id) {
        // If the call has a registry, log the transaction
        if let Some(registry) = call.registry() {
            if let Err(e) = registry.log_transaction(call_id, transaction).await {
                error!("Failed to log transaction: {}", e);
            } else {
                debug!("Transaction logged successfully");
            }
        } else {
            debug!("Call has no registry, transaction not logged");
        }
    } else {
        debug!("Call not found, transaction not logged");
    }
}

/// Store transaction ID for the response
async fn store_transaction_id(call: Arc<Call>, transaction_id: &str) -> Result<()> {
    use tracing::{debug, error};
    
    debug!("Storing transaction ID {} for call {}", transaction_id, call.id());
    
    // Store the transaction ID in the call
    if let Err(e) = call.store_invite_transaction_id(transaction_id.to_string()).await {
        error!("Failed to store transaction ID: {}", e);
    }
    
    Ok(())
} 