use std::collections::HashMap;
use std::net::{SocketAddr, IpAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result as AnyResult;
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
use crate::call::{CallState, CallEvent, CallDirection};
use crate::call::call_struct::Call;
use crate::call_registry;
use crate::DEFAULT_SIP_PORT;

use super::events::SipClientEvent;
use super::registration::Registration;
use super::lightweight::LightweightClient;
use super::utils::{ChannelTransformer, add_response_headers};

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
    /// Get a reference to the client configuration
    pub fn config_ref(&self) -> &ClientConfig {
        &self.config
    }
    
    /// Get a reference to the transaction manager
    pub fn transaction_manager_ref(&self) -> &Arc<TransactionManager> {
        &self.transaction_manager
    }
    
    /// Get a reference to the CSeq counter
    pub fn cseq_ref(&self) -> &Arc<Mutex<u32>> {
        &self.cseq
    }
    
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
        
        // Store references for transaction event handling
        let transaction_manager = self.transaction_manager.clone();
        let subscription = transaction_manager.subscribe();
        let mut transaction_events_rx = std::mem::replace(&mut self.transaction_events_rx, subscription);
        let event_tx = self.event_tx.clone();
        let running = self.running.clone();
        let pending_responses = self.pending_responses.clone();
        
        // Spawn transaction event processor task
        let event_task = tokio::spawn(async move {
            info!("SIP client transaction event processor started");
            
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
                
                // Process transaction event
                match &event {
                    TransactionEvent::ResponseReceived { message, transaction_id, .. } => {
                        if let Message::Response(response) = message {
                            debug!("Response received for transaction {}: {}", transaction_id, response.status);
                            
                            // Get the pending response handler if any
                            let mut handlers = pending_responses.lock().await;
                            if let Some(tx) = handlers.remove(transaction_id) {
                                // Send the response to the waiting handler
                                let _ = tx.send(response.clone());
                            }
                        }
                    },
                    // Process other events as needed
                    _ => {}
                }
                
                // Forward event to client event stream
                let _ = event_tx.send(SipClientEvent::Error("Transaction event received".into())).await;
            }
            
            info!("SIP client transaction event processor stopped");
        });
        
        self.event_task = Some(event_task);
        
        info!("SIP client started");
        
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
        let request_uri: Uri = format!("sip:{}", self.config_ref().domain).parse()
            .map_err(|e| Error::SipProtocol(format!("Invalid domain URI: {}", e)))?;
        
        // Create REGISTER request
        let mut request = self.create_request(Method::Register, request_uri.clone()).await?;
        
        // Add Expires header
        request.headers.push(Header::text(
            HeaderName::Expires, 
            self.config_ref().register_expires.to_string()
        ));
        
        // Add Contact header with expires parameter
        let contact = format!(
            "<sip:{}@{};transport=udp>;expires={}",
            self.config_ref().username,
            self.config_ref().local_addr.unwrap(),
            self.config_ref().register_expires
        );
        request.headers.push(Header::text(HeaderName::Contact, contact));
        
        // Add Content-Length
        request.headers.push(Header::text(HeaderName::ContentLength, "0"));
        
        // Store registration state
        let mut registration = Registration {
            server: server_addr,
            uri: request_uri,
            registered: false,
            expires: self.config_ref().register_expires,
            registered_at: None,
            error: None,
            refresh_task: None,
        };
        
        // Set registration state
        *self.registration.write().await = Some(registration.clone());
        
        // Send via transaction layer
        let response = self.send_via_transaction(request, server_addr).await?;
        
        if response.status == StatusCode::Ok {
            info!("Registration successful");
            
            // Update registration state
            registration.registered = true;
            registration.registered_at = Some(Instant::now());
            registration.error = None;
            
            // Set up registration refresh
            let refresh_interval = (self.config_ref().register_expires as f32 * self.config_ref().register_refresh) as u64;
            
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
                expires: Some(self.config_ref().register_expires),
                error: None,
            }).await;
            
            Ok(())
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
        // Implementation would go here
        Ok(())
    }
    
    /// Call a SIP URI
    pub async fn call(&self, target_uri: &str, config: CallConfig) -> Result<Arc<Call>> {
        // Parse the target URI
        let uri = target_uri.parse::<Uri>()
            .map_err(|e| Error::SipProtocol(format!("Invalid target URI: {}", e)))?;
        
        // Get local address from config
        let local_addr = self.config_ref().local_addr
            .ok_or_else(|| Error::Configuration("Local address not configured".into()))?;
        
        // Create a call ID (UUID)
        let call_id = format!("{}@{}", uuid::Uuid::new_v4(), self.config_ref().domain);
        
        // Create a random tag
        let local_tag = format!("{}", uuid::Uuid::new_v4().as_simple());
        
        // Extract host and port from URI
        let host = uri.host.clone();
        let port = uri.port.unwrap_or(DEFAULT_SIP_PORT);
        
        // Create a socket address from host and port
        let remote_addr = match &host {
            rvoip_sip_core::Host::IPv4(ip) => {
                match ip.parse::<std::net::Ipv4Addr>() {
                    Ok(ip_addr) => SocketAddr::new(std::net::IpAddr::V4(ip_addr), port),
                    Err(_) => return Err(Error::Call(format!("Could not parse IPv4 host: {}", ip))),
                }
            },
            rvoip_sip_core::Host::IPv6(ip) => {
                match ip.parse::<std::net::Ipv6Addr>() {
                    Ok(ip_addr) => SocketAddr::new(std::net::IpAddr::V6(ip_addr), port),
                    Err(_) => return Err(Error::Call(format!("Could not parse IPv6 host: {}", ip))),
                }
            },
            rvoip_sip_core::Host::Domain(domain) => {
                // Try to parse as IP address first
                if let Ok(ip) = domain.parse::<IpAddr>() {
                    SocketAddr::new(ip, port)
                } else {
                    // Hostname instead of IP, need to resolve
                    // For now, just return an error, in practice would use DNS
                    return Err(Error::Call(format!("Could not resolve domain: {}", domain)));
                }
            }
        };
        
        // Create a new call event channel
        let (call_event_tx, _call_event_rx) = mpsc::channel(10);
        
        // Create From URI
        let from_uri = format!("sip:{}@{}", self.config_ref().username, self.config_ref().domain).parse::<Uri>()
            .map_err(|e| Error::SipProtocol(format!("Invalid From URI: {}", e)))?;
        
        // Create a new call
        let (call, _state_tx) = Call::new(
            CallDirection::Outgoing,
            config,
            call_id.clone(),
            local_tag.clone(),
            from_uri,
            uri.clone(),
            remote_addr,
            self.transaction_manager_ref().clone(),
            call_event_tx,
        );
        
        // Create an INVITE request
        let mut invite = Request::new(Method::Invite, uri.clone());
        
        // Add headers
        invite.headers.push(Header::text(HeaderName::From, 
            format!("<sip:{}@{}>;tag={}", self.config_ref().username, self.config_ref().domain, local_tag)));
        invite.headers.push(Header::text(HeaderName::To, 
            format!("<{}>", uri)));
        invite.headers.push(Header::text(HeaderName::CallId, call_id));
        
        // Get next CSeq
        let cseq = {
            let mut cseq_lock = self.cseq_ref().lock().await;
            let current = *cseq_lock;
            *cseq_lock += 1;
            current
        };
        
        invite.headers.push(Header::text(HeaderName::CSeq, 
            format!("{} INVITE", cseq)));
        invite.headers.push(Header::text(HeaderName::Contact, 
            format!("<sip:{}@{}>", self.config_ref().username, local_addr)));
        invite.headers.push(Header::text(HeaderName::MaxForwards, "70"));
        
        // Store the request
        call.store_invite_request(invite.clone()).await?;
        
        // Get local SDP for INVITE
        let local_sdp = call.setup_local_sdp().await?;
        
        // Add SDP if we have it
        if let Some(sdp) = local_sdp {
            let sdp_str = sdp.to_string();
            invite.headers.push(Header::text(HeaderName::ContentType, "application/sdp"));
            invite.headers.push(Header::text(HeaderName::ContentLength, sdp_str.len().to_string()));
            invite.body = sdp_str.into_bytes().into();
        } else {
            invite.headers.push(Header::text(HeaderName::ContentLength, "0"));
        }
        
        // Store the call in our map
        self.calls.lock().await.insert(call.id().to_string(), Arc::new(RwLock::new(call.as_ref().clone())));
        
        // Send the INVITE request via transaction layer
        let transaction_id = self.transaction_manager_ref().create_client_transaction(
            invite.clone(), 
            remote_addr
        ).await.map_err(|e| Error::Transport(e.to_string()))?;
        
        // Send the request
        self.transaction_manager_ref().send_request(&transaction_id).await
            .map_err(|e| Error::Transport(e.to_string()))?;
        
        // Store the transaction ID
        call.store_invite_transaction_id(transaction_id.clone()).await?;
        
        // Now wait for response in a separate task so we don't block
        let pending_responses = self.pending_responses.clone();
        let task_call = call.clone();
        
        tokio::spawn(async move {
            // Create a channel for the response
            let (tx, rx) = oneshot::channel();
            
            // Register for the response
            pending_responses.lock().await.insert(transaction_id, tx);
            
            // Wait for response
            match tokio::time::timeout(Duration::from_secs(32), rx).await {
                Ok(Ok(response)) => {
                    // Process the response
                    if let Err(e) = task_call.handle_response(&response).await {
                        error!("Error handling call response: {}", e);
                    }
                },
                Ok(Err(_)) => {
                    error!("Response channel closed");
                    // Transition to failed state
                    let _ = task_call.transition_to(CallState::Failed).await;
                },
                Err(_) => {
                    error!("Timeout waiting for INVITE response");
                    // Transition to failed state
                    let _ = task_call.transition_to(CallState::Failed).await;
                }
            }
        });
        
        // Update call state
        call.transition_to(CallState::Ringing).await?;
        
        Ok(call)
    }
    
    /// Get a stream of client events
    pub fn event_stream(&self) -> broadcast::Receiver<SipClientEvent> {
        self.event_broadcast.subscribe()
    }
    
    /// Get all active calls
    pub async fn calls(&self) -> HashMap<String, Arc<RwLock<Call>>> {
        self.calls.lock().await.clone()
    }
    
    /// Get a call by ID
    pub async fn call_by_id(&self, call_id: &str) -> Option<Arc<RwLock<Call>>> {
        self.calls.lock().await.get(call_id).cloned()
    }
    
    /// Get registration state
    pub async fn registration_state(&self) -> Option<(String, bool, Option<u32>)> {
        if let Some(reg) = self.registration.read().await.as_ref() {
            Some((
                reg.server.to_string(),
                reg.registered,
                if reg.registered { Some(reg.expires) } else { None }
            ))
        } else {
            None
        }
    }
    
    /// Run the client (blocking)
    pub async fn run(&mut self) -> Result<()> {
        // Make sure client is started
        self.start().await?;
        
        // Get the event receiver
        let mut rx = self.event_rx.take()
            .ok_or_else(|| Error::Client("Event receiver not available".into()))?;
        
        // Process events
        while let Some(event) = rx.recv().await {
            // Broadcast the event
            let _ = self.event_broadcast.send(event.clone());
            
            // Handle event based on type
            match event {
                SipClientEvent::Call(CallEvent::Terminated { .. }) => {
                    // Call terminated, no special handling needed here
                },
                _ => {}
            }
        }
        
        Ok(())
    }
    
    /// Create a new SIP request
    pub async fn create_request(&self, method: Method, target_uri: Uri) -> Result<Request> {
        // Create a new request
        let mut request = Request::new(method.clone(), target_uri);
        
        // Add From header
        let from = format!(
            "<sip:{}@{}>",
            self.config_ref().username, self.config_ref().domain
        );
        request.headers.push(Header::text(HeaderName::From, from));
        
        // Add Via header
        let via = format!(
            "SIP/2.0/UDP {};branch=z9hG4bK{}",
            self.config_ref().local_addr.unwrap(),
            uuid::Uuid::new_v4().to_string()
        );
        request.headers.push(Header::text(HeaderName::Via, via));
        
        // Add Call-ID header
        let call_id = format!("{}", uuid::Uuid::new_v4());
        request.headers.push(Header::text(HeaderName::CallId, call_id));
        
        // Add CSeq header
        let cseq = self.next_cseq().await;
        request.headers.push(Header::text(
            HeaderName::CSeq,
            format!("{} {}", cseq, method)
        ));
        
        // Add Max-Forwards header
        request.headers.push(Header::text(HeaderName::MaxForwards, "70"));
        
        // Add User-Agent header
        request.headers.push(Header::text(
            HeaderName::UserAgent,
            format!("RVOIP SIP Client {}", crate::VERSION)
        ));
        
        Ok(request)
    }
    
    /// Get the next CSeq value
    async fn next_cseq(&self) -> u32 {
        let mut cseq = self.cseq_ref().lock().await;
        *cseq += 1;
        *cseq
    }
    
    /// Resolve a URI to a socket address
    fn resolve_uri(&self, _uri: &Uri) -> Result<SocketAddr> {
        // Implementation would go here
        // For now, we'll leave this as a stub that returns an error
        Err(Error::Call("Resolver not implemented".into()))
    }
    
    /// Create a lightweight clone for use in tasks
    fn clone_lightweight(&self) -> LightweightClient {
        LightweightClient {
            transaction_manager: self.transaction_manager_ref().clone(),
            config: self.config_ref().clone(),
            cseq: self.cseq_ref().clone(),
            registration: self.registration.clone(),
            event_tx: self.event_tx.clone(),
        }
    }
    
    /// Send a request via transaction layer and wait for response
    async fn send_via_transaction(&self, request: Request, destination: SocketAddr) -> Result<Response> {
        // Create a client transaction
        let transaction_id = self.transaction_manager_ref().create_client_transaction(
            request.clone(), 
            destination
        ).await.map_err(|e| Error::Transport(e.to_string()))?;
        
        // Create a oneshot channel for the response
        let (tx, rx) = oneshot::channel();
        
        // Store the response handler
        self.pending_responses.lock().await.insert(transaction_id.clone(), tx);
        
        // Send the request
        self.transaction_manager_ref().send_request(&transaction_id).await
            .map_err(|e| Error::Transport(e.to_string()))?;
        
        // Wait for the response with timeout
        match time::timeout(Duration::from_secs(30), rx).await {
            Ok(Ok(response)) => {
                Ok(response)
            },
            Ok(Err(_)) => {
                Err(Error::Transport("Response channel closed".into()))
            },
            Err(_) => {
                // Remove the pending response handler
                self.pending_responses.lock().await.remove(&transaction_id);
                Err(Error::Timeout("Timeout waiting for response".into()))
            }
        }
    }
    
    /// Set the call registry
    pub async fn set_call_registry(&mut self, registry: call_registry::CallRegistry) {
        self.registry = Some(Arc::new(registry));
    }
    
    /// Subscribe to events
    pub fn subscribe(&self) -> broadcast::Receiver<SipClientEvent> {
        self.event_broadcast.subscribe()
    }
} 