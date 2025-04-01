use std::net::SocketAddr;
use std::str::FromStr;

use rvoip_sip_core::{Header, HeaderName, Method, Request, StatusCode, Uri};
use rvoip_sip_transport::{Transport, TransportEvent, UdpTransport};

use anyhow::Result;
use tracing::{info, warn, error, debug};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging with a more verbose filter
    tracing_subscriber::fmt()
        .with_env_filter("rvoip=debug,rvoip_sip_core=debug,rvoip_sip_transport=debug")
        .init();
    
    info!("Starting rvoip simple softswitch example");
    
    // Bind to a UDP socket
    let addr = SocketAddr::from_str("0.0.0.0:5060")?;
    let (transport, mut events) = UdpTransport::bind(addr, None).await
        .map_err(|e| anyhow::anyhow!("Failed to bind UDP transport: {}", e))?;
    
    info!("SIP UDP transport bound to {}", transport.local_addr()?);
    
    // Create a test message
    let test_message = create_test_message()?;
    debug!("Created test OPTIONS message:\n{}", test_message);
    
    // Send a test message to ourselves
    let local_addr = transport.local_addr()?;
    if let Err(e) = transport.send_message(test_message.into(), local_addr).await {
        error!("Failed to send test message: {}", e);
    } else {
        info!("Sent test message to {}", local_addr);
    }
    
    // Process incoming SIP messages
    info!("Waiting for incoming messages...");
    info!("Press Ctrl+C to exit");
    while let Some(event) = events.recv().await {
        match event {
            TransportEvent::MessageReceived { message, source, destination } => {
                info!("Received message from {} to {}", source, destination);
                
                if let Some(req) = message.as_request() {
                    // Handle request
                    info!("Received {} request for {}", req.method, req.uri);
                    debug!("Request details:\n{}", req);
                    
                    // Send a simple 200 OK response for any request
                    if let Err(e) = handle_request(req, source, &transport).await {
                        error!("Error handling request: {}", e);
                    }
                } else if let Some(resp) = message.as_response() {
                    // Handle response
                    info!("Received {} response", resp.status);
                    debug!("Response details:\n{}", resp);
                }
            }
            TransportEvent::Error { error } => {
                warn!("Transport error: {}", error);
            }
            TransportEvent::Closed => {
                info!("Transport closed");
                break;
            }
        }
    }
    
    info!("Exiting");
    Ok(())
}

// Creates a test SIP OPTIONS request
fn create_test_message() -> Result<Request> {
    let uri = Uri::from_str("sip:localhost")?;
    
    let message = Request::new(Method::Options, uri)
        .with_header(Header::text(HeaderName::From, "<sip:test@example.com>"))
        .with_header(Header::text(HeaderName::To, "<sip:localhost>"))
        .with_header(Header::text(HeaderName::CallId, "test-call-id@example.com"))
        .with_header(Header::text(HeaderName::CSeq, "1 OPTIONS"))
        .with_header(Header::text(HeaderName::Via, "SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-test"))
        .with_header(Header::text(HeaderName::MaxForwards, "70"))
        .with_header(Header::text(HeaderName::Contact, "<sip:test@127.0.0.1:5060>"))
        .with_header(Header::integer(HeaderName::ContentLength, 0));
    
    Ok(message)
}

// Handle incoming SIP requests and send a response
async fn handle_request(request: &Request, source: SocketAddr, transport: &UdpTransport) -> Result<()> {
    // Create a 200 OK response for any request type
    let mut response = rvoip_sip_core::Response::new(StatusCode::Ok);
    
    // Copy headers from request
    for header in &request.headers {
        if matches!(header.name, HeaderName::Via | HeaderName::From | HeaderName::To | HeaderName::CallId | HeaderName::CSeq) {
            response = response.with_header(header.clone());
        }
    }
    
    // Add content-length header (empty body)
    response = response.with_header(Header::integer(HeaderName::ContentLength, 0));
    
    // Send the response
    transport.send_message(response.into(), source).await
        .map_err(|e| anyhow::anyhow!("Failed to send response: {}", e))?;
    
    info!("Sent 200 OK response to {}", source);
    Ok(())
} 