//! API Usage Examples
//!
//! This module provides practical examples of using the session-core API.

use crate::api::*;
use crate::api::handlers::CallHandler;
use crate::api::types::{IncomingCall, CallSession, SessionId, CallDecision};
use crate::api::control::SessionControl;
use crate::create::{generate_sdp_offer, generate_sdp_answer};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Example: Basic outgoing call
/// 
/// Shows how to make a simple outgoing call and handle the result.
#[allow(dead_code)]
pub async fn example_basic_call() -> Result<()> {
    // Create session manager
    let session_mgr = SessionManagerBuilder::new()
        .with_sip_port(5060)
        .with_local_address("sip:alice@example.com")
        .build()
        .await?;
    
    // Make a call
    let call = session_mgr.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        None
    ).await?;
    
    println!("Call initiated: {}", call.id);
    
    // Wait for some time...
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    
    // End the call
    session_mgr.terminate_session(&call.id).await?;
    
    Ok(())
}

/// Example: Call with custom handler
/// 
/// Shows how to implement a custom call handler for incoming calls.
#[allow(dead_code)]
pub async fn example_with_handler() -> Result<()> {
    #[derive(Debug)]
    struct MyHandler;
    
    #[async_trait::async_trait]
    impl CallHandler for MyHandler {
        async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
            println!("Incoming call from: {}", call.from);
            
            // Auto-accept all calls
            CallDecision::Accept(None)
        }
        
        async fn on_call_ended(&self, call: CallSession, reason: &str) {
            println!("Call {} ended: {}", call.id, reason);
        }
    }
    
    // Create session manager with handler
    let session_mgr = SessionManagerBuilder::new()
        .with_sip_port(5060)
        .with_local_address("sip:alice@example.com")
        .with_handler(Arc::new(MyHandler))
        .build()
        .await?;
    
    // Wait for incoming calls...
    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    
    Ok(())
}

/// Example: Conference call
/// 
/// Shows how to create a conference with multiple participants.
#[allow(dead_code)]
pub async fn example_conference() -> Result<()> {
    // Create session manager
    let session_mgr = SessionManagerBuilder::new()
        .with_sip_port(5060)
        .with_local_address("sip:conference@example.com")
        .build()
        .await?;
    
    // Create calls to participants
    let participants = vec![
        "sip:alice@example.com",
        "sip:bob@example.com",
        "sip:charlie@example.com",
    ];
    
    let mut calls = Vec::new();
    for participant in participants {
        match session_mgr.create_outgoing_call(
            "sip:conference@example.com",
            participant,
            None
        ).await {
            Ok(call) => {
                println!("Added participant: {}", participant);
                calls.push(call);
            }
            Err(e) => {
                tracing::error!("Failed to add participant {}: {}", participant, e);
            }
        }
    }
    
    // Conference is active...
    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    
    // End all calls
    for call in calls {
        let _ = session_mgr.terminate_session(&call.id).await;
    }
    
    Ok(())
}

/// Example: Call with media control
/// 
/// Shows how to control media during a call.
#[allow(dead_code)]
pub async fn example_media_control() -> Result<()> {
    // Create session manager
    let session_mgr = SessionManagerBuilder::new()
        .with_sip_port(5060)
        .with_local_address("sip:alice@example.com")
        .build()
        .await?;
    
    // Make a call
    let call = session_mgr.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        None
    ).await?;
    
    // Wait for call to be established
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    
    // Mute audio
    session_mgr.set_audio_muted(&call.id, true).await?;
    println!("Audio muted");
    
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    
    // Unmute audio
    session_mgr.set_audio_muted(&call.id, false).await?;
    println!("Audio unmuted");
    
    // Put call on hold
    session_mgr.hold_session(&call.id).await?;
    println!("Call on hold");
    
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    
    // Resume call
    session_mgr.resume_session(&call.id).await?;
    println!("Call resumed");
    
    // End the call
    session_mgr.terminate_session(&call.id).await?;
    
    Ok(())
}

