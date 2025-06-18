//! SIP REGISTER Client Demo
//!
//! This client demonstrates sending a REGISTER request to the CallCenterEngine server.
//! It uses sip-core to build the request and sip-transport to send it over UDP.

use anyhow::Result;
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::types::{Method, Message, TypedHeader};
use rvoip_sip_core::types::expires::Expires;
use rvoip_sip_transport::transport::{UdpTransport, Transport, TransportEvent};
use std::time::Duration;
use tokio::time::sleep;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    println!("🚀 SIP REGISTER Client Demo\n");

    // Configuration
    let server_addr = "127.0.0.1:5060"; // Default SIP port where server listens
    let client_addr = "127.0.0.1:0";    // Let OS assign a port
    let from_uri = "sip:agent001@callcenter.example.com";
    let contact_uri = "sip:agent001@192.168.1.100:5062"; // Where agent can be reached
    
    // Create UDP transport for the client
    println!("📡 Creating UDP transport...");
    let (transport, mut events_rx) = UdpTransport::bind(client_addr.parse()?).await?;
    let local_addr = transport.local_addr()?;
    println!("✅ Client listening on: {}\n", local_addr);
    
    // Build REGISTER request
    println!("📝 Building REGISTER request...");
    let register_request = SimpleRequestBuilder::register(&format!("sip:{}", server_addr))?
        .from("Agent 001", from_uri, Some("agent-tag-12345"))
        .to("Agent 001", from_uri, None) // No tag for To header in REGISTER
        .call_id(&format!("reg-{}-{}", local_addr.port(), std::process::id()))
        .cseq(1)
        .via(&local_addr.to_string(), "UDP", Some(&format!("z9hG4bK{}", uuid::Uuid::new_v4())))
        .contact(contact_uri, None)
        .header(TypedHeader::Expires(Expires::new(3600))) // 1 hour registration
        .max_forwards(70)
        .user_agent("RVoIP-Agent/1.0")
        .build();
    
    println!("✅ REGISTER request built:\n");
    println!("  From: {}", from_uri);
    println!("  Contact: {}", contact_uri);
    println!("  Expires: 3600 seconds");
    println!("  Server: {}\n", server_addr);
    
    // Send REGISTER request
    println!("📤 Sending REGISTER to {}...", server_addr);
    transport.send_message(Message::Request(register_request), server_addr.parse()?).await?;
    println!("✅ REGISTER request sent!\n");
    
    // Start a task to handle transport events
    let transport_clone = transport.clone();
    let handle = tokio::spawn(async move {
        println!("👂 Listening for responses...\n");
        
        while let Some(event) = events_rx.recv().await {
            match event {
                TransportEvent::MessageReceived { message, source, .. } => {
                    match message {
                        Message::Response(response) => {
                            println!("📥 Received response from {}:", source);
                            println!("   Status: {} {}", response.status_code(), response.reason_phrase().unwrap_or(""));
                            
                            // Check if it's a successful registration
                            if response.status_code().as_u16() == 200 {
                                println!("\n✅ Registration successful!");
                                
                                // Check for Expires header in response
                                if let Some(expires) = response.typed_header::<Expires>() {
                                    println!("   Registration expires in: {} seconds", expires.0);
                                }
                            } else if response.status_code().as_u16() == 404 {
                                println!("\n❌ Registration failed: Agent not found");
                            } else {
                                println!("\n⚠️  Registration returned status: {}", response.status_code());
                            }
                            
                            // Close transport after receiving response
                            let _ = transport_clone.close().await;
                            break;
                        }
                        Message::Request(_) => {
                            println!("📥 Unexpected request received from {}", source);
                        }
                    }
                }
                TransportEvent::Error { error } => {
                    println!("❌ Transport error: {}", error);
                }
                TransportEvent::Closed => {
                    println!("🔌 Transport closed");
                    break;
                }
            }
        }
    });
    
    // Wait for response or timeout
    println!("⏳ Waiting for response (timeout: 5 seconds)...\n");
    tokio::select! {
        _ = handle => {
            println!("\n📋 Registration flow completed!");
        }
        _ = sleep(Duration::from_secs(5)) => {
            println!("\n⏰ Timeout waiting for response");
            transport.close().await?;
        }
    }
    
    // Demo: De-registration (expires=0)
    println!("\n📝 Demonstrating de-registration...");
    
    // Create new transport for de-registration
    let (transport2, mut events_rx2) = UdpTransport::bind("127.0.0.1:0".parse()?).await?;
    let local_addr2 = transport2.local_addr()?;
    
    // Build de-register request (expires=0)
    let deregister_request = SimpleRequestBuilder::register(&format!("sip:{}", server_addr))?
        .from("Agent 001", from_uri, Some("agent-tag-12345"))
        .to("Agent 001", from_uri, None)
        .call_id(&format!("dereg-{}-{}", local_addr2.port(), std::process::id()))
        .cseq(1)
        .via(&local_addr2.to_string(), "UDP", Some(&format!("z9hG4bK{}", uuid::Uuid::new_v4())))
        .contact(contact_uri, None)
        .header(TypedHeader::Expires(Expires::new(0))) // De-register
        .max_forwards(70)
        .user_agent("RVoIP-Agent/1.0")
        .build();
    
    println!("📤 Sending de-registration (expires=0)...");
    transport2.send_message(Message::Request(deregister_request), server_addr.parse()?).await?;
    
    // Wait briefly for response
    let transport2_clone = transport2.clone();
    tokio::spawn(async move {
        while let Some(event) = events_rx2.recv().await {
            if let TransportEvent::MessageReceived { message: Message::Response(response), .. } = event {
                if response.status_code().as_u16() == 200 {
                    println!("✅ De-registration successful!");
                } else {
                    println!("⚠️  De-registration returned status: {}", response.status_code());
                }
                let _ = transport2_clone.close().await;
                break;
            }
        }
    });
    
    sleep(Duration::from_secs(2)).await;
    transport2.close().await?;
    
    println!("\n✅ Demo completed!");
    
    Ok(())
} 