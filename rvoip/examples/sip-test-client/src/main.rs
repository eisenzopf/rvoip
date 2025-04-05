use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, Context, anyhow};
use clap::Parser;
use tokio::sync::{mpsc, oneshot};
use tokio::signal::ctrl_c;
use tracing::{info, debug, error, warn};
use uuid::Uuid;
use bytes::Bytes;

// Rust converts "-" to "_" when importing crates
use rvoip_sip_core as _;
use rvoip_sip_transport as _;
use rvoip_transaction_core as _;
use rvoip_session_core as _;

// Now import the specific types we need
use rvoip_sip_core::{
    Request, Response, Message, Method, StatusCode, 
    Uri, Header, HeaderName, HeaderValue
};
use rvoip_sip_transport::UdpTransport;
use rvoip_transaction_core::{TransactionManager, TransactionEvent};

// Import user agent module
mod user_agent;

/// SIP Test Client
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Operating mode. Use:
    /// - 'ua' to run as a User Agent that listens for incoming calls
    /// - 'call' to make an outgoing call to a target
    #[arg(short, long, default_value = "ua")]
    mode: String,
    
    /// Local address to bind to
    #[arg(short, long, default_value = "127.0.0.1:5070")]
    local_addr: String,
    
    /// Client username
    #[arg(short, long, default_value = "alice")]
    username: String,
    
    /// Server domain
    #[arg(short, long, default_value = "rvoip.local")]
    domain: String,
    
    /// Remote address to send requests to (only needed in 'call' mode)
    #[arg(short, long, default_value = "127.0.0.1:5060")]
    server_addr: String,
    
    /// Target URI for call mode (only needed in 'call' mode)
    /// Example: sip:bob@rvoip.local
    #[arg(short, long)]
    target_uri: Option<String>,
}

/// SIP test client
struct SipClient {
    /// Local address
    local_addr: SocketAddr,
    
    /// Server address
    server_addr: SocketAddr,
    
    /// Username
    username: String,
    
    /// Domain
    domain: String,
    
    /// Transaction manager
    transaction_manager: Arc<TransactionManager>,
    
    /// Events channel receiver
    events_rx: mpsc::Receiver<TransactionEvent>,
    
    /// Call-ID for this session
    call_id: String,
    
    /// Tag for From header
    tag: String,
    
    /// CSeq counter
    cseq: u32,
    
    /// Pending transaction responses
    pending_responses: Arc<tokio::sync::Mutex<std::collections::HashMap<String, oneshot::Sender<Response>>>>,
    
    /// Is the event processor running
    event_processor_running: Arc<tokio::sync::Mutex<bool>>,
}

impl Clone for SipClient {
    fn clone(&self) -> Self {
        // Use the new subscribe method to get a new events_rx
        let events_rx = self.transaction_manager.subscribe();
        
        Self {
            local_addr: self.local_addr,
            server_addr: self.server_addr,
            username: self.username.clone(),
            domain: self.domain.clone(),
            transaction_manager: self.transaction_manager.clone(),
            events_rx,
            call_id: self.call_id.clone(),
            tag: self.tag.clone(),
            cseq: self.cseq,
            pending_responses: self.pending_responses.clone(),
            event_processor_running: self.event_processor_running.clone(),
        }
    }
}

impl SipClient {
    /// Create a new SIP client
    async fn new(
        local_addr: SocketAddr,
        server_addr: SocketAddr,
        username: String,
        domain: String,
    ) -> Result<Self> {
        // Create UDP transport
        let (udp_transport, transport_rx) = UdpTransport::bind(local_addr, None).await
            .context("Failed to bind UDP transport")?;
        
        info!("UDP transport bound to {}", local_addr);
        
        // Wrap transport in Arc
        let arc_transport = Arc::new(udp_transport);
        
        // Create transaction manager
        let (transaction_manager, events_rx) = TransactionManager::new(
            arc_transport,
            transport_rx,
            None, // Use default event capacity
        ).await.context("Failed to create transaction manager")?;
        
        // Generate Call-ID and tag
        let call_id = format!("{}-{}", username, Uuid::new_v4());
        let tag = format!("tag-{}", Uuid::new_v4());
        
        // Create pending responses map
        let pending_responses = Arc::new(tokio::sync::Mutex::new(
            std::collections::HashMap::new()
        ));
        
        info!("SIP client initialized with Call-ID: {}", call_id);
        
        Ok(Self {
            local_addr,
            server_addr,
            username,
            domain,
            transaction_manager: Arc::new(transaction_manager),
            events_rx,
            call_id,
            tag,
            cseq: 1,
            pending_responses,
            event_processor_running: Arc::new(tokio::sync::Mutex::new(true)),
        })
    }
    