/// Example: SDP negotiation
/// 
/// Shows how to handle custom SDP offers and answers.
#[allow(dead_code)]
pub async fn example_sdp_negotiation() -> Result<()> {
    #[derive(Debug)]
    struct SdpHandler;
    
    #[async_trait::async_trait]
    impl CallHandler for SdpHandler {
        async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
            if let Some(offer_sdp) = call.sdp {
                // Generate answer based on offer
                match generate_sdp_answer(&offer_sdp, "192.168.1.100", 20000) {
                    Ok(answer) => CallDecision::Accept(Some(answer)),
                    Err(_) => CallDecision::Reject("Incompatible media".to_string()),
                }
            } else {
                CallDecision::Accept(None)
            }
        }
        
        async fn on_call_ended(&self, _call: CallSession, _reason: &str) {}
    }
    
    // Create session manager with SDP handler
    let session_mgr = SessionManagerBuilder::new()
        .with_sip_port(5060)
        .with_local_address("sip:alice@example.com")
        .with_handler(Arc::new(SdpHandler))
        .build()
        .await?;
    
    // Make a call with custom SDP
    let sdp_offer = generate_sdp_offer("192.168.1.100", 20000)?;
    let call = session_mgr.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some(sdp_offer)
    ).await?;
    
    println!("Call with custom SDP: {}", call.id);
    
    Ok(())
}

/// Example: Simple auto-answer handler
#[derive(Debug)]
pub struct AutoAnswerHandler;

#[async_trait::async_trait]
impl CallHandler for AutoAnswerHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        println!("Auto-answering call from: {}", call.from);
        CallDecision::Accept(None)
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        println!("Call {} ended: {}", call.id, reason);
    }
}

/// Example: Queue handler for call centers
#[derive(Debug)]
pub struct QueueHandler {
    max_queue_size: usize,
    queue: tokio::sync::Mutex<Vec<IncomingCall>>,
    notify: tokio::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<IncomingCall>>>,
}

impl QueueHandler {
    pub fn new(max_queue_size: usize) -> Self {
        Self {
            max_queue_size,
            queue: tokio::sync::Mutex::new(Vec::new()),
            notify: tokio::sync::Mutex::new(None),
        }
    }
    
    pub async fn enqueue(&self, call: IncomingCall) {
        let mut queue = self.queue.lock().await;
        if queue.len() < self.max_queue_size {
            queue.push(call.clone());
            
            // Notify if channel is set
            if let Some(tx) = self.notify.lock().await.as_ref() {
                let _ = tx.send(call);
            }
        }
    }
    
    pub fn queue_size(&self) -> usize {
        // This is approximate since we're not locking
        0
    }
    
    pub fn set_notify_channel(&self, tx: tokio::sync::mpsc::UnboundedSender<IncomingCall>) {
        let mut notify = self.notify.blocking_lock();
        *notify = Some(tx);
    }
}

/// Example: Routing handler for gateways
#[derive(Debug)]
pub struct RoutingHandler {
    routes: std::collections::HashMap<String, String>,
    default_action: CallDecision,
}

impl RoutingHandler {
    pub fn new() -> Self {
        Self {
            routes: std::collections::HashMap::new(),
            default_action: CallDecision::Reject("No route".to_string()),
        }
    }
    
    pub fn add_route(&mut self, prefix: &str, destination: &str) {
        self.routes.insert(prefix.to_string(), destination.to_string());
    }
    
    pub fn set_default_action(&mut self, action: CallDecision) {
        self.default_action = action;
    }
}

#[async_trait::async_trait]
impl CallHandler for RoutingHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        // Extract number from To header
        let to_number = call.to.split('@').next().unwrap_or("")
            .split(':').last().unwrap_or("");
        
        // Find matching route
        for (prefix, _destination) in &self.routes {
            if to_number.starts_with(prefix) {
                // In a real implementation, you would forward the call to destination
                return CallDecision::Accept(None);
            }
        }
        
        self.default_action.clone()
    }
    
    async fn on_call_ended(&self, _call: CallSession, _reason: &str) {}
} 