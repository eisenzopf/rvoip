use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, Context, anyhow};
use tokio::signal::ctrl_c;
use tracing::{info, debug, error, warn};
use uuid::Uuid;

// Rust converts "-" to "_" when importing crates
use rvoip_sip_core as _;
use rvoip_sip_transport as _;
use rvoip_transaction_core as _;
use rvoip_session_core as _;

// Now import the specific types
use rvoip_sip_core::{
    Request, Response, Message, Method, StatusCode, 
    Header, HeaderName
};
use rvoip_sip_transport::UdpTransport;
use rvoip_transaction_core::{TransactionManager, TransactionEvent};

/// Simple user agent that can respond to SIP requests
pub struct UserAgent {
    /// Local address
    local_addr: SocketAddr,
    
    /// Username
    username: String,
    
    /// Domain
    domain: String,
    
    /// Transaction manager
    transaction_manager: Arc<TransactionManager>,
    
    /// Event receiver
    events_rx: tokio::sync::mpsc::Receiver<TransactionEvent>,
}

impl UserAgent {
    /// Create a new user agent
    pub async fn new(
        local_addr: SocketAddr,
        username: String,
        domain: String,
    ) -> Result<Self> {
        // Create UDP transport
        let (udp_transport, transport_rx) = UdpTransport::bind(local_addr, None).await
            .context("Failed to bind UDP transport")?;
        
        info!("User agent UDP transport bound to {}", local_addr);
        
        // Wrap transport in Arc
        let arc_transport = Arc::new(udp_transport);
        
        // Create transaction manager
        let (transaction_manager, events_rx) = TransactionManager::new(
            arc_transport,
            transport_rx,
            None, // Use default event capacity
        ).await.context("Failed to create transaction manager")?;
        
        info!("User agent {} initialized", username);
        
        Ok(Self {
            local_addr,
            username,
            domain,
            transaction_manager: Arc::new(transaction_manager),
            events_rx,
        })
    }
    
    /// Generate a new branch parameter
    fn new_branch(&self) -> String {
        format!("z9hG4bK-{}", Uuid::new_v4())
    }
    
    /// Process incoming requests and respond appropriately
    pub async fn process_requests(&mut self) -> Result<()> {
        info!("User agent {} started, waiting for requests on {}...", self.username, self.local_addr);
        
        // Process events from our channel
        while let Some(event) = self.events_rx.recv().await {
            match event {
                TransactionEvent::UnmatchedMessage { message, source } => {
                    match message {
                        Message::Request(request) => {
                            info!("Received {} request from {}", request.method, source);
                            debug!("Request details: {:?}", request);
                            
                            // Print out important headers for debugging
                            if let Some(h) = request.header(&HeaderName::From) {
                                info!("From: {}", h.value.as_text().unwrap_or("(invalid)"));
                            }
                            if let Some(h) = request.header(&HeaderName::To) {
                                info!("To: {}", h.value.as_text().unwrap_or("(invalid)"));
                            }
                            if let Some(h) = request.header(&HeaderName::CallId) {
                                info!("Call-ID: {}", h.value.as_text().unwrap_or("(invalid)"));
                            }
                            
                            let response = match request.method {
                                Method::Register => {
                                    info!("Received REGISTER request (ignoring, this is a user agent)");
                                    None
                                },
                                Method::Invite => {
                                    info!("Received INVITE request, sending 200 OK");
                                    self.handle_invite(request, source).await?
                                },
                                Method::Ack => {
                                    info!("Received ACK request");
                                    None // No response for ACK
                                },
                                Method::Bye => {
                                    info!("Received BYE request, sending 200 OK");
                                    self.handle_bye(request, source).await?
                                },
                                Method::Cancel => {
                                    info!("Received CANCEL request, sending 200 OK");
                                    let mut response = Response::new(StatusCode::Ok);
                                    self.add_common_headers(&mut response, &request);
                                    Some(response)
                                },
                                Method::Options => {
                                    info!("Received OPTIONS request, sending 200 OK");
                                    let mut response = Response::new(StatusCode::Ok);
                                    self.add_common_headers(&mut response, &request);
                                    Some(response)
                                },
                                _ => {
                                    warn!("Received unsupported request: {}", request.method);
                                    let mut response = Response::new(StatusCode::NotImplemented);
                                    self.add_common_headers(&mut response, &request);
                                    Some(response)
                                }
                            };
                            
                            // Send response if one was created
                            if let Some(response) = response {
                                info!("Sending {} response", response.status);
                                let message = Message::Response(response);
                                if let Err(e) = self.transaction_manager.transport().send_message(message, source).await {
                                    error!("Failed to send response: {}", e);
                                } else {
                                    info!("Response sent successfully");
                                }
                            }
                        },
                        Message::Response(response) => {
                            info!("Received unmatched response: {:?} from {}", response.status, source);
                        }
                    }
                },
                TransactionEvent::TransactionCreated { transaction_id } => {
                    debug!("Transaction created: {}", transaction_id);
                },
                TransactionEvent::TransactionCompleted { transaction_id, response } => {
                    if let Some(resp) = response {
                        info!("Transaction completed: {}, response: {:?}", transaction_id, resp.status);
                    } else {
                        info!("Transaction completed: {}, no response", transaction_id);
                    }
                },
                TransactionEvent::TransactionTerminated { transaction_id } => {
                    debug!("Transaction terminated: {}", transaction_id);
                },
                TransactionEvent::Error { error, transaction_id } => {
                    error!("Transaction error: {}, id: {:?}", error, transaction_id);
                },
            }
        }
        
        Ok(())
    }
    
