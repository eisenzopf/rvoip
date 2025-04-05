use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock, Mutex};
use tokio::time::Instant;
use tracing::{debug, error, info, warn};
use uuid::Uuid;
use futures::StreamExt;
use async_trait::async_trait;

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
    
    /// Background task handle
    event_task: Option<tokio::task::JoinHandle<()>>,
}

/// Registration state
#[derive(Debug, Clone)]
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
        })
    }
    
    /// Start the client and event processing
    pub async fn start(&mut self) -> Result<()> {
        // Check if already running
        if *self.running.read().await {
            return Ok(());
        }
        
        // Set running flag
        *self.running.write().await = true;
        
        // Start event processing task
        let client_events_tx = self.event_tx.clone();
        let mut transaction_events_rx = self.transaction_events_rx.take();
        
        let transaction_manager = self.transaction_manager.clone();
        let calls = self.calls.clone();
        let running = self.running.clone();
        
        let event_task = tokio::spawn(async move {
            debug!("SIP client event processing task started");
            
            while *running.read().await {
                // Wait for transaction event
                let event = if let Some(ref mut rx) = transaction_events_rx {
                    match rx.recv().await {
                        Some(event) => event,
                        None => {
                            error!("Transaction event channel closed");
                            break;
                        }
                    }
                } else {
                    // No receiver, exit
                    break;
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
                                    
                                    // Send error event
                                    let _ = client_events_tx.send(SipClientEvent::Error(
                                        format!("Error handling request: {}", e)
                                    )).await;
                                }
                            },
                            Message::Response(response) => {
                                debug!("Received unmatched response: {:?} from {}", response.status, source);
                                
                                // Typically won't get unmatched responses unless they're very delayed
                                // or for a transaction that's already been cleaned up
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
                                let calls_read = calls.read().await;
                                if let Some(call) = calls_read.get(call_id) {
                                    // Let the call handle the response
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
                        
                        // Send error event
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
        let request_uri = format!("sip:{}", self.config.domain).parse()
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
        let transaction = self.transaction_manager.create_client_transaction(
            request.into(),
            server_addr,
            None,
        ).await.map_err(|e| Error::Transport(e.to_string()))?;
        
        // Wait for response
        let response = transaction.wait_for_final_response().await
            .map_err(|e| Error::Transport(e.to_string()))?;
        
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
        let transaction = self.transaction_manager.create_client_transaction(
            request.into(),
            registration.server,
            None,
        ).await.map_err(|e| Error::Transport(e.to_string()))?;
        
        // Wait for response
        let response = transaction.wait_for_final_response().await
            .map_err(|e| Error::Transport(e.to_string()))?;
        
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
        
        // TODO: Set up media and SDP
        
        // Add Content-Length
        request.headers.push(Header::text(HeaderName::ContentLength, "0"));
        
        // Send INVITE request via transaction
        let transaction = self.transaction_manager.create_client_transaction(
            request.into(),
            target_addr,
            None,
        ).await.map_err(|e| Error::Transport(e.to_string()))?;
        
        // Update call state to ringing
        state_tx.send(CallState::Ringing)
            .map_err(|_| Error::Call("Failed to update call state".into()))?;
        
        // Wait for response in the call object
        
        Ok(call)
    }
    
    /// Create an event stream for the client
    pub fn event_stream(&self) -> mpsc::Receiver<SipClientEvent> {
        self.event_rx.clone()
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
        // For now, just use default port
        // In the future, implement proper SIP DNS resolution (NAPTR, SRV, etc.)
        match uri.host() {
            Some(host) => {
                let port = uri.port().unwrap_or(5060);
                match host.parse() {
                    Ok(ip) => Ok(SocketAddr::new(ip, port)),
                    Err(_) => {
                        // For now, fail on hostnames
                        // In the future, implement DNS resolution
                        Err(Error::SipProtocol(format!("Cannot resolve hostname: {}", host)))
                    }
                }
            },
            None => Err(Error::SipProtocol("URI has no host component".into())),
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

#[async_trait]
impl Clone for SipClient {
    async fn clone(&self) -> Self {
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
        let mut response = Response::new(StatusCode::CallTransactionDoesNotExist);
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