    /// Generate a new branch parameter
    fn new_branch(&self) -> String {
        format!("z9hG4bK-{}", Uuid::new_v4())
    }
    
    /// Create a new request
    fn create_request(&self, method: Method, uri: Uri) -> Request {
        let method_clone = method.clone();
        let mut request = Request::new(method, uri.clone());
        
        // Add Via header with branch parameter
        let branch = format!("z9hG4bK-{}", Uuid::new_v4());
        let via_value = format!("SIP/2.0/UDP {};branch={}", self.local_addr, branch);
        request.headers.push(Header::text(HeaderName::Via, via_value));
        
        // Add Max-Forwards
        request.headers.push(Header::integer(HeaderName::MaxForwards, 70));
        
        // Add From header with tag
        let from_value = format!("<sip:{}@{}>;tag={}", self.username, self.domain, self.tag);
        request.headers.push(Header::text(HeaderName::From, from_value));
        
        // Add To header
        let to_value = format!("<{}>", uri);
        request.headers.push(Header::text(HeaderName::To, to_value));
        
        // Add Call-ID
        request.headers.push(Header::text(HeaderName::CallId, self.call_id.clone()));
        
        // Add CSeq
        request.headers.push(Header::text(HeaderName::CSeq, format!("{} {}", self.cseq, method_clone)));
        
        // Add Contact
        let contact_value = format!("<sip:{}@{}>", self.username, self.local_addr);
        request.headers.push(Header::text(HeaderName::Contact, contact_value));
        
        // Add User-Agent
        request.headers.push(Header::text(HeaderName::UserAgent, "RVOIP-Test-Client/0.1.0"));
        
        request
    }
    
    /// Send a REGISTER request
    async fn register(&mut self) -> Result<Response> {
        info!("Sending REGISTER request");
        
        // Create request URI for REGISTER (domain)
        let request_uri = format!("sip:{}", self.domain).parse()
            .context("Invalid domain URI")?;
        
        // Create REGISTER request
        let mut request = self.create_request(Method::Register, request_uri);
        
        // Add Expires header
        request.headers.push(Header::text(HeaderName::Expires, "3600"));
        
        // Add Content-Length
        request.headers.push(Header::text(HeaderName::ContentLength, "0"));
        
        // Send request and wait for response
        let response = self.send_request(request).await?;
        
        if response.status == StatusCode::Ok {
            info!("Registration successful");
            Ok(response)
        } else if response.status == StatusCode::Unauthorized {
            info!("Authentication required, sending authenticated request");
            
            // Extract authentication parameters
            let www_auth = response.headers.iter()
                .find(|h| h.name == HeaderName::WwwAuthenticate)
                .ok_or_else(|| anyhow!("Missing WWW-Authenticate header"))?;
            
            let www_auth_text = www_auth.value.as_text()
                .ok_or_else(|| anyhow!("Invalid WWW-Authenticate header"))?;
            
            // Parse realm and nonce
            let realm = parse_auth_param(www_auth_text, "realm")
                .ok_or_else(|| anyhow!("Missing realm in WWW-Authenticate header"))?;
            
            let nonce = parse_auth_param(www_auth_text, "nonce")
                .ok_or_else(|| anyhow!("Missing nonce in WWW-Authenticate header"))?;
            
            info!("Got auth challenge - realm: {}, nonce: {}", realm, nonce);
            
            // Create a new REGISTER request with auth
            let request_uri = format!("sip:{}", self.domain).parse()
                .context("Invalid domain URI")?;
            
            let mut request = self.create_request(Method::Register, request_uri);
            
            // Add Expires header
            request.headers.push(Header::text(HeaderName::Expires, "3600"));
            
            // Add Authorization header
            // In a real client, we would calculate a proper digest response
            // Here we're just providing the expected header format
            let password = "password123"; // In a real client, this would be securely stored
            let auth_response = format!("{:x}", md5::compute(
                format!("{}:{}:{}", self.username, realm, password)
            ));
            
            let auth_value = format!(
                "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"sip:{}\", response=\"{}\", algorithm=MD5",
                self.username, realm, nonce, self.domain, auth_response
            );
            
            request.headers.push(Header::text(HeaderName::Authorization, auth_value));
            
            // Add Content-Length
            request.headers.push(Header::text(HeaderName::ContentLength, "0"));
            
            // Send request and wait for response
            let auth_response = self.send_request(request).await?;
            
            if auth_response.status == StatusCode::Ok {
                info!("Authenticated registration successful");
            } else {
                warn!("Authentication failed: {:?}", auth_response);
            }
            
            Ok(auth_response)
        } else {
            warn!("Unexpected response: {:?}", response);
            Ok(response)
        }
    }
    
