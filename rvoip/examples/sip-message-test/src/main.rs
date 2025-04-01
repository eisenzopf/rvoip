use anyhow::{Context, Result};
use bytes::Bytes;
use rvoip_sip_core::{
    Header, HeaderName, HeaderValue, Message, Method, Request, Response, StatusCode, Uri, Version,
};
use rvoip_sip_transport::{
    bind_udp, Transport, TransportEvent,
};
use rvoip_transaction_core::{
    new_transaction_manager, ClientTransactionHandle, ServerTransactionHandle, TransactionManagerExt,
    TransactionOptions,
};
use std::{collections::HashMap, net::SocketAddr, str::FromStr, sync::Arc, time::Duration};
use tokio::{sync::mpsc, time::sleep};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

async fn run_client(
    local_addr: SocketAddr,
    remote_addr: SocketAddr,
) -> Result<()> {
    // Create a UDP transport
    let (transport, mut transport_events) = bind_udp(local_addr)
        .await
        .context("Failed to bind UDP transport")?;
    
    // Create a transaction manager
    let tx_options = TransactionOptions {
        t1: Duration::from_millis(500), // SIP timer T1 (RTT estimate)
        t4: Duration::from_secs(5),     // SIP timer T4 (max message lifetime)
    };
    let transaction_manager = new_transaction_manager(tx_options);
    
    // Spawn a task to handle transport events
    let tm = transaction_manager.clone();
    let transport_clone = transport.clone();
    tokio::spawn(async move {
        while let Some(event) = transport_events.recv().await {
            match event {
                TransportEvent::MessageReceived { source, message, destination } => {
                    debug!("Received message from {}: {:?}", source, message);
                    
                    // Log more detailed message information
                    match &message {
                        Message::Request(req) => {
                            info!("📥 REQUEST: {} {}", req.method, req.uri);
                            log_headers(&req.headers);
                        },
                        Message::Response(resp) => {
                            info!("📥 RESPONSE: {} {}", resp.status.as_u16(), resp.reason_phrase());
                            log_headers(&resp.headers);
                        }
                    }
                }
                TransportEvent::Error { error } => {
                    error!("Transport error: {}", error);
                }
                TransportEvent::Closed => {
                    error!("Transport closed unexpectedly");
                    break;
                }
            }
        }
    });
    
    // Test all SIP message types
    info!("Starting SIP message type tests...");
    
    // Test INVITE transaction
    info!("🔄 Testing INVITE transaction...");
    let invite_request = create_request(Method::Invite, "sip:bob@example.com", remote_addr);
    info!("📤 SENDING: INVITE sip:bob@example.com");
    log_headers(&invite_request.headers);
    
    let invite_tx = transaction_manager
        .create_client_transaction(invite_request, transport.clone(), remote_addr)
        .await?;
    
    // Wait for the transaction to complete or timeout
    let invite_result = invite_tx.wait_for_final_response().await;
    info!("📊 INVITE transaction complete: status={}", invite_result.as_ref().map(|r| r.status.as_u16()).unwrap_or(0));
    
    if let Ok(response) = invite_result {
        if response.status.is_success() {
            // Send ACK
            let ack_request = create_ack_request(&response, "sip:bob@example.com", remote_addr);
            info!("📤 Sending ACK");
            log_headers(&ack_request.headers);
            transport.send_message(ack_request.into(), remote_addr).await?;
            
            // Wait a bit and then send BYE
            sleep(Duration::from_secs(1)).await;
            
            // Test BYE transaction
            info!("🔄 Testing BYE transaction...");
            let bye_request = create_request(Method::Bye, "sip:bob@example.com", remote_addr);
            info!("📤 SENDING: BYE sip:bob@example.com");
            log_headers(&bye_request.headers);
            let bye_tx = transaction_manager
                .create_client_transaction(bye_request, transport.clone(), remote_addr)
                .await?;
            
            let bye_result = bye_tx.wait_for_final_response().await;
            info!("📊 BYE transaction complete: status={}", bye_result.as_ref().map(|r| r.status.as_u16()).unwrap_or(0));
        }
    }
    
    // Test REGISTER transaction
    info!("🔄 Testing REGISTER transaction...");
    let register_request = create_request(Method::Register, "sip:registrar.example.com", remote_addr);
    info!("📤 SENDING: REGISTER sip:registrar.example.com");
    log_headers(&register_request.headers);
    let register_tx = transaction_manager
        .create_client_transaction(register_request, transport.clone(), remote_addr)
        .await?;
    
    let register_result = register_tx.wait_for_final_response().await;
    info!("📊 REGISTER transaction complete: status={}", register_result.as_ref().map(|r| r.status.as_u16()).unwrap_or(0));
    
    // Test OPTIONS transaction
    info!("🔄 Testing OPTIONS transaction...");
    let options_request = create_request(Method::Options, "sip:bob@example.com", remote_addr);
    info!("📤 SENDING: OPTIONS sip:bob@example.com");
    log_headers(&options_request.headers);
    let options_tx = transaction_manager
        .create_client_transaction(options_request, transport.clone(), remote_addr)
        .await?;
    
    let options_result = options_tx.wait_for_final_response().await;
    info!("📊 OPTIONS transaction complete: status={}", options_result.as_ref().map(|r| r.status.as_u16()).unwrap_or(0));
    
    // Test SUBSCRIBE transaction
    info!("🔄 Testing SUBSCRIBE transaction...");
    let subscribe_request = create_request(Method::Subscribe, "sip:bob@example.com", remote_addr);
    info!("📤 SENDING: SUBSCRIBE sip:bob@example.com");
    log_headers(&subscribe_request.headers);
    let subscribe_tx = transaction_manager
        .create_client_transaction(subscribe_request, transport.clone(), remote_addr)
        .await?;
    
    let subscribe_result = subscribe_tx.wait_for_final_response().await;
    info!("📊 SUBSCRIBE transaction complete: status={}", subscribe_result.as_ref().map(|r| r.status.as_u16()).unwrap_or(0));
    
    // Test MESSAGE transaction
    info!("🔄 Testing MESSAGE transaction...");
    let message_request = create_request(Method::Message, "sip:bob@example.com", remote_addr)
        .with_body(Bytes::from("Hello, this is a SIP MESSAGE test"));
    info!("📤 SENDING: MESSAGE sip:bob@example.com");
    log_headers(&message_request.headers);
    info!("   Body: Hello, this is a SIP MESSAGE test");
    let message_tx = transaction_manager
        .create_client_transaction(message_request, transport.clone(), remote_addr)
        .await?;
    
    let message_result = message_tx.wait_for_final_response().await;
    info!("📊 MESSAGE transaction complete: status={}", message_result.as_ref().map(|r| r.status.as_u16()).unwrap_or(0));
    
    // Test UPDATE transaction
    info!("🔄 Testing UPDATE transaction...");
    let update_request = create_request(Method::Update, "sip:bob@example.com", remote_addr);
    info!("📤 SENDING: UPDATE sip:bob@example.com");
    log_headers(&update_request.headers);
    let update_tx = transaction_manager
        .create_client_transaction(update_request, transport.clone(), remote_addr)
        .await?;
    
    let update_result = update_tx.wait_for_final_response().await;
    info!("📊 UPDATE transaction complete: status={}", update_result.as_ref().map(|r| r.status.as_u16()).unwrap_or(0));
    
    info!("✅ All SIP message type tests completed!");
    
    // Wait a bit before exiting
    sleep(Duration::from_secs(2)).await;
    Ok(())
}

