use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, trace, warn};

use rvoip_sip_core::{Message, Method, Request, Response, StatusCode, HeaderName, Header, HeaderValue};
use rvoip_sip_transport::Transport;

use crate::error::{Error, Result};
use crate::transaction::{Transaction, TransactionState, TransactionType};
use crate::utils;

/// Server transaction trait
#[async_trait::async_trait]
pub trait ServerTransaction: Transaction {
    /// Send a response for this transaction
    async fn send_response(&mut self, response: Response) -> Result<()>;
}

/// Server INVITE transaction
#[derive(Debug)]
pub struct ServerInviteTransaction {
    /// Transaction ID
    id: String,
    /// Current state
    state: TransactionState,
    /// Original request
    request: Request,
    /// Last response sent
    last_response: Option<Response>,
    /// Remote address (where to send responses)
    remote_addr: SocketAddr,
    /// Transport to use for sending responses
    transport: Arc<dyn Transport>,
    /// Timer G duration (INVITE retransmission interval)
    timer_g: Duration,
    /// Timer H duration (wait time for ACK)
    timer_h: Duration,
    /// Timer I duration (wait time in CONFIRMED state)
    timer_i: Duration,
    /// Retransmission count
    retransmit_count: u32,
}

impl ServerInviteTransaction {
    /// Create a new server INVITE transaction
    pub fn new(
        mut request: Request,
        remote_addr: SocketAddr,
        transport: Arc<dyn Transport>,
    ) -> Result<Self> {
        // Extract branch to generate ID, or generate a new one if missing
        let branch = match utils::extract_branch(&Message::Request(request.clone())) {
            Some(branch) => branch,
            None => {
                // No branch parameter found, add one
                debug!("Adding missing branch parameter to Via header");
                
                // Generate a branch parameter
                let new_branch = utils::generate_branch();
                
                // Check if there's an existing Via header we can modify
                let found_via = request.headers.iter_mut().find(|h| h.name == HeaderName::Via);
                
                if let Some(via_header) = found_via {
                    // Modify existing Via header to add branch
                    let current_value = via_header.value.to_string();
                    let new_value = if current_value.contains("branch=") {
                        current_value // Already has branch, don't modify (shouldn't happen)
                    } else {
                        format!("{};branch={}", current_value, new_branch)
                    };
                    
                    // Update header value
                    via_header.value = HeaderValue::text(new_value);
                    new_branch // Return the branch we just added
                } else {
                    // No Via header found, add a new one with branch
                    debug!("No Via header found, adding one with branch parameter");
                    
                    // Use localhost as placeholder
                    let via_value = format!("SIP/2.0/UDP 127.0.0.1:5060;branch={}", new_branch);
                    request.headers.push(Header::text(HeaderName::Via, via_value));
                    
                    new_branch // Return the branch we just added
                }
            }
        };
        
        let id = format!("ist_{}", branch);
        
        Ok(ServerInviteTransaction {
            id,
            state: TransactionState::Initial,
            request,
            last_response: None,
            remote_addr,
            transport,
            timer_g: Duration::from_millis(500),  // RFC 3261 recommends T1 (500ms)
            timer_h: Duration::from_secs(32),     // 64*T1 seconds
            timer_i: Duration::from_secs(5),      // T4 seconds
            retransmit_count: 0,
        })
    }
    
    /// Transition to a new state
    fn transition_to(&mut self, new_state: TransactionState) -> Result<()> {
        debug!("[{}] State transition: {:?} -> {:?}", self.id, self.state, new_state);
        
        // Validate state transition
        match (self.state, new_state) {
            // Valid transitions
            (TransactionState::Initial, TransactionState::ServerProceeding) => {},
            (TransactionState::ServerProceeding, TransactionState::Completed) => {},
            (TransactionState::Completed, TransactionState::Confirmed) => {},
            (TransactionState::Completed, TransactionState::Terminated) => {},
            (TransactionState::Confirmed, TransactionState::Terminated) => {},
            
            // Invalid transitions
            _ => return Err(Error::InvalidStateTransition(
                format!("Invalid state transition: {:?} -> {:?}", self.state, new_state)
            )),
        }
        
        self.state = new_state;
        Ok(())
    }
}