    /// Make a call to a target URI
    async fn make_call(&mut self, target: &str) -> Result<()> {
        info!("Making call to {}", target);
        
        // Parse target URI
        let request_uri: Uri = target.parse().context("Invalid target URI")?;
        
        // Create INVITE request
        let mut request = self.create_request(Method::Invite, request_uri.clone());
        
        // Add SDP content
        let sdp = format!(
            "v=0\r\n\
             o={} 123456 789012 IN IP4 {}\r\n\
             s=Call\r\n\
             c=IN IP4 {}\r\n\
             t=0 0\r\n\
             m=audio 10000 RTP/AVP 0\r\n\
             a=rtpmap:0 PCMU/8000\r\n\
             a=sendrecv\r\n",
            self.username, self.local_addr.ip(), self.local_addr.ip()
        );
        
        // Add Content-Type
        request.headers.push(Header::text(HeaderName::ContentType, "application/sdp"));
        
        // Add Content-Length
        request.headers.push(Header::text(
            HeaderName::ContentLength, 
            sdp.len().to_string()
        ));
        
        // Add SDP body
        request.body = sdp.into();
        
        // Send INVITE and wait for response
        info!("Sending INVITE");
        let invite_response = self.send_request(request).await?;
        
        if invite_response.status == StatusCode::Ok {
            info!("Call accepted, sending ACK");
            
            // Extract To tag from response
            let to_header = invite_response.headers.iter()
                .find(|h| h.name == HeaderName::To)
                .ok_or_else(|| anyhow!("Missing To header in response"))?;
            
            let to_text = to_header.value.as_text()
                .ok_or_else(|| anyhow!("Invalid To header format"))?;
            
            // Create ACK request
            let mut ack_request = self.create_request(Method::Ack, request_uri.clone());
            
            // For ACK, we need to use the same CSeq number as the INVITE but with ACK method
            ack_request.headers.iter_mut()
                .find(|h| h.name == HeaderName::CSeq)
                .map(|h| h.value = HeaderValue::Text(format!("{} ACK", self.cseq - 1)));
            
            // Use the To header with tag from the 200 OK response
            ack_request.headers.iter_mut()
                .find(|h| h.name == HeaderName::To)
                .map(|h| h.value = HeaderValue::Text(to_text.to_string()));
            
            // Add Content-Length
            ack_request.headers.push(Header::text(HeaderName::ContentLength, "0"));
            
            // Send ACK directly (ACK is end-to-end, not transaction based)
            let message = Message::Request(ack_request);
            self.transaction_manager.transport().send_message(message, self.server_addr).await
                .context("Failed to send ACK")?;
            
            // Wait a bit before hanging up
            tokio::time::sleep(Duration::from_secs(3)).await;
            
            // Send BYE to hang up
            info!("Hanging up, sending BYE");
            let mut bye_request = self.create_request(Method::Bye, request_uri);
            
            // Use the To header with tag from the 200 OK response
            bye_request.headers.iter_mut()
                .find(|h| h.name == HeaderName::To)
                .map(|h| h.value = HeaderValue::Text(to_text.to_string()));
            
            // Add Content-Length
            bye_request.headers.push(Header::text(HeaderName::ContentLength, "0"));
            
            // Send BYE and wait for response
            let bye_response = self.send_request(bye_request).await?;
            
            if bye_response.status == StatusCode::Ok {
                info!("Call ended successfully");
            } else {
                warn!("Unexpected BYE response: {:?}", bye_response);
            }
        } else {
            warn!("Call failed: {:?}", invite_response);
        }
        
        Ok(())
    }
    
