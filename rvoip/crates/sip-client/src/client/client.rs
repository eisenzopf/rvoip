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
use crate::call::{Call, CallState, CallEvent, CallDirection};
use crate::call_registry;

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
        // Implementation would go here
        // For now, we'll leave this as a stub to be filled in later
        Ok(())
    }
    
    /// Stop the client
    pub async fn stop(&mut self) -> Result<()> {
        // Implementation would go here
        // For now, we'll leave this as a stub to be filled in later
        Ok(())
    }
    
    /// Register with a SIP server
    pub async fn register(&self, server_addr: SocketAddr) -> Result<()> {
        // Implementation would go here
        // For now, we'll leave this as a stub to be filled in later
        Ok(())
    }
    
    /// Unregister from SIP server
    pub async fn unregister(&self) -> Result<()> {
        // Implementation would go here
        // For now, we'll leave this as a stub to be filled in later
        Ok(())
    }
    
    /// Call a SIP URI
    pub async fn call(&self, target_uri: &str, config: CallConfig) -> Result<Arc<Call>> {
        // Implementation would go here
        // For now, we'll leave this as a stub that returns an error
        Err(Error::Call("Not implemented".into()))
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
            self.config.username, self.config.domain
        );
        request.headers.push(Header::text(HeaderName::From, from));
        
        // Add Via header
        let via = format!(
            "SIP/2.0/UDP {};branch=z9hG4bK{}",
            self.config.local_addr.unwrap(),
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
        let mut cseq = self.cseq.lock().await;
        *cseq += 1;
        *cseq
    }
    
    /// Resolve a URI to a socket address
    fn resolve_uri(&self, uri: &Uri) -> Result<SocketAddr> {
        // Implementation would go here
        // For now, we'll leave this as a stub that returns an error
        Err(Error::Call("Not implemented".into()))
    }
    
    /// Create a lightweight clone for use in tasks
    fn clone_lightweight(&self) -> LightweightClient {
        LightweightClient {
            transaction_manager: self.transaction_manager.clone(),
            config: self.config.clone(),
            cseq: self.cseq.clone(),
            registration: self.registration.clone(),
            event_tx: self.event_tx.clone(),
        }
    }
    
    /// Send a request via transaction layer and wait for response
    async fn send_via_transaction(&self, request: Request, destination: SocketAddr) -> Result<Response> {
        // Send the request via transaction manager
        let transaction_id = self.transaction_manager.send_request(request, destination).await
            .map_err(|e| Error::Transport(e.to_string()))?;
        
        // Create a oneshot channel for the response
        let (tx, rx) = oneshot::channel();
        
        // Store the response handler
        self.pending_responses.lock().await.insert(transaction_id.clone(), tx);
        
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
    
    /// Wait for a response to a transaction
    async fn wait_for_transaction_response(&self, transaction_id: &str) -> Result<Response> {
        // Wait for the response
        match self.transaction_manager.wait_for_response(transaction_id).await {
            Ok(response) => {
                Ok(response)
            },
            Err(e) => {
                Err(Error::Transport(format!("Error waiting for response: {}", e)))
            }
        }
    }
    
    /// Set the call registry
    pub async fn set_call_registry(&mut self, registry: call_registry::CallRegistry) {
        self.registry = Some(Arc::new(registry));
    }
    
    /// Clone the client
    pub fn clone(&self) -> LightweightClient {
        self.clone_lightweight()
    }
    
    /// Subscribe to events
    pub fn subscribe(&self) -> broadcast::Receiver<SipClientEvent> {
        self.event_broadcast.subscribe()
    }
} 