#[async_trait::async_trait]
impl Transaction for ServerInviteTransaction {
    fn id(&self) -> &str {
        &self.id
    }
    
    fn transaction_type(&self) -> TransactionType {
        TransactionType::Server
    }
    
    fn state(&self) -> TransactionState {
        self.state
    }
    
    fn original_request(&self) -> &Request {
        &self.request
    }
    
    fn last_response(&self) -> Option<&Response> {
        self.last_response.as_ref()
    }
    
    async fn process_message(&mut self, message: Message) -> Result<Option<Message>> {
        match message {
            Message::Request(request) => {
                // We only expect ACK in server INVITE transaction
                if request.method != Method::Ack {
                    warn!("[{}] Received non-ACK request in server INVITE transaction: {:?}", self.id, request.method);
                    return Ok(None);
                }
                
                match self.state {
                    TransactionState::Initial => {
                        // In some cases, ACK might arrive before the state has been properly updated
                        // This is not ideal but should be handled gracefully
                        debug!("[{}] Received ACK in INITIAL state, handling it anyway", self.id);
                        // We won't transition state here as this is an abnormal case
                        // The dialog layer should handle this appropriately
                        Ok(None)
                    },
                    TransactionState::ServerProceeding => {
                        // ACK received prematurely, but we'll be tolerant
                        debug!("[{}] Received ACK in PROCEEDING state, transitioning to CONFIRMED", self.id);
                        self.transition_to(TransactionState::Confirmed)?;
                        Ok(None)
                    },
                    TransactionState::Completed => {
                        debug!("[{}] Received ACK in COMPLETED state, transitioning to CONFIRMED", self.id);
                        self.transition_to(TransactionState::Confirmed)?;
                        
                        // Start timer I
                        // In a real implementation, we'd start a timer here
                        
                        Ok(None)
                    },
                    TransactionState::Confirmed => {
                        // Already received ACK, ignore
                        trace!("[{}] Ignoring duplicate ACK in CONFIRMED state", self.id);
                        Ok(None)
                    },
                    _ => {
                        warn!("[{}] Received ACK in invalid state: {:?}", self.id, self.state);
                        Ok(None)
                    }
                }
            },
            Message::Response(_) => {
                warn!("[{}] Received response in server transaction", self.id);
                // Server transactions don't process responses
                Ok(None)
            }
        }
    }
    
    fn matches(&self, message: &Message) -> bool {
        if let Message::Request(request) = message {
            // Only match ACK requests for INVITE transactions
            if request.method != Method::Ack {
                return false;
            }
            
            // Check if branch matches
            if let (Some(incoming_branch), Some(our_branch)) = (
                message.first_via().and_then(|via| via.get("branch").flatten().map(|s| s.to_string())),
                utils::extract_branch(&Message::Request(self.request.clone()))
            ) {
                // Match transaction by branch
                return incoming_branch == our_branch;
            }
        }
        
        false
    }
    
    fn timeout_duration(&self) -> Option<Duration> {
        match self.state {
            TransactionState::Completed => Some(self.timer_g),
            TransactionState::Confirmed => Some(self.timer_i),
            _ => None,
        }
    }
    
