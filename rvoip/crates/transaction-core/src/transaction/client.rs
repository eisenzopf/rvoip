use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use std::str::FromStr;
use uuid::Uuid;

use tracing::{debug, trace, warn};

use rvoip_sip_core::{Message, Method, Request, Response, StatusCode, Uri, HeaderName, Header, HeaderValue};
use rvoip_sip_transport::Transport;

use crate::error::{Error, Result};
use crate::transaction::{Transaction, TransactionState, TransactionType};
use crate::utils;

/// Client transaction trait
#[async_trait::async_trait]
pub trait ClientTransaction: Transaction {
    /// Send the initial request to start this transaction
    async fn send_request(&mut self) -> Result<()>;
}

/// Client INVITE transaction
#[derive(Debug)]
pub struct ClientInviteTransaction {
    /// Transaction ID
    id: String,
    /// Current state
    state: TransactionState,
    /// Original request
    request: Request,
    /// Last response received
    last_response: Option<Response>,
    /// Remote address (where to send requests)
    remote_addr: SocketAddr,
    /// Transport to use for sending requests
    transport: Arc<dyn Transport>,
    /// Timer A duration (INVITE retransmission interval)
    timer_a: Duration,
    /// Timer B duration (INVITE timeout)
    timer_b: Duration,
    /// Timer D duration (wait time for response retransmissions)
    timer_d: Duration,
    /// Retransmission count
    retransmit_count: u32,
}