    /// Process transaction events
    async fn process_events(&mut self) -> Result<()> {
        info!("Starting event processing for SIP client on {}", self.local_addr);
        debug!("Pending response channels map initialized, will track transaction completions");
        
        // Set the event processor as running
        *self.event_processor_running.lock().await = true;
        
        // Process events from our channel
        info!("Listening for transaction events...");
        loop {
            // Check if we're asked to stop
            if !*self.event_processor_running.lock().await {
                info!("Event processor stopping due to external request");
                break;
            }
            
            // Wait for the next event with a timeout to allow checking if we should stop
            let event = tokio::select! {
                event = self.events_rx.recv() => {
                    match event {
                        Some(e) => e,
                        None => {
                            // Channel closed, but this shouldn't happen unless TransactionManager is dropped
                            warn!("Transaction event channel closed, stopping event processor");
                            break;
                        }
                    }
                },
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    // Timeout - just check running state again
                    continue;
                }
            };
            
            debug!("Received transaction event: {:?}", event);
            
            match event {
                TransactionEvent::TransactionCreated { transaction_id } => {
                    debug!("Transaction created: {}", transaction_id);
                },
                TransactionEvent::TransactionCompleted { transaction_id, response } => {
                    warn!("Transaction completed: {} with response: {:?}", 
                          transaction_id, response.as_ref().map(|r| r.status));
                    
                    // Dump contents of pending_responses
                    let mut pending = self.pending_responses.lock().await;
                    let pending_keys = pending.keys().cloned().collect::<Vec<_>>();
                    debug!("Current pending transactions: {:?}", pending_keys);
                    
                    if let Some(resp) = response {
                        warn!("Delivering response for transaction {}, status: {}", 
                             transaction_id, resp.status);
                        
                        // Check if we have a pending channel for this transaction
                        if let Some(tx) = pending.remove(&transaction_id) {
                            warn!("Found channel for transaction {}, sending response", transaction_id);
                            match tx.send(resp) {
                                Ok(_) => debug!("Response sent to waiting request handler"),
                                Err(e) => error!("Failed to send response: {:?}", e),
                            }
                        } else {
                            warn!("No waiting channel found for transaction: {}", transaction_id);
                        }
                    } else {
                        warn!("Transaction {} completed with no response", transaction_id);
                        
                        // Check if we have a pending channel for this transaction
                        if let Some(tx) = pending.remove(&transaction_id) {
                            let mut timeout_response = Response::new(StatusCode::RequestTimeout);
                            timeout_response.headers.push(Header::text(HeaderName::CallId, self.call_id.clone()));
                            timeout_response.headers.push(Header::text(HeaderName::CSeq, format!("{} {}", self.cseq, Method::Invite)));
                            
                            warn!("Sending timeout response for transaction {}", transaction_id);
                            if let Err(e) = tx.send(timeout_response) {
                                error!("Failed to send timeout response: {:?}", e);
                            }
                        }
                    }
                },
                TransactionEvent::TransactionTerminated { transaction_id } => {
                    debug!("Transaction terminated: {}", transaction_id);
                    // Remove any pending channel for this transaction
                    if let Some(_) = self.pending_responses.lock().await.remove(&transaction_id) {
                        debug!("Removed pending channel for terminated transaction {}", transaction_id);
                    }
                },
                TransactionEvent::UnmatchedMessage { message, source } => {
                    match message {
                        Message::Request(request) => {
                            info!("Received request: {} from {}", request.method, source);
                            
                            // Extract important headers for debugging
                            let call_id = request.call_id().unwrap_or("unknown");
                            let from = request.header(&HeaderName::From)
                                .and_then(|h| h.value.as_text())
                                .unwrap_or("unknown");
                            let to = request.header(&HeaderName::To)
                                .and_then(|h| h.value.as_text())
                                .unwrap_or("unknown");
                                
                            info!("Request details - Call-ID: {}, From: {}, To: {}", call_id, from, to);
                            
                            // Handle the request and send a response if needed
                            if let Err(e) = self.handle_request(request, source).await {
                                error!("Error handling request: {}", e);
                            }
                        },
                        Message::Response(response) => {
                            info!("Received unmatched response: {} from {}", response.status, source);
                            
                            // This could be a late response where transaction was already removed
                            // Check if we can find a matching channel based on headers
                            if let Some(call_id) = response.header(&HeaderName::CallId)
                                .and_then(|h| h.value.as_text()) {
                                
                                // For now we just log; in a real client we would try to match
                                // this response to an ongoing dialog
                                info!("Unmatched response for Call-ID: {}", call_id);
                            }
                        }
                    }
                },
                TransactionEvent::Error { error, transaction_id } => {
                    error!("Transaction error: {}, id: {:?}", error, transaction_id);
                    
                    // If there's an associated transaction, remove any pending channel
                    if let Some(id) = transaction_id {
                        if let Some(_) = self.pending_responses.lock().await.remove(&id) {
                            debug!("Removed pending channel for failed transaction {}", id);
                        }
                    }
                },
            }
        }
        