async fn run_server(local_addr: SocketAddr) -> Result<()> {
    // Create a UDP transport
    let (transport, mut transport_events) = bind_udp(local_addr)
        .await
        .context("Failed to bind UDP transport")?;
    
    // Create a transaction manager
    let tx_options = TransactionOptions {
        t1: Duration::from_millis(500), // SIP timer T1 (RTT estimate)
        t4: Duration::from_secs(5),     // SIP timer T4 (max message lifetime)
    };
    let transaction_manager = new_transaction_manager(tx_options);
    
    // Maintain a map of call-ids to dialogs
    let mut active_calls: HashMap<String, (String, String)> = HashMap::new();
    
    // Process incoming messages
    info!("SIP server listening on {}", local_addr);
    while let Some(event) = transport_events.recv().await {
        match event {
            TransportEvent::MessageReceived { source, message, destination } => {
                info!("Received message from {}: {:?}", source, message);
                
                // Log more detailed message information
                match &message {
                    Message::Request(req) => {
                        info!("📥 REQUEST: {} {}", req.method, req.uri);
                        log_headers(&req.headers);
                    },
                    Message::Response(resp) => {
                        info!("📥 RESPONSE: {} {}", resp.status.as_u16(), resp.reason_phrase());
                        log_headers(&resp.headers);
                    }
                }
                
                // Process based on message type
                if let Some(request) = message.as_request() {
                    let method = request.method.clone();
                    
                    // Extract Call-ID, From and To headers
                    let call_id = request.call_id().unwrap_or("unknown").to_string();
                    let from = request.from().unwrap_or("unknown").to_string();
                    let to = request.to().unwrap_or("unknown").to_string();
                    
                    // Store dialog info for non-ACK requests that we'll respond to
                    if method != Method::Ack {
                        active_calls.insert(call_id.clone(), (from.clone(), to.clone()));
                    }
                    
                    match method {
                        Method::Invite => {
                            info!("🔄 Processing INVITE request");
                            
                            // Create a server transaction for the INVITE
                            let server_tx = transaction_manager
                                .create_server_transaction(request.clone(), transport.clone(), source)
                                .await?;
                            
                            // Send a 100 Trying immediately
                            let trying = create_response(
                                StatusCode::Trying,
                                &call_id,
                                &from,
                                &to,
                            );
                            info!("📤 Sending 100 Trying");
                            log_headers(&trying.headers);
                            server_tx.send_provisional_response(trying.clone()).await?;
                            
                            // Simulate some processing delay (ringing)
                            sleep(Duration::from_millis(500)).await;
                            
                            // Send a 180 Ringing
                            let ringing = create_response(
                                StatusCode::Ringing,
                                &call_id,
                                &from,
                                &to,
                            );
                            info!("📤 Sending 180 Ringing");
                            log_headers(&ringing.headers);
                            server_tx.send_provisional_response(ringing.clone()).await?;
                            
                            // Simulate call being answered
                            sleep(Duration::from_millis(1000)).await;
                            
                            // Send a 200 OK for the INVITE
                            let ok = create_response(
                                StatusCode::Ok,
                                &call_id,
                                &from, 
                                &to,
                            );
                            info!("📤 Sending 200 OK for INVITE");
                            log_headers(&ok.headers);
                            server_tx.send_final_response(ok.clone()).await?;
                            
                            info!("✅ INVITE processed successfully - call established");
                        }
                        Method::Ack => {
                            // ACK doesn't need a response, but log it
                            info!("Received ACK for dialog {}", call_id);
                        }
                        Method::Bye => {
                            info!("🔄 Processing BYE request for dialog {}", call_id);
                            
                            // Create a server transaction for the BYE
                            let server_tx = transaction_manager
                                .create_server_transaction(request.clone(), transport.clone(), source)
                                .await?;
                            
                            // Send a 200 OK for the BYE
                            let ok = create_response(
                                StatusCode::Ok,
                                &call_id,
                                &from,
                                &to,
                            );
                            info!("📤 Sending 200 OK for BYE");
                            log_headers(&ok.headers);
                            server_tx.send_final_response(ok.clone()).await?;
                            
                            // Remove call from active calls
                            active_calls.remove(&call_id);
                            info!("✅ Call {} ended", call_id);
                        }
                        // Handle all other methods with a 200 OK
                        _ => {
                            info!("🔄 Processing {} request", method);
                            
                            // Create a server transaction for the request
                            let server_tx = transaction_manager
                                .create_server_transaction(request.clone(), transport.clone(), source)
                                .await?;
                            
                            // Send a 200 OK for the request
                            let ok = create_response(
                                StatusCode::Ok,
                                &call_id,
                                &from,
                                &to,
                            );
                            info!("📤 Sending 200 OK for {}", method);
                            log_headers(&ok.headers);
                            server_tx.send_final_response(ok.clone()).await?;
                            
                            info!("✅ {} processed successfully", method);
                        }
                    }
                }
            }
            TransportEvent::Error { error } => {
                error!("Transport error: {}", error);
            }
            TransportEvent::Closed => {
                error!("Transport closed unexpectedly");
                break;
            }
        }
    }
    
    Ok(())
}

