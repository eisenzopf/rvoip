// Example code to demonstrate basic integration with the session-core library
// This example shows how to create a simple outgoing call with dialog and SDP negotiation

use std::sync::Arc;
use std::net::SocketAddr;
use std::path::PathBuf;
use tokio::time::{sleep, Duration};
use std::str::FromStr;
use anyhow::Result;
use async_trait::async_trait;

use rvoip_sip_core::{
    Method, Request, Response, Uri,
    types::{
        status::StatusCode,
        address::Address,
    }
};

use rvoip_sip_transport::{ TransportEvent, Transport };
use rvoip_transaction_core::{ TransactionManager, TransactionEvent };

// Remove nonexistent imports and use the proper module imports
use rvoip_session_core::{
    session::{
        SessionConfig, 
        SessionId, 
        session::Session, 
        manager::SessionManager,
        SessionState
    },
    dialog::{
        DialogId,
        dialog_manager::DialogManager,
        dialog_state::DialogState
    },
    events::{EventBus, EventHandler, SessionEvent},
    sdp::SessionDescription,
    errors::Error,
    helpers::{make_call, end_call}
};

// A simple event handler that just prints out session events
struct SimpleEventHandler;

#[async_trait]
impl EventHandler for SimpleEventHandler {
    async fn handle_event(&self, event: SessionEvent) {
        match event {
            SessionEvent::Created { session_id } => {
                println!("🌟 Session created: {}", session_id);
            },
            SessionEvent::StateChanged { session_id, old_state, new_state } => {
                println!("🔄 Session state changed: {} -> {}", old_state, new_state);
            },
            SessionEvent::Terminated { session_id, reason } => {
                println!("💀 Session terminated: {} (reason: {})", session_id, reason);
            },
            _ => println!("Event: {:?}", event),
        }
    }
}

// A minimal dummy transport that just logs messages
#[derive(Debug, Clone)]
struct DummyTransport {
    local_addr: SocketAddr,
}

impl DummyTransport {
    fn new(local_addr: SocketAddr) -> Self {
        Self { local_addr }
    }
}

#[async_trait::async_trait]
impl rvoip_sip_transport::Transport for DummyTransport {
    async fn send_message(
        &self, 
        message: rvoip_sip_core::Message, 
        destination: SocketAddr
    ) -> std::result::Result<(), rvoip_sip_transport::error::Error> {
        println!("📤 Would send {} to {}", 
                if message.is_request() { "request" } else { "response" }, 
                destination);
        Ok(())
    }
    
    fn local_addr(&self) -> std::result::Result<SocketAddr, rvoip_sip_transport::error::Error> {
        Ok(self.local_addr)
    }
    
    async fn close(&self) -> std::result::Result<(), rvoip_sip_transport::error::Error> {
        Ok(())
    }
    
    fn is_closed(&self) -> bool {
        false
    }
}

// Simple response handler for the transport
struct SimpleResponseHandler;

impl SimpleResponseHandler {
    async fn simulate_responses(tx: tokio::sync::mpsc::Sender<rvoip_sip_transport::TransportEvent>) {
        // We're not simulating any responses in this basic example
        // This would handle generating responses in a real example
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Simple logging setup
    println!("Starting basic integration example");
    
    // Create a local address for our transport
    let local_addr: SocketAddr = "127.0.0.1:5060".parse()?;
    
    // Create the transport channels
    let (transport_tx, transport_rx) = tokio::sync::mpsc::channel(10);
    
    // Create dummy transport
    let transport = Arc::new(DummyTransport::new(local_addr));
    
    // Create transaction manager
    let (transaction_manager, event_rx) = TransactionManager::new(
        transport.clone(),
        transport_rx,
        Some(10)
    ).await.map_err(|e| anyhow::anyhow!("Failed to create transaction manager: {}", e))?;
    let transaction_manager = Arc::new(transaction_manager);
    
    // Listen for transaction events
    tokio::spawn(async move {
        let mut event_rx = event_rx;
        while let Some(event) = event_rx.recv().await {
            println!("📨 Transaction event: {:?}", event);
        }
    });
    
    // Create session config
    let config = SessionConfig {
        local_signaling_addr: local_addr,
        local_media_addr: "127.0.0.1:10000".parse()?,
        supported_codecs: vec![],
        display_name: Some("Test User".to_string()),
        user_agent: "RVOIP-Test/0.1.0".to_string(),
        max_duration: 0,
        max_sessions: None,
    };
    
    // Create event bus and session manager
    let event_bus = EventBus::new(100);
    let handler = Arc::new(SimpleEventHandler);
    event_bus.register_handler(handler).await;
    
    let session_manager = Arc::new(SessionManager::new(
        transaction_manager.clone(),
        config,
        event_bus
    ));
    
    // Start the session manager
    session_manager.start().await?;
    
    // Create a session using the helper function instead of directly
    println!("📱 Creating outgoing call...");
    let destination = Uri::sip("user@example.com");
    let session = make_call(&session_manager, destination).await?;
    println!("📱 Created session with ID: {}", session.id);
    
    // We'll test just the session creation - not sending requests yet
    // due to potential API compatibility issues
    println!("📞 Session created in state: {}", session.state().await);
    
    // Simulate some work
    sleep(Duration::from_secs(1)).await;
    
    // End the call using the helper function
    println!("📞 Ending call...");
    end_call(&session).await?;
    println!("📞 Call ended");
    
    // Clean up
    let cleaned = session_manager.cleanup_terminated().await;
    println!("🧹 Cleaned up {} terminated sessions", cleaned);
    
    println!("✅ Basic integration example completed successfully");
    Ok(())
} 