        // Set the event processor as not running
        *self.event_processor_running.lock().await = false;
        info!("Event processor stopped");
        
        Ok(())
    }

    /// Handle an incoming SIP request
    async fn handle_request(&self, request: Request, source: SocketAddr) -> Result<()> {
        match request.method {
            Method::Invite => {
                info!("Received INVITE request, sending 200 OK");
                
                // Generate a tag for the To header
                let tag = format!("tag-{}", Uuid::new_v4());
                
                // Create a 200 OK response
                let mut response = Response::new(StatusCode::Ok);
                
                // Add Via headers from request
                for header in &request.headers {
                    if header.name == HeaderName::Via {
                        response.headers.push(header.clone());
                    }
                }
                
                // Add From header from request
                if let Some(from) = request.header(&HeaderName::From) {
                    response.headers.push(from.clone());
                }
                
                // Add To header with tag
                if let Some(to) = request.header(&HeaderName::To) {
                    if let Some(to_text) = to.value.as_text() {
                        response.headers.push(Header::text(
                            HeaderName::To,
                            format!("{};tag={}", to_text, tag)
                        ));
                    }
                }
                
                // Add Call-ID
                if let Some(call_id) = request.header(&HeaderName::CallId) {
                    response.headers.push(call_id.clone());
                }
                
                // Add CSeq
                if let Some(cseq) = request.header(&HeaderName::CSeq) {
                    response.headers.push(cseq.clone());
                }
                
                // Add Contact
                let contact = format!("<sip:{}@{}>", self.username, self.local_addr);
                response.headers.push(Header::text(HeaderName::Contact, contact));
                
                // Add User-Agent
                response.headers.push(Header::text(
                    HeaderName::UserAgent,
                    "RVOIP-Test-Client/0.1.0"
                ));
                
                // Add SDP content if the request had an SDP body
                if !request.body.is_empty() {
                    // Create a simple SDP answer
                    let sdp = format!(
                        "v=0\r\n\
                         o={} 123456 789012 IN IP4 {}\r\n\
                         s=Call\r\n\
                         c=IN IP4 {}\r\n\
                         t=0 0\r\n\
                         m=audio 10000 RTP/AVP 0\r\n\
                         a=rtpmap:0 PCMU/8000\r\n\
                         a=sendrecv\r\n",
                        self.username, self.local_addr.ip(), self.local_addr.ip()
                    );
                    
                    // Add Content-Type
                    response.headers.push(Header::text(HeaderName::ContentType, "application/sdp"));
                    
                    // Add Content-Length
                    response.headers.push(Header::text(
                        HeaderName::ContentLength, 
                        sdp.len().to_string()
                    ));
                    
                    // Add SDP body
                    response.body = sdp.into();
                } else {
                    // No SDP body
                    response.headers.push(Header::text(HeaderName::ContentLength, "0"));
                }
                
                // Send the response
                let message = Message::Response(response);
                self.transaction_manager.transport().send_message(message, source).await?;
                
                info!("200 OK response sent for INVITE");
            },
            Method::Ack => {
                info!("Received ACK request (no response needed)");
                // No response for ACK
            },
            Method::Bye => {
                info!("Received BYE request, sending 200 OK");
                
                // Create a 200 OK response
                let mut response = Response::new(StatusCode::Ok);
                
                // Add Via headers from request
                for header in &request.headers {
                    if header.name == HeaderName::Via {
                        response.headers.push(header.clone());
                    }
                }
                
                // Add From, To, Call-ID, CSeq headers from request
                if let Some(from) = request.header(&HeaderName::From) {
                    response.headers.push(from.clone());
                }
                
                if let Some(to) = request.header(&HeaderName::To) {
                    response.headers.push(to.clone());
                }
                
                if let Some(call_id) = request.header(&HeaderName::CallId) {
                    response.headers.push(call_id.clone());
                }
                
                if let Some(cseq) = request.header(&HeaderName::CSeq) {
                    response.headers.push(cseq.clone());
                }
                
                // Add Content-Length
                response.headers.push(Header::text(HeaderName::ContentLength, "0"));
                
                // Send the response
                let message = Message::Response(response);
                self.transaction_manager.transport().send_message(message, source).await?;
                
                info!("200 OK response sent for BYE");
            },
            _ => {
                info!("Received {} request, sending 501 Not Implemented", request.method);
                
                // Create a 501 Not Implemented response
                let mut response = Response::new(StatusCode::NotImplemented);
                
                // Add Via headers from request
                for header in &request.headers {
                    if header.name == HeaderName::Via {
                        response.headers.push(header.clone());
                    }
                }
                
                // Add From, To, Call-ID, CSeq headers from request
                if let Some(from) = request.header(&HeaderName::From) {
                    response.headers.push(from.clone());
                }
                
                if let Some(to) = request.header(&HeaderName::To) {
                    response.headers.push(to.clone());
                }
                
                if let Some(call_id) = request.header(&HeaderName::CallId) {
                    response.headers.push(call_id.clone());
                }
                
                if let Some(cseq) = request.header(&HeaderName::CSeq) {
                    response.headers.push(cseq.clone());
                }
                
                // Add Content-Length
                response.headers.push(Header::text(HeaderName::ContentLength, "0"));
                
                // Send the response
                let message = Message::Response(response);
                self.transaction_manager.transport().send_message(message, source).await?;
                
                info!("501 Not Implemented response sent for {}", request.method);
            }
        }
        
        Ok(())
    }

    /// Test that the client can receive messages
    async fn test_connectivity(&self) -> Result<()> {
        // Try to send a message to ourselves to test that UDP is working
        let loopback_addr = format!("127.0.0.1:{}", self.local_addr.port()).parse::<SocketAddr>()?;
        
        info!("Sending test message to {}", loopback_addr);
        
        // Create a simple OPTIONS request
        let uri = format!("sip:test@{}", loopback_addr).parse()?;
        let mut request = Request::new(Method::Options, uri);
        
        // Add minimal headers
        request.headers.push(Header::text(HeaderName::Via, format!("SIP/2.0/UDP {};branch=z9hG4bK-test", self.local_addr)));
        request.headers.push(Header::text(HeaderName::From, "<sip:test@localhost>;tag=test"));
        request.headers.push(Header::text(HeaderName::To, "<sip:test@localhost>"));
        request.headers.push(Header::text(HeaderName::CallId, "test-connectivity"));
        request.headers.push(Header::text(HeaderName::CSeq, "1 OPTIONS"));
        request.headers.push(Header::text(HeaderName::MaxForwards, "70"));
        request.headers.push(Header::text(HeaderName::ContentLength, "0"));
        
        // Send the message
        let message = Message::Request(request);
        self.transaction_manager.transport().send_message(message, loopback_addr).await?;
        
        // Wait a moment to see if we receive it
        tokio::time::sleep(Duration::from_millis(500)).await;
        
        info!("Connectivity test completed");
        Ok(())
    }

    /// Send a request and wait for the response
    async fn send_request(&self, request: Request) -> Result<Response> {
        // Create a copy of the request details for matching the response
        let method = request.method.clone();
        let call_id = request.call_id().unwrap_or_default();
        
        info!("Sending {} request to {} with Call-ID: {}", method, self.server_addr, call_id);
        
        // Create a channel for this specific request
        let (response_tx, response_rx) = oneshot::channel::<Response>();
        
        // Create transaction based on the method
        let transaction_id = if method == Method::Invite {
            self.transaction_manager.create_client_invite_transaction(
                request.clone(),
                self.server_addr,
            ).await.context("Failed to create INVITE transaction")?
        } else {
            self.transaction_manager.create_client_non_invite_transaction(
                request.clone(),
                self.server_addr,
            ).await.context("Failed to create non-INVITE transaction")?
        };
        
        // Store the response channel
        {
            let mut pending = self.pending_responses.lock().await;
            debug!("Adding transaction {} to pending_responses", transaction_id);
            pending.insert(transaction_id.clone(), response_tx);
            
            // Debug print pending transactions
            let pending_keys = pending.keys().cloned().collect::<Vec<_>>();
            debug!("Current pending transactions: {:?}", pending_keys);
        }
        
        // Send the request
        if let Err(e) = self.transaction_manager.send_request(&transaction_id).await {
            error!("Failed to send request through transaction: {}", e);
            return Err(anyhow!("Failed to send request: {}", e));
        }
        
        info!("Request sent through transaction: {}", transaction_id);
        
        // Wait for the response with a timeout
        // Increase timeout from 10s to 30s to avoid racing with event processor
        let timeout = tokio::time::sleep(Duration::from_secs(30));
        
        tokio::select! {
            response = response_rx => {
                match response {
                    Ok(resp) => {
                        info!("Received response to {} request: {}", method, resp.status);
                        Ok(resp)
                    },
                    Err(e) => {
                        error!("Response channel error: {}", e);
                        Err(anyhow!("Response channel error: {}", e))
                    }
                }
            },
            _ = timeout => {
                warn!("Timeout waiting for response to {}", method);
                
                // Attempt to remove the pending sender on timeout
                {
                    let mut pending = self.pending_responses.lock().await;
                    if pending.remove(&transaction_id).is_some() {
                        debug!("Removed pending response sender for {} due to timeout", transaction_id);
                    }
                }
                
                // Create a timeout response
                let mut timeout_response = Response::new(StatusCode::RequestTimeout);
                timeout_response.headers.push(Header::text(HeaderName::CallId, call_id));
                timeout_response.headers.push(Header::text(HeaderName::CSeq, format!("{} {}", self.cseq, method)));
                
                Ok(timeout_response)
            }
        }
    }
}