impl ClientInviteTransaction {
    /// Create a new client INVITE transaction
    pub fn new(
        mut request: Request,
        remote_addr: SocketAddr,
        transport: Arc<dyn Transport>,
    ) -> Result<Self> {
        // Ensure the request is an INVITE
        if request.method != Method::Invite {
            return Err(Error::Other("Request must be INVITE for INVITE client transaction".to_string()));
        }
        
        // Check if request has a Via header with branch parameter
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
                    via_header.value = HeaderValue::Text(new_value);
                    new_branch // Return the branch we just added
                } else {
                    // No Via header found, add a new one with branch
                    debug!("No Via header found, adding one with branch parameter");
                    
                    // Use localhost as placeholder - in real scenarios the correct local interface would be used
                    let via_value = format!("SIP/2.0/UDP 127.0.0.1:5060;branch={}", new_branch);
                    request.headers.push(Header::text(HeaderName::Via, via_value));
                    
                    new_branch // Return the branch we just added
                }
            }
        };
        
        // Ensure we have a Max-Forwards header
        if !request.headers.iter().any(|h| h.name == HeaderName::MaxForwards) {
            debug!("Adding missing Max-Forwards header");
            request.headers.push(Header::integer(HeaderName::MaxForwards, 70));
        }
        
        let id = format!("ict_{}", branch);
        debug!("Created transaction with ID: {}", id);
        
        Ok(ClientInviteTransaction {
            id,
            state: TransactionState::Initial,
            request,
            last_response: None,
            remote_addr,
            transport,
            timer_a: Duration::from_millis(500), // T1 (500ms)
            timer_b: Duration::from_secs(32),    // 64*T1 seconds
            timer_d: Duration::from_secs(32),     // > 32s for unreliable transport
            retransmit_count: 0,
        })
    }
    
    /// Get the remote address
    pub fn remote_addr(&self) -> SocketAddr {
        self.remote_addr
    }
    
    /// Transition to a new state
    fn transition_to(&mut self, new_state: TransactionState) -> Result<()> {
        debug!("[{}] State transition: {:?} -> {:?}", self.id, self.state, new_state);
        
        // Validate state transition
        match (self.state, new_state) {
            // Valid transitions
            (TransactionState::Initial, TransactionState::Calling) => {},
            (TransactionState::Calling, TransactionState::Proceeding) => {},
            (TransactionState::Calling, TransactionState::Completed) => {},
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
    
    /// Create an ACK request for a response
    fn create_ack(&self, response: &Response) -> Result<Request> {
        // Create ACK based on original request and received response
        let mut ack_request = Request::new(Method::Ack, self.request.uri.clone());
        ack_request.version = self.request.version.clone(); // Clone Version

        // Copy essential headers (To, From, Call-ID, CSeq Method=ACK)
        if let Some(to_header) = response.header(&HeaderName::To) {
            ack_request = ack_request.with_header(to_header.clone());
        }
        if let Some(from_header) = self.request.header(&HeaderName::From) {
            ack_request = ack_request.with_header(from_header.clone());
        }
        if let Some(call_id_header) = self.request.header(&HeaderName::CallId) {
            ack_request = ack_request.with_header(call_id_header.clone());
        }
        if let Some((cseq_num, _)) = utils::extract_cseq(&Message::Request(self.request.clone())) {
            ack_request = ack_request.with_header(Header::text(
                HeaderName::CSeq,
                format!("{} ACK", cseq_num)
            ));
        }
        
        // Add Content-Length header if not present
        ack_request = ack_request.clone().with_header(Header::integer(
            HeaderName::ContentLength,
            ack_request.body.len() as i64
        ));
        
        Ok(ack_request)
    }

    /// Create an ACK request for a 2xx response
    pub fn create_2xx_ack(&self, response: &Response) -> Result<Request> {
        // Extract Request-URI from Contact header in the response
        let request_uri = if let Some(contact) = response.header(&HeaderName::Contact) {
            if let Some(contact_text) = contact.value.as_text() {
                // Try to extract URI
                if let Some(uri_start) = contact_text.find('<') {
                    let uri_start = uri_start + 1;
                    if let Some(uri_end) = contact_text[uri_start..].find('>') {
                        let uri_str = &contact_text[uri_start..(uri_start + uri_end)];
                        
                        // Try to parse URI
                        match Uri::from_str(uri_str) {
                            Ok(uri) => uri,
                            Err(_) => self.request.uri.clone() // Fallback to original request URI
                        }
                    } else {
                        self.request.uri.clone() // Fallback
                    }
                } else {
                    self.request.uri.clone() // Fallback
                }
            } else {
                self.request.uri.clone() // Fallback
            }
        } else {
            self.request.uri.clone() // Fallback
        };
        
        // Create base request
        let mut ack = Request {
            method: Method::Ack,
            uri: request_uri,
            version: self.request.version.clone(),
            headers: Vec::new(),
            body: bytes::Bytes::new(), // ACK typically has no body
        };
        
        // Copy headers from original request
        for header in &self.request.headers {
            if matches!(header.name, 
                HeaderName::From | 
                HeaderName::CallId | 
                HeaderName::Route | 
                HeaderName::MaxForwards
            ) {
                ack = ack.with_header(header.clone());
            }
        }
        
        // Add Via header with new branch (ACK for 2xx is a separate transaction)
        let branch = format!("z9hG4bK-{}", Uuid::new_v4());
        for via in self.request.headers.iter().filter(|h| h.name == HeaderName::Via) {
            if let Some(via_text) = via.value.as_text() {
                let via_parts: Vec<&str> = via_text.split(';').collect();
                if !via_parts.is_empty() {
                    // Keep the protocol and address part, but generate a new branch
                    ack = ack.with_header(Header::text(
                        HeaderName::Via,
                        format!("{};branch={}", via_parts[0], branch)
                    ));
                    break;
                }
            }
        }
        
        // Use To header from response (with tag)
        if let Some(to_header) = response.header(&HeaderName::To) {
            ack = ack.with_header(to_header.clone());
        }
        
        // Update CSeq for ACK
        if let Some((cseq_num, _)) = utils::extract_cseq(&Message::Request(self.request.clone())) {
            ack = ack.with_header(Header::text(
                HeaderName::CSeq,
                format!("{} ACK", cseq_num)
            ));
        }
        
        // Add Content-Length header
        ack = ack.clone().with_header(Header::integer(
            HeaderName::ContentLength,
            0
        ));
        
        Ok(ack)
    }
}

#[async_trait::async_trait]
impl Transaction for ClientInviteTransaction {
    fn id(&self) -> &str {
        &self.id
    }
    
    fn transaction_type(&self) -> TransactionType {
        TransactionType::Client
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
            Message::Request(_) => {
                warn!("[{}] Received request in client transaction", self.id);
                // Client transactions don't process requests
                Ok(None)
            },
            Message::Response(_response) => {
                if self.state == TransactionState::Terminated {
                    // In terminated state, we ignore responses
                    trace!("[{}] Client transaction already terminated, ignoring response", self.id);
                    return Ok(None);
                }

                let status = _response.status;
                
                match self.state {
                    TransactionState::Calling => {
                        if status.is_provisional() {
                            debug!("[{}] Received provisional response: {}", self.id, status);
                            self.transition_to(TransactionState::Proceeding)?;
                            
                            // Store response
                            self.last_response = Some(_response);
                            
                            // Continue with timer B (request timeout)
                            Ok(None)
                        } else if status.is_success() || status.is_error() {
                            debug!("[{}] Received final response: {}", self.id, status);
                            self.transition_to(TransactionState::Completed)?;
                            
                            // Store response
                            self.last_response = Some(_response.clone());
                            
                            // Create and send ACK for non-2xx responses
                            if !status.is_success() {
                                let ack = self.create_ack(&_response)?;
                                debug!("[{}] Sending ACK for non-2xx response", self.id);
                                self.transport.send_message(ack.into(), self.remote_addr).await?;
                                
                                // Start timer D
                                // We'll handle this in the transaction manager with on_timeout
                            }
                            
                            // For 2xx responses, the core/TU will send the ACK
                            // (outside of transaction layer)
                            
                            Ok(None)
                        } else {
                            warn!("[{}] Received invalid response: {}", self.id, status);
                            Ok(None)
                        }
                    },
                    TransactionState::Proceeding => {
                        if status.is_provisional() {
                            debug!("[{}] Received additional provisional response: {}", self.id, status);
                            
                            // Store latest provisional response
                            self.last_response = Some(_response);
                            
                            Ok(None)
                        } else if status.is_success() || status.is_error() {
                            debug!("[{}] Received final response: {}", self.id, status);
                            self.transition_to(TransactionState::Completed)?;
                            
                            // Store response
                            self.last_response = Some(_response.clone());
                            
                            // Create and send ACK for non-2xx responses
                            if !status.is_success() {
                                let ack = self.create_ack(&_response)?;
                                debug!("[{}] Sending ACK for non-2xx response", self.id);
                                self.transport.send_message(ack.into(), self.remote_addr).await?;
                                
                                // Start timer D
                                // We'll handle this in the transaction manager with on_timeout
                            }
                            
                            // For 2xx responses, the core/TU will send the ACK
                            // (outside of transaction layer)
                            
                            Ok(None)
                        } else {
                            warn!("[{}] Received invalid response: {}", self.id, status);
                            Ok(None)
                        }
                    },
                    TransactionState::Completed => {
                        if !status.is_success() {
                            // Retransmission of final response, resend ACK
                            debug!("[{}] Received retransmission of final non-2xx response, resending ACK", self.id);
                            
                            let ack = self.create_ack(&_response)?;
                            self.transport.send_message(ack.into(), self.remote_addr).await?;
                            
                            Ok(None)
                        } else {
                            // 2xx responses are handled by TU/core directly
                            debug!("[{}] Received 2xx response in COMPLETED state", self.id);
                            
                            // Store latest response
                            self.last_response = Some(_response);
                            
                            Ok(None)
                        }
                    },
                    _ => {
                        warn!("[{}] Received response in invalid state: {:?}", self.id, self.state);
                        Ok(None)
                    }
                }
            }
        }
    }
    
    fn matches(&self, message: &Message) -> bool {
        if let Message::Response(_response) = message {
            // Extract CSeq and method
            if let Some((_, method)) = utils::extract_cseq(message) {
                // Match method with our request
                if method != self.request.method {
                    return false;
                }
                
                // Check if branch and call-id match
                if let (Some(incoming_branch), Some(our_branch)) = (
                    message.first_via().and_then(|via| via.get("branch").flatten().map(|s| s.to_string())),
                    utils::extract_branch(&Message::Request(self.request.clone()))
                ) {
                    if incoming_branch == our_branch {
                        return true;
                    }
                }
                
                // Fall back to checking call-id
                if let (Some(call_id), Some(our_call_id)) = (
                    utils::extract_call_id(message),
                    utils::extract_call_id(&Message::Request(self.request.clone()))
                ) {
                    return call_id == our_call_id;
                }
            }
        }
        
        false
    }
    
    fn timeout_duration(&self) -> Option<Duration> {
        match self.state {
            TransactionState::Calling => Some(self.timer_a),
            TransactionState::Completed => Some(self.timer_d),
            _ => None,
        }
    }
    
    async fn on_timeout(&mut self) -> Result<Option<Message>> {
        match self.state {
            TransactionState::Calling => {
                // Timer A fired - retransmit INVITE
                debug!("[{}] Timer A fired, retransmitting INVITE", self.id);
                
                // Send request again
                self.transport.send_message(self.request.clone().into(), self.remote_addr).await?;
                
                // Double retransmission interval (exponential backoff)
                self.timer_a = Duration::min(
                    self.timer_a * 2,
                    Duration::from_secs(4)
                );
                
                self.retransmit_count += 1;
                
                // Check if we've hit timer B
                if self.retransmit_count > 10 { // Arbitrary limit for now, would use actual timer in production
                    debug!("[{}] Timer B fired, no response received, terminating transaction", self.id);
                    
                    // Typically we'd switch to TERMINATED, but for INVITE we want to go through
                    // COMPLETED to handle any late responses properly
                    self.transition_to(TransactionState::Completed)?;
                    
                    // Create a 408 response to indicate timeout
                    let mut timeout_response = Response::new(StatusCode::RequestTimeout);
                    
                    // Add basic headers
                    for header in &self.request.headers {
                        if matches!(header.name, 
                            HeaderName::From | 
                            HeaderName::To | 
                            HeaderName::CallId | 
                            HeaderName::CSeq
                        ) {
                            timeout_response = timeout_response.with_header(header.clone());
                        }
                    }
                    
                    timeout_response = timeout_response.with_header(Header::integer(
                        HeaderName::ContentLength, 0
                    ));
                    
                    self.last_response = Some(timeout_response.clone());
                    
                    // Return the timeout response to be handled by upper layers
                    return Ok(Some(Message::Response(timeout_response)));
                }
                
                Ok(None)
            },
            TransactionState::Completed => {
                // Timer D fired - terminate transaction
                debug!("[{}] Timer D fired, terminating transaction", self.id);
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
impl ClientTransaction for ClientInviteTransaction {
    async fn send_request(&mut self) -> Result<()> {
        match self.state {
            TransactionState::Initial => {
                debug!("[{}] Sending initial INVITE request", self.id);
                self.transition_to(TransactionState::Calling)?;
                
                // Send request
                self.transport.send_message(self.request.clone().into(), self.remote_addr).await?;
                
                // Start timer A (retransmission)
                // We'll handle this in the transaction manager with on_timeout
                
                // Start timer B (transaction timeout)
                // We'll handle this in the transaction manager with on_timeout
                
                Ok(())
            },
            _ => {
                Err(Error::InvalidStateTransition(
                    format!("Cannot send request in {:?} state", self.state)
                ))
            }
        }
    }
}

/// Client non-INVITE transaction
#[derive(Debug)]
pub struct ClientNonInviteTransaction {
    /// Transaction ID
    id: String,
    /// Current state
    state: TransactionState,
    /// Original request
    request: Request,
    /// Last response received
    last_response: Option<Response>,
    /// Remote address (where to send requests)
    remote_addr: SocketAddr,
    /// Transport to use for sending requests
    transport: Arc<dyn Transport>,
    /// Timer E duration (non-INVITE retransmission interval)
    timer_e: Duration,
    /// Timer F duration (non-INVITE timeout)
    timer_f: Duration,
    /// Timer K duration (wait time for response retransmissions)
    timer_k: Duration,
    /// Retransmission count
    retransmit_count: u32,
}

impl ClientNonInviteTransaction {
    /// Create a new client non-INVITE transaction
    pub fn new(
        mut request: Request,
        remote_addr: SocketAddr,
        transport: Arc<dyn Transport>,
    ) -> Result<Self> {
        // Ensure the request is not an INVITE
        if request.method == Method::Invite {
            return Err(Error::Other("Request must not be INVITE for non-INVITE client transaction".to_string()));
        }
        
        // Check if request has a Via header with branch parameter
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
                    via_header.value = HeaderValue::Text(new_value);
                    new_branch // Return the branch we just added
                } else {
                    // No Via header found, add a new one with branch
                    debug!("No Via header found, adding one with branch parameter");
                    
                    // Use localhost as placeholder - in real scenarios the correct local interface would be used
                    let via_value = format!("SIP/2.0/UDP 127.0.0.1:5060;branch={}", new_branch);
                    request.headers.push(Header::text(HeaderName::Via, via_value));
                    
                    new_branch // Return the branch we just added
                }
            }
        };
        
        // Ensure we have a Max-Forwards header
        if !request.headers.iter().any(|h| h.name == HeaderName::MaxForwards) {
            debug!("Adding missing Max-Forwards header");
            request.headers.push(Header::integer(HeaderName::MaxForwards, 70));
        }
        
        let id = format!("nict_{}", branch);
        debug!("Created transaction with ID: {}", id);
        
        Ok(ClientNonInviteTransaction {
            id,
            state: TransactionState::Initial,
            request,
            last_response: None,
            remote_addr,
            transport,
            timer_e: Duration::from_millis(500),  // T1 (500ms)
            timer_f: Duration::from_secs(32),     // 64*T1 seconds
            timer_k: Duration::from_secs(5),      // 5s for unreliable transport
            retransmit_count: 0,
        })
    }
    
    /// Create an ACK request for a non-2xx response
    fn create_ack(&self, _response: &Response) -> Result<Request> {
        // Only INVITE transactions should send ACKs
        // This method should never be called for non-INVITE transactions
        Err(Error::Other("Non-INVITE transactions do not send ACKs".to_string()))
    }
    
    /// Transition to a new state
    fn transition_to(&mut self, new_state: TransactionState) -> Result<()> {
        debug!("[{}] State transition: {:?} -> {:?}", self.id, self.state, new_state);
        
        // Validate state transition
        match (self.state, new_state) {
            // Valid transitions
            (TransactionState::Initial, TransactionState::Trying) => {},
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
impl Transaction for ClientNonInviteTransaction {
    fn id(&self) -> &str {
        &self.id
    }
    
    fn transaction_type(&self) -> TransactionType {
        TransactionType::Client
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
            Message::Request(_) => {
                warn!("[{}] Received request in client transaction", self.id);
                // Client transactions don't process requests
                Ok(None)
            },
            Message::Response(_response) => {
                if self.state == TransactionState::Terminated {
                    // In terminated state, we ignore responses
                    trace!("[{}] Client transaction already terminated, ignoring response", self.id);
                    return Ok(None);
                }

                let status = _response.status;
                
                match self.state {
                    TransactionState::Trying => {
                        if status.is_provisional() {
                            debug!("[{}] Received provisional response: {}", self.id, status);
                            self.transition_to(TransactionState::Proceeding)?;
                            
                            // Store response
                            self.last_response = Some(_response);
                            
                            Ok(None)
                        } else if status.is_success() || status.is_error() {
                            debug!("[{}] Received final response: {}", self.id, status);
                            self.transition_to(TransactionState::Completed)?;
                            
                            // Store response
                            self.last_response = Some(_response.clone());
                            
                            Ok(None)
                        } else {
                            warn!("[{}] Received invalid response: {}", self.id, status);
                            Ok(None)
                        }
                    },
                    TransactionState::Proceeding => {
                        if status.is_provisional() {
                            debug!("[{}] Received additional provisional response: {}", self.id, status);
                            
                            // Store latest provisional response
                            self.last_response = Some(_response);
                            
                            Ok(None)
                        } else if status.is_success() || status.is_error() {
                            debug!("[{}] Received final response: {}", self.id, status);
                            self.transition_to(TransactionState::Completed)?;
                            
                            // Store response
                            self.last_response = Some(_response.clone());
                            
                            Ok(None)
                        } else {
                            warn!("[{}] Received invalid response: {}", self.id, status);
                            Ok(None)
                        }
                    },
                    TransactionState::Completed => {
                        if !status.is_success() {
                            // Retransmission of final response, resend ACK
                            debug!("[{}] Received retransmission of final non-2xx response, resending ACK", self.id);
                            
                            let ack = self.create_ack(&_response)?;
                            self.transport.send_message(ack.into(), self.remote_addr).await?;
                            
                            Ok(None)
                        } else {
                            // 2xx responses are handled by TU/core directly
                            debug!("[{}] Received 2xx response in COMPLETED state", self.id);
                            
                            // Store latest response
                            self.last_response = Some(_response);
                            
                            Ok(None)
                        }
                    },
                    _ => {
                        warn!("[{}] Received response in invalid state: {:?}", self.id, self.state);
                        Ok(None)
                    }
                }
            }
        }
    }
    
    fn matches(&self, message: &Message) -> bool {
        if let Message::Response(_response) = message {
            // Extract CSeq and method
            if let Some((_, method)) = utils::extract_cseq(message) {
                // Match method with our request
                if method != self.request.method {
                    return false;
                }
                
                // Check if branch and call-id match
                if let (Some(incoming_branch), Some(our_branch)) = (
                    message.first_via().and_then(|via| via.get("branch").flatten().map(|s| s.to_string())),
                    utils::extract_branch(&Message::Request(self.request.clone()))
                ) {
                    if incoming_branch == our_branch {
                        return true;
                    }
                }
                
                // Fall back to checking call-id
                if let (Some(call_id), Some(our_call_id)) = (
                    utils::extract_call_id(message),
                    utils::extract_call_id(&Message::Request(self.request.clone()))
                ) {
                    return call_id == our_call_id;
                }
            }
        }
        
        false
    }
    
    fn timeout_duration(&self) -> Option<Duration> {
        match self.state {
            TransactionState::Trying | TransactionState::Proceeding => Some(self.timer_e),
            TransactionState::Completed => Some(self.timer_k),
            _ => None,
        }
    }
    
    async fn on_timeout(&mut self) -> Result<Option<Message>> {
        match self.state {
            TransactionState::Trying | TransactionState::Proceeding => {
                // Timer E fired - retransmit request
                debug!("[{}] Timer E fired, retransmitting request", self.id);
                
                // Send request again
                self.transport.send_message(self.request.clone().into(), self.remote_addr).await?;
                
                // Double retransmission interval (exponential backoff)
                self.timer_e = Duration::min(
                    self.timer_e * 2,
                    Duration::from_secs(4)
                );
                
                self.retransmit_count += 1;
                
                // Check if we've hit timer F
                if self.retransmit_count > 10 { // Arbitrary limit for now, would use actual timer in production
                    debug!("[{}] Timer F fired, no final response received, terminating transaction", self.id);
                    self.transition_to(TransactionState::Completed)?;
                    
                    // Create a 408 response to indicate timeout
                    let mut timeout_response = Response::new(StatusCode::RequestTimeout);
                    
                    // Add basic headers
                    for header in &self.request.headers {
                        if matches!(header.name, 
                            HeaderName::From | 
                            HeaderName::To | 
                            HeaderName::CallId | 
                            HeaderName::CSeq
                        ) {
                            timeout_response = timeout_response.with_header(header.clone());
                        }
                    }
                    
                    timeout_response = timeout_response.with_header(Header::integer(
                        HeaderName::ContentLength, 0
                    ));
                    
                    self.last_response = Some(timeout_response.clone());
                    
                    // Start timer K
                    // We'll handle this in the transaction manager
                    
                    // Return the timeout response to be handled by upper layers
                    return Ok(Some(Message::Response(timeout_response)));
                }
                
                Ok(None)
            },
            TransactionState::Completed => {
                // Timer K fired - terminate transaction
                debug!("[{}] Timer K fired, terminating transaction", self.id);
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
impl ClientTransaction for ClientNonInviteTransaction {
    async fn send_request(&mut self) -> Result<()> {
        match self.state {
            TransactionState::Initial => {
                debug!("[{}] Sending initial {} request", self.id, self.request.method);
                self.transition_to(TransactionState::Trying)?;
                
                // Send request
                self.transport.send_message(self.request.clone().into(), self.remote_addr).await?;
                
                // Start timer E (retransmission)
                // We'll handle this in the transaction manager with on_timeout
                
                // Start timer F (transaction timeout)
                // We'll handle this in the transaction manager with on_timeout
                
                Ok(())
            },
            _ => {
                Err(Error::InvalidStateTransition(
                    format!("Cannot send request in {:?} state", self.state)
                ))
            }
        }
    }
} 