fn create_request(method: Method, target_uri: &str, _remote_addr: SocketAddr) -> Request {
    info!("🔨 Creating {} request to {}", method, target_uri);
    let uri = Uri::from_str(target_uri).expect("Invalid URI");
    let call_id = format!("{}@example.com", Uuid::new_v4());
    let from_tag = format!("from-{}", Uuid::new_v4().simple());
    
    Request::new(method.clone(), uri)
        .with_header(Header::text(
            HeaderName::Via,
            format!("SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-{}", Uuid::new_v4().simple()),
        ))
        .with_header(Header::text(
            HeaderName::From,
            format!("sip:alice@example.com;tag={}", from_tag),
        ))
        .with_header(Header::text(HeaderName::To, target_uri))
        .with_header(Header::text(HeaderName::CallId, call_id))
        .with_header(Header::text(
            HeaderName::CSeq,
            format!("1 {}", method),
        ))
        .with_header(Header::text(HeaderName::MaxForwards, "70"))
        .with_header(Header::text(
            HeaderName::Contact,
            "sip:alice@127.0.0.1:5060",
        ))
        .with_header(Header::integer(HeaderName::ContentLength, 0))
}

fn create_ack_request(response: &Response, target_uri: &str, _remote_addr: SocketAddr) -> Request {
    let uri = Uri::from_str(target_uri).expect("Invalid URI");
    
    // Extract the necessary headers from the response, or use default values
    let call_id = response
        .header(&HeaderName::CallId)
        .and_then(|h| h.value.as_text())
        .unwrap_or("dummy-call-id@example.com")
        .to_string();
    
    let from = response
        .header(&HeaderName::From)
        .and_then(|h| h.value.as_text())
        .unwrap_or("sip:alice@example.com")
        .to_string();
    
    let to = response
        .header(&HeaderName::To)
        .and_then(|h| h.value.as_text())
        .unwrap_or("sip:bob@example.com")
        .to_string();
    
    // Use a default CSeq value
    let seq_num = "1";
    
    Request::new(Method::Ack, uri)
        .with_header(Header::text(
            HeaderName::Via,
            format!("SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-{}", Uuid::new_v4().simple()),
        ))
        .with_header(Header::text(HeaderName::From, from))
        .with_header(Header::text(HeaderName::To, to))
        .with_header(Header::text(HeaderName::CallId, call_id))
        .with_header(Header::text(
            HeaderName::CSeq,
            format!("{} ACK", seq_num),
        ))
        .with_header(Header::text(HeaderName::MaxForwards, "70"))
        .with_header(Header::integer(HeaderName::ContentLength, 0))
}

