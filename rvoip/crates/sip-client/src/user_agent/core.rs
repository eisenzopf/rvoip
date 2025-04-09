use std::collections::HashMap;
use std::net::{SocketAddr, IpAddr};
use std::sync::Arc;
use std::time::Duration;
use std::fmt;

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
use crate::call_registry::{CallRegistry, CallRecord};
use crate::media::MediaSession;

use super::handlers::handle_incoming_request;

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
                        let call_id = call.sip_call_id().to_string();
                        debug!("Registering incoming call in registry: id={}, sip_call_id={}", call.id(), call_id);
                        if let Err(e) = registry_clone.register_call(call.clone()).await {
                            error!("Failed to register incoming call: {}", e);
                        }
                    },
                    CallEvent::StateChanged { call, previous, current } => {
                        let call_id = call.sip_call_id().to_string();
                        debug!("Updating call state in registry: id={}, sip_call_id={}, {} -> {}", 
                               call.id(), call_id, previous, current);
                        if let Err(e) = registry_clone.update_call_state(&call_id, *previous, *current).await {
                            error!("Failed to update call state in registry (sip_call_id={}): {}", call_id, e);
                        }
                    },
                    CallEvent::Terminated { call, .. } => {
                        // Ensure termination is recorded in call history
                        let current_state = match call.state().await {
                            CallState::Terminated => CallState::Terminated,
                            _ => CallState::Terminated, // Force to terminated state
                        };
                        
                        if let Err(e) = registry_clone.update_call_state(
                            &call.sip_call_id(), CallState::Terminating, current_state
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
                                    if let Err(e) = call.handle_response(&response.clone()).await {
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
                                    if let Err(e) = call.handle_response(&response).await {
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
    
    /// Set the call registry for persistence
    pub async fn set_call_registry(&mut self, registry: CallRegistry) {
        let registry_arc = Arc::new(registry);
        self.call_registry = registry_arc;
        info!("Call registry set for UserAgent");
    }
} 