/// Helper to parse authentication parameters from WWW-Authenticate header
fn parse_auth_param(header: &str, param: &str) -> Option<String> {
    // Find the parameter
    let param_prefix = format!("{}=\"", param);
    let start = header.find(&param_prefix)? + param_prefix.len();
    let end = header[start..].find("\"")?;
    
    Some(header[start..(start + end)].to_string())
}

/// Run the SIP test client
async fn run_client(args: Args) -> Result<()> {
    // Parse socket addresses
    let local_addr: SocketAddr = args.local_addr.parse()
        .context("Invalid local address format")?;
    
    let server_addr: SocketAddr = args.server_addr.parse()
        .context("Invalid server address format")?;
    
    // Get values we'll need multiple times
    let username = args.username.clone();
    
    // Create SIP client
    let mut client = SipClient::new(
        local_addr,
        server_addr,
        args.username,
        args.domain,
    ).await?;
    
    // Start event processing in background
    let event_processor_running = client.event_processor_running.clone();
    let client_for_events = client.clone();
    let event_task = tokio::spawn(async move {
        let mut client = client_for_events;
        if let Err(e) = client.process_events().await {
            error!("Event processing error: {}", e);
        }
        *event_processor_running.lock().await = false;
    });
    
    // Parse the target URI
    let target_uri = args.target_uri.as_deref().unwrap_or("sip:bob@rvoip.local");
    info!("Calling target: {}", target_uri);
    
    // Create INVITE request
    let mut request = client.create_request(Method::Invite, target_uri.parse()?);
    
    // Add SDP to the request
    let sdp = format!(
        "v=0\r\n\
         o={} 123456 789012 IN IP4 {}\r\n\
         s=Call\r\n\
         c=IN IP4 {}\r\n\
         t=0 0\r\n\
         m=audio 10000 RTP/AVP 0\r\n\
         a=rtpmap:0 PCMU/8000\r\n\
         a=sendrecv",
        username,
        local_addr.ip(),
        local_addr.ip()
    );
    
    // Convert the SDP body to Bytes
    request.body = Bytes::from(sdp);
    
    request.headers.push(Header::text(HeaderName::ContentType, "application/sdp"));
    request.headers.push(Header::integer(HeaderName::ContentLength, request.body.len() as i64));
    
    // Send the request and wait for the response
    match client.send_request(request).await {
        Ok(response) => {
            if response.status == StatusCode::Ok {
                info!("Call established: {}", response.status);
                
                // Extract the To header with tag from the response
                let to_header = response.headers.iter()
                    .find(|h| h.name == HeaderName::To)
                    .ok_or_else(|| anyhow!("Missing To header in 200 OK"))?;
                
                let to_text = to_header.value.as_text()
                    .ok_or_else(|| anyhow!("Invalid To header format"))?;
                
                // Send ACK to complete the three-way handshake
                info!("Sending ACK to acknowledge 200 OK");
                let mut ack_request = client.create_request(Method::Ack, target_uri.parse()?);
                
                // For ACK after 2xx to INVITE, use the same CSeq number but method ACK
                ack_request.headers.iter_mut()
                    .find(|h| h.name == HeaderName::CSeq)
                    .map(|h| h.value = HeaderValue::Text(format!("{} ACK", client.cseq - 1)));
                
                // Use the To header with tag from the 200 OK
                ack_request.headers.iter_mut()
                    .find(|h| h.name == HeaderName::To)
                    .map(|h| h.value = HeaderValue::Text(to_text.to_string()));
                
                // Add Content-Length: 0
                ack_request.headers.push(Header::text(HeaderName::ContentLength, "0"));
                
                // Send ACK directly (ACK is end-to-end for 2xx responses)
                info!("Sending ACK to {}", client.server_addr);
                let message = Message::Request(ack_request);
                client.transaction_manager.transport().send_message(message, client.server_addr).await
                    .context("Failed to send ACK")?;
                
                // Wait for Ctrl+C to terminate the call
                info!("Call established! Press Ctrl+C to end the call");
                ctrl_c().await.context("Failed to listen for Ctrl+C")?;
                
                info!("Terminating call by sending BYE");
                
                // Send BYE to terminate the call
                let mut bye_request = client.create_request(Method::Bye, target_uri.parse()?);
                
                // Use the To header with tag from the 200 OK
                bye_request.headers.iter_mut()
                    .find(|h| h.name == HeaderName::To)
                    .map(|h| h.value = HeaderValue::Text(to_text.to_string()));
                
                // Add Content-Length: 0
                bye_request.headers.push(Header::text(HeaderName::ContentLength, "0"));
                
                // Send BYE and wait for response
                let bye_response = client.send_request(bye_request).await?;
                if bye_response.status == StatusCode::Ok {
                    info!("Call ended successfully");
                } else {
                    warn!("Unexpected response to BYE: {}", bye_response.status);
                }
            } else {
                warn!("Call failed: {}", response);
            }
        },
        Err(e) => {
            error!("Failed to send INVITE: {}", e);
        }
    }
    
    // Stop the event processing task
    event_task.abort();
    
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    // Parse command line arguments
    let args = Args::parse();
    
    info!("Starting SIP test client");
    info!("Local address: {}", args.local_addr);
    info!("Server address: {}", args.server_addr);
    info!("Username: {}", args.username);
    info!("Domain: {}", args.domain);
    info!("Mode: {}", args.mode);
    
    // Run in the requested mode
    match args.mode.as_str() {
        "ua" => {
            // Run in user agent mode (passive, waits for requests)
            info!("Running in user agent mode");
            user_agent::run_user_agent(&args.local_addr, &args.username, &args.domain).await
        },
        _ => {
            // Run in active client mode
            run_client(args).await
        }
    }
} 