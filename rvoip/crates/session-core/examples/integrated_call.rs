use std::sync::Arc;
use std::time::Duration;
use std::net::SocketAddr;
use tokio::sync::mpsc;
use tokio::time::sleep;
use anyhow::{Result, Context};

// Import the correct types from our libraries
use rvoip_sip_core::{
    Uri, Message, Method, StatusCode, 
    Request, Response, HeaderName, TypedHeader
};
use rvoip_sip_transport::{Transport, TransportEvent};
use rvoip_transaction_core::{TransactionManager, TransactionEvent, TransactionKey};

// Import from session-core with correct paths
use rvoip_session_core::{
    events::{EventBus, EventHandler, SessionEvent},
    session::{SessionConfig, manager::SessionManager},
    media::AudioCodecType,
    // Import helper functions
    make_call, answer_call, end_call, 
    create_dialog_from_invite, send_dialog_request
};

/// Simple SIP transport implementation for the example
#[derive(Debug, Clone)]
struct MockTransport {
    event_tx: mpsc::Sender<TransportEvent>,
    local_addr: SocketAddr,
}

impl MockTransport {
    fn new(event_tx: mpsc::Sender<TransportEvent>, local_addr: SocketAddr) -> Self {
        Self { event_tx, local_addr }
    }
}

#[async_trait::async_trait]
impl Transport for MockTransport {
    async fn send_message(&self, message: Message, destination: SocketAddr) 
        -> std::result::Result<(), rvoip_sip_transport::error::Error> {
        println!("Transport: Sending {} to {}", 
            if message.is_request() { "request" } else { "response" }, 
            destination);
        
        // In a real example, we would simulate responses here
        // For now, we're skipping the response simulation to focus on compiling
        
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

/// Event handler implementation for session events
struct CallEventHandler;

#[async_trait::async_trait]
impl EventHandler for CallEventHandler {
    async fn handle_event(&self, event: SessionEvent) {
        match event {
            SessionEvent::Created { session_id } => {
                println!("🌟 Session created: {}", session_id);
            },
            SessionEvent::StateChanged { session_id, old_state, new_state } => {
                println!("🔄 Session state changed: {} -> {}", old_state, new_state);
            },
            SessionEvent::DialogUpdated { session_id, dialog_id } => {
                println!("🔄 Dialog updated: {}", dialog_id);
            },
            SessionEvent::Terminated { session_id, reason } => {
                println!("💀 Session terminated: {} (reason: {})", session_id, reason);
            },
            _ => println!("Other event: {:?}", event),
        }
    }
}

/// Transaction event listener to demonstrate transaction events
struct TransactionListener;

impl TransactionListener {
    async fn start_listening(mut events_rx: mpsc::Receiver<TransactionEvent>) {
        tokio::spawn(async move {
            println!("🎧 Started listening for transaction events");
            
            while let Some(event) = events_rx.recv().await {
                match event {
                    TransactionEvent::StateChanged { transaction_id, previous_state, new_state } => {
                        println!("🔁 Transaction state changed: {:?} -> {:?}", previous_state, new_state);
                    },
                    _ => println!("📣 Transaction event: {:?}", event),
                }
            }
        });
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Setup logging
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(tracing::Level::DEBUG)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .context("setting default subscriber failed")?;
    
    // Create transport channels
    let (transport_tx, transport_rx) = mpsc::channel(100);
    
    // Define our local address
    let local_addr: SocketAddr = "127.0.0.1:5060".parse()?;
    
    // Create mock transport
    let transport = Arc::new(MockTransport::new(transport_tx, local_addr));
    
    // Create transaction manager
    let (transaction_manager, events_rx) = TransactionManager::new(
        transport.clone(),
        transport_rx,
        Some(10)
    ).await.map_err(|e| anyhow::anyhow!("Failed to create transaction manager: {}", e))?;
    let transaction_manager = Arc::new(transaction_manager);
    
    // Start listening for transaction events
    TransactionListener::start_listening(events_rx).await;
    
    // Create session config
    let config = SessionConfig {
        local_signaling_addr: local_addr,
        local_media_addr: "127.0.0.1:10000".parse()?,
        supported_codecs: vec![AudioCodecType::PCMU, AudioCodecType::PCMA],
        display_name: Some("Alice".to_string()),
        user_agent: "rvoip-test/0.1.0".to_string(),
        max_duration: 0,
        max_sessions: None,
    };
    
    // Create event bus and register handler
    let event_bus = EventBus::new(100);
    let event_handler = Arc::new(CallEventHandler);
    event_bus.register_handler(event_handler).await;
    
    // Create session manager
    let session_manager = Arc::new(SessionManager::new(
        transaction_manager,
        config,
        event_bus,
    ));
    
    // Start session manager to process transaction events
    session_manager.start().await?;
    
    // Create an outgoing call using the helper function
    println!("\n📱 Creating outgoing call...");
    let destination = Uri::sip("bob@example.com");
    let session = make_call(&session_manager, destination).await?;
    println!("📱 Created outgoing session with ID: {}", session.id);
    println!("📞 Session state: {}", session.state().await);
    
    // In a real scenario, we'd receive an incoming call and could use the answer_call function
    // Simulating how to use the answer function (this wouldn't work in this example)
    println!("💡 To answer a call, you would use: answer_call(&incoming_session).await");
    
    // Wait for a bit
    println!("⏳ Waiting...");
    sleep(Duration::from_secs(2)).await;
    
    // End the call using the helper function
    println!("📞 Ending call...");
    end_call(&session).await?;
    println!("📞 Call ended");
    
    // Clean up
    println!("🧹 Cleaning up");
    let cleaned = session_manager.cleanup_terminated().await;
    println!("🧹 Cleaned up {} terminated sessions", cleaned);
    
    println!("\n✅ Example completed successfully");
    Ok(())
} 