    async fn on_timeout(&mut self) -> Result<Option<Message>> {
        match self.state {
            TransactionState::Completed => {
                // Timer G fired - retransmit response
                if let Some(response) = &self.last_response {
                    debug!("[{}] Timer G fired, retransmitting response", self.id);
                    
                    let cloned_response = response.clone();
                    self.transport.send_message(cloned_response.into(), self.remote_addr).await?;
                    
                    // Double retransmission interval (exponential backoff)
                    self.timer_g = Duration::min(
                        self.timer_g * 2,
                        Duration::from_secs(4)
                    );
                    
                    self.retransmit_count += 1;
                    
                    // Check if we've hit timer H
                    if self.retransmit_count > 10 { // Arbitrary limit for now, would use actual timer in production
                        debug!("[{}] Timer H fired, no ACK received, terminating transaction", self.id);
                        self.transition_to(TransactionState::Terminated)?;
                    }
                    
                    Ok(None)
                } else {
                    warn!("[{}] Timer G fired but no response to retransmit", self.id);
                    Ok(None)
                }
            },
            TransactionState::Confirmed => {
                // Timer I fired - terminate transaction
                debug!("[{}] Timer I fired, terminating transaction", self.id);
                self.transition_to(TransactionState::Terminated)?;
                Ok(None)
            },
            _ => {
                warn!("[{}] Timeout in unexpected state: {:?}", self.id, self.state);
                Ok(None)
            }
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[async_trait::async_trait]
impl ServerTransaction for ServerInviteTransaction {
    async fn send_response(&mut self, response: Response) -> Result<()> {
        let status = response.status;
        
        match self.state {
            TransactionState::Initial => {
                if status.is_provisional() {
                    debug!("[{}] Sending provisional response: {}", self.id, status);
                    self.transition_to(TransactionState::ServerProceeding)?;
                } else if status.is_success() || status.is_error() {
                    debug!("[{}] Sending final response: {}", self.id, status);
                    self.transition_to(TransactionState::ServerProceeding)?;
                    self.transition_to(TransactionState::Completed)?;
                    
                    // Start timer G (response retransmission) for unreliable transports
                    // We'll handle this in the transaction manager with on_timeout
                    
                    // Start timer H (wait for ACK)
                    // We'll handle this in the transaction manager with on_timeout
                } else {
                    return Err(Error::InvalidStateTransition(
                        format!("Cannot send response with status {} in INITIAL state", status)
                    ));
                }
            },
            TransactionState::ServerProceeding => {
                if status.is_provisional() {
                    debug!("[{}] Sending provisional response: {}", self.id, status);
                } else if status.is_success() || status.is_error() {
                    debug!("[{}] Sending final response: {}", self.id, status);
                    self.transition_to(TransactionState::Completed)?;
                    
                    // Start timer G (response retransmission) for unreliable transports
                    // We'll handle this in the transaction manager with on_timeout
                    
                    // Start timer H (wait for ACK)
                    // We'll handle this in the transaction manager with on_timeout
                } else {
                    return Err(Error::InvalidStateTransition(
                        format!("Cannot send response with status {} in PROCEEDING state", status)
                    ));
                }
            },
            TransactionState::Completed | TransactionState::Confirmed | TransactionState::Terminated => {
                return Err(Error::InvalidStateTransition(
                    format!("Cannot send response in {:?} state", self.state)
                ));
            },
            _ => {
                return Err(Error::InvalidStateTransition(
                    format!("Invalid state for server INVITE transaction: {:?}", self.state)
                ));
            }
        }
        
        // Store response for potential retransmissions
        self.last_response = Some(response.clone());
        
        // Send response
        self.transport.send_message(response.into(), self.remote_addr).await?;
        
        Ok(())
    }
}

/// Server non-INVITE transaction
#[derive(Debug)]
pub struct ServerNonInviteTransaction {
    /// Transaction ID
    id: String,
    /// Current state
    state: TransactionState,
    /// Original request
    request: Request,
    /// Last response sent
    last_response: Option<Response>,
    /// Remote address (where to send responses)
    remote_addr: SocketAddr,
    /// Transport to use for sending responses
    transport: Arc<dyn Transport>,
    /// Timer J duration (wait time for retransmissions)
    timer_j: Duration,
}

impl ServerNonInviteTransaction {
    /// Create a new server non-INVITE transaction
    pub fn new(
        mut request: Request,
        remote_addr: SocketAddr,
        transport: Arc<dyn Transport>,
    ) -> Result<Self> {
        // Extract branch to generate ID, or generate a new one if missing
        let branch = match utils::extract_branch(&Message::Request(request.clone())) {
            Some(branch) => branch,
            None => {
                // No branch parameter found, add one
                debug!("Adding missing branch parameter to Via header");
                
                // Generate a branch parameter
                let new_branch = utils::generate_branch();
                
                // Check if there's an existing Via header we can modify
                let found_via = request.headers.iter_mut().find(|h| h.name == HeaderName::Via);
                
                if let Some(via_header) = found_via {
                    // Modify existing Via header to add branch
                    let current_value = via_header.value.to_string();
                    let new_value = if current_value.contains("branch=") {
                        current_value // Already has branch, don't modify (shouldn't happen)
                    } else {
                        format!("{};branch={}", current_value, new_branch)
                    };
                    
                    // Update header value
                    via_header.value = HeaderValue::text(new_value);
                    new_branch // Return the branch we just added
                } else {
                    // No Via header found, add a new one with branch
                    debug!("No Via header found, adding one with branch parameter");
                    
                    // Use localhost as placeholder
                    let via_value = format!("SIP/2.0/UDP 127.0.0.1:5060;branch={}", new_branch);
                    request.headers.push(Header::text(HeaderName::Via, via_value));
                    
                    new_branch // Return the branch we just added
                }
            }
        };
        
        let id = format!("nist_{}", branch);
        
        Ok(ServerNonInviteTransaction {
            id,
            state: TransactionState::Initial,
            request,
            last_response: None,
            remote_addr,
            transport,
            timer_j: Duration::from_secs(32),   // 64*T1 seconds
        })
    }
    
    /// Transition to a new state
    fn transition_to(&mut self, new_state: TransactionState) -> Result<()> {
        debug!("[{}] State transition: {:?} -> {:?}", self.id, self.state, new_state);
        
        // Validate state transition
        match (self.state, new_state) {
            // Valid transitions
            (TransactionState::Initial, TransactionState::Trying) => {},
            (TransactionState::Initial, TransactionState::Proceeding) => {},
            (TransactionState::Initial, TransactionState::Completed) => {},
            (TransactionState::Trying, TransactionState::Proceeding) => {},
            (TransactionState::Trying, TransactionState::Completed) => {},
            (TransactionState::Proceeding, TransactionState::Completed) => {},
            (TransactionState::Completed, TransactionState::Terminated) => {},
            
            // Invalid transitions
            _ => return Err(Error::InvalidStateTransition(
                format!("Invalid state transition: {:?} -> {:?}", self.state, new_state)
            )),
        }
        
        self.state = new_state;
        Ok(())
    }
}

#[async_trait::async_trait]
impl Transaction for ServerNonInviteTransaction {
    fn id(&self) -> &str {
        &self.id
    }
    
    fn transaction_type(&self) -> TransactionType {
        TransactionType::Server
    }
    
    fn state(&self) -> TransactionState {
        self.state
    }
    
    fn original_request(&self) -> &Request {
        &self.request
    }
    
    fn last_response(&self) -> Option<&Response> {
        self.last_response.as_ref()
    }
    
    async fn process_message(&mut self, message: Message) -> Result<Option<Message>> {
        match message {
            Message::Request(request) => {
                // For non-INVITE, we might receive retransmission of the original request
                if request.method != self.request.method {
                    warn!("[{}] Received request with different method: {:?} vs {:?}", 
                        self.id, request.method, self.request.method);
                    return Ok(None);
                }
                
                match self.state {
                    TransactionState::Trying | TransactionState::Proceeding => {
                        debug!("[{}] Received retransmission in {:?} state", self.id, self.state);
                        
                        // Retransmit last response if we have one
                        if let Some(response) = &self.last_response {
                            let cloned_response = response.clone();
                            self.transport.send_message(cloned_response.into(), self.remote_addr).await?;
                        }
                        
                        Ok(None)
                    },
                    TransactionState::Completed => {
                        // Retransmit final response
                        if let Some(response) = &self.last_response {
                            debug!("[{}] Retransmitting final response", self.id);
                            let cloned_response = response.clone();
                            self.transport.send_message(cloned_response.into(), self.remote_addr).await?;
                        }
                        
                        Ok(None)
                    },
                    _ => {
                        warn!("[{}] Received request in invalid state: {:?}", self.id, self.state);
                        Ok(None)
                    }
                }
            },
            Message::Response(_) => {
                warn!("[{}] Received response in server transaction", self.id);
                // Server transactions don't process responses
                Ok(None)
            }
        }
    }
    
    fn matches(&self, message: &Message) -> bool {
        if let Message::Request(request) = message {
            // Check method
            if request.method != self.request.method {
                return false;
            }
            
            // Skip ACK matching, handled by INVITE transactions
            if request.method == Method::Ack {
                return false;
            }
            
            // Check if branch matches
            if let (Some(incoming_branch), Some(our_branch)) = (
                message.first_via().and_then(|via| via.get("branch").flatten().map(|s| s.to_string())),
                utils::extract_branch(&Message::Request(self.request.clone()))
            ) {
                // Match transaction by branch
                return incoming_branch == our_branch;
            }
        }
        
        false
    }
    
    fn timeout_duration(&self) -> Option<Duration> {
        match self.state {
            TransactionState::Completed => Some(self.timer_j),
            _ => None,
        }
    }
    
    async fn on_timeout(&mut self) -> Result<Option<Message>> {
        match self.state {
            TransactionState::Completed => {
                // Timer J fired - terminate transaction
                debug!("[{}] Timer J fired, terminating transaction", self.id);
                self.transition_to(TransactionState::Terminated)?;
                Ok(None)
            },
            _ => {
                warn!("[{}] Timeout in unexpected state: {:?}", self.id, self.state);
                Ok(None)
            }
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[async_trait::async_trait]
impl ServerTransaction for ServerNonInviteTransaction {
    async fn send_response(&mut self, response: Response) -> Result<()> {
        let status = response.status;
        
        match self.state {
            TransactionState::Initial => {
                if status == StatusCode::Trying {
                    debug!("[{}] Sending 100 Trying response", self.id);
                    self.transition_to(TransactionState::Trying)?;
                } else if status.is_provisional() {
                    debug!("[{}] Sending provisional response: {}", self.id, status);
                    self.transition_to(TransactionState::Proceeding)?;
                } else {
                    debug!("[{}] Sending final response: {}", self.id, status);
                    self.transition_to(TransactionState::Completed)?;
                    
                    // Start timer J
                    // We'll handle this in the transaction manager with on_timeout
                }
            },
            TransactionState::Trying => {
                if status.is_provisional() && status != StatusCode::Trying {
                    debug!("[{}] Sending provisional response: {}", self.id, status);
                    self.transition_to(TransactionState::Proceeding)?;
                } else if !status.is_provisional() {
                    debug!("[{}] Sending final response: {}", self.id, status);
                    self.transition_to(TransactionState::Completed)?;
                    
                    // Start timer J
                    // We'll handle this in the transaction manager with on_timeout
                }
            },
            TransactionState::Proceeding => {
                if status.is_provisional() {
                    debug!("[{}] Sending provisional response: {}", self.id, status);
                } else {
                    debug!("[{}] Sending final response: {}", self.id, status);
                    self.transition_to(TransactionState::Completed)?;
                    
                    // Start timer J
                    // We'll handle this in the transaction manager with on_timeout
                }
            },
            TransactionState::Completed | TransactionState::Terminated => {
                return Err(Error::InvalidStateTransition(
                    format!("Cannot send response in {:?} state", self.state)
                ));
            },
            _ => {
                return Err(Error::InvalidStateTransition(
                    format!("Invalid state for server non-INVITE transaction: {:?}", self.state)
                ));
            }
        }
        
        // Store response for potential retransmissions
        self.last_response = Some(response.clone());
        
        // Send response
        self.transport.send_message(response.into(), self.remote_addr).await?;
        
        Ok(())
    }
} 