    /// Handle an INVITE request
    async fn handle_invite(&self, request: Request, _source: SocketAddr) -> Result<Option<Response>> {
        // Extract Call-ID for tracking
        let call_id = request.call_id().unwrap_or("unknown");
        
        // Generate a tag for To header
        let tag = format!("tag-{}", Uuid::new_v4());
        
        info!("Processing INVITE for call {}", call_id);
        
        // Create 200 OK response
        let mut response = Response::new(StatusCode::Ok);
        
        // Add common headers
        self.add_common_headers(&mut response, &request);
        
        // Add To header with tag
        if let Some(to_header) = request.header(&HeaderName::To) {
            if let Some(to_text) = to_header.value.as_text() {
                info!("Adding To header with tag: {}", tag);
                response.headers.push(Header::text(
                    HeaderName::To,
                    format!("{};tag={}", to_text, tag)
                ));
            }
        }
        
        // Add Contact header
        let contact = format!("<sip:{}@{}>", self.username, self.local_addr);
        response.headers.push(Header::text(HeaderName::Contact, contact));
        
        // Add SDP content
        let sdp = format!(
            "v=0\r\n\
             o={} 654321 210987 IN IP4 {}\r\n\
             s=Call\r\n\
             c=IN IP4 {}\r\n\
             t=0 0\r\n\
             m=audio 10001 RTP/AVP 0\r\n\
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
        
        info!("Created OK response for INVITE with {} headers", response.headers.len());
        
        Ok(Some(response))
    }
    
    /// Handle a BYE request
    async fn handle_bye(&self, request: Request, _source: SocketAddr) -> Result<Option<Response>> {
        // Extract Call-ID for tracking
        let call_id = request.call_id().unwrap_or("unknown");
        
        info!("Processing BYE for call {}", call_id);
        
        // Create 200 OK response
        let mut response = Response::new(StatusCode::Ok);
        
        // Add common headers
        self.add_common_headers(&mut response, &request);
        
        // Add Content-Length
        response.headers.push(Header::text(HeaderName::ContentLength, "0"));
        
        Ok(Some(response))
    }
    
    /// Add common headers to a response based on the request
    fn add_common_headers(&self, response: &mut Response, request: &Request) {
        info!("Adding common headers to response");
        
        // Copy Via headers from request
        for header in &request.headers {
            if header.name == HeaderName::Via {
                response.headers.push(header.clone());
                debug!("Added Via header: {:?}", header.value);
            }
        }
        
        // Copy From header
        if let Some(from_header) = request.header(&HeaderName::From) {
            response.headers.push(from_header.clone());
            debug!("Added From header: {:?}", from_header.value);
        }
        
        // Copy To header if not already added
        if response.headers.iter().find(|h| h.name == HeaderName::To).is_none() {
            if let Some(to_header) = request.header(&HeaderName::To) {
                response.headers.push(to_header.clone());
                debug!("Added To header: {:?}", to_header.value);
            }
        }
        
        // Copy Call-ID
        if let Some(call_id_header) = request.header(&HeaderName::CallId) {
            response.headers.push(call_id_header.clone());
            debug!("Added Call-ID header: {:?}", call_id_header.value);
        }
        
        // Copy CSeq
        if let Some(cseq_header) = request.header(&HeaderName::CSeq) {
            response.headers.push(cseq_header.clone());
            debug!("Added CSeq header: {:?}", cseq_header.value);
        }
        
        // Add User-Agent
        response.headers.push(Header::text(
            HeaderName::UserAgent,
            "RVOIP-Test-UA/0.1.0"
        ));
        
        info!("Added {} common headers to response", response.headers.len());
    }
}

/// Run the user agent and wait for requests
pub async fn run_user_agent(addr: &str, username: &str, domain: &str) -> Result<()> {
    // Parse socket address
    let local_addr: SocketAddr = addr.parse()
        .context("Invalid address format")?;
    
    // Create user agent
    let mut user_agent = UserAgent::new(
        local_addr,
        username.to_string(),
        domain.to_string(),
    ).await?;
    
    // Process requests in background
    let handle = tokio::spawn(async move {
        if let Err(e) = user_agent.process_requests().await {
            error!("Error processing requests: {}", e);
        }
    });
    
    // Wait for Ctrl+C to shutdown
    ctrl_c().await.context("Failed to listen for ctrl+c signal")?;
    info!("Shutting down user agent...");
    
    // Cancel the handle
    handle.abort();
    
    Ok(())
} 