fn create_response(
    status: StatusCode,
    call_id: &str,
    from: &str,
    to: &str,
) -> Response {
    Response::new(status)
        .with_header(Header::text(
            HeaderName::Via,
            "SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-123456;received=127.0.0.1",
        ))
        .with_header(Header::text(HeaderName::From, from))
        .with_header(Header::text(HeaderName::To, to))
        .with_header(Header::text(HeaderName::CallId, call_id))
        .with_header(Header::text(HeaderName::CSeq, "1 INVITE"))
        .with_header(Header::text(
            HeaderName::Contact,
            "sip:bob@127.0.0.1:5070",
        ))
        .with_header(Header::integer(HeaderName::ContentLength, 0))
}

// Add a helper function to log headers
fn log_headers(headers: &[Header]) {
    for header in headers {
        match header.name {
            HeaderName::Via | HeaderName::From | HeaderName::To | 
            HeaderName::CallId | HeaderName::CSeq => {
                debug!("  📋 {}: {}", header.name, header.value);
            },
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing with more detailed output
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();
    
    // Parse command line arguments
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <server|client>", args[0]);
        std::process::exit(1);
    }
    
    match args[1].as_str() {
        "server" => {
            let addr = "127.0.0.1:5070".parse()?;
            run_server(addr).await
        }
        "client" => {
            let local_addr = "127.0.0.1:5060".parse()?;
            let remote_addr = "127.0.0.1:5070".parse()?;
            run_client(local_addr, remote_addr).await
        }
        _ => {
            eprintln!("Unknown command: {}. Use 'server' or 'client'.", args[1]);
            std::process::exit(1);
        }
    }
} 