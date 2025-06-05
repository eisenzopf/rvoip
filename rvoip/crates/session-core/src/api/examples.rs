//! Example Code for Common Use Cases
//!
//! This module contains complete examples showing how to use the session-core API
//! for different types of applications.

#![allow(unused_imports)]
#![allow(dead_code)]

use std::sync::Arc;
use tokio::sync::mpsc;
use crate::api::*;
use crate::manager::SessionManager;

/// Example: Simple SIP Server that auto-answers all calls
/// 
/// This shows the minimal code needed to create a SIP server that accepts
/// all incoming calls.
pub mod simple_sip_server {
    use super::*;

    pub async fn run() -> crate::errors::Result<()> {
        // Create a session manager with auto-answer handler
        let session_mgr = SessionManagerBuilder::new()
            .with_sip_port(5060)
            .with_handler(Arc::new(AutoAnswerHandler))
            .build()
            .await?;
        
        // Start the server
        session_mgr.start().await?;
        println!("SIP server running on port 5060");
        
        // Wait for shutdown signal
        tokio::signal::ctrl_c().await?;
        println!("Server shutting down...");
        
        Ok(())
    }
}

/// Example: WebSocket API Bridge
/// 
/// This shows how to create a WebSocket API that bridges to SIP sessions,
/// allowing web applications to make and receive calls.
pub mod websocket_bridge {
    use super::*;
    use serde_json::json;

    // Mock WebSocket types - replace with your WebSocket library
    pub struct WebSocket;
    pub struct WebSocketMessage { pub command: String, pub from: String, pub to: String, pub call_id: String }
    
    impl WebSocket {
        pub async fn recv(&mut self) -> Option<WebSocketMessage> { None }
        pub async fn send(&mut self, _value: serde_json::Value) -> crate::errors::Result<()> { Ok(()) }
    }

    pub async fn handle_websocket(mut ws: WebSocket, session_mgr: Arc<SessionManager>) -> crate::errors::Result<()> {
        while let Some(msg) = ws.recv().await {
            match msg.command.as_str() {
                "make_call" => {
                    let call = make_call_with_manager(&session_mgr, &msg.from, &msg.to).await?;
                    ws.send(json!({ 
                        "type": "call_created",
                        "call_id": call.id().as_str() 
                    })).await?;
                }
                "hangup" => {
                    if let Some(call) = find_session(&session_mgr, &SessionId(msg.call_id)).await? {
                        terminate_call(&call).await?;
                        ws.send(json!({ 
                            "type": "call_ended",
                            "call_id": call.id().as_str() 
                        })).await?;
                    }
                }
                "hold" => {
                    if let Some(call) = find_session(&session_mgr, &SessionId(msg.call_id)).await? {
                        hold_call(&call).await?;
                        ws.send(json!({ 
                            "type": "call_held",
                            "call_id": call.id().as_str() 
                        })).await?;
                    }
                }
                "resume" => {
                    if let Some(call) = find_session(&session_mgr, &SessionId(msg.call_id)).await? {
                        resume_call(&call).await?;
                        ws.send(json!({ 
                            "type": "call_resumed",
                            "call_id": call.id().as_str() 
                        })).await?;
                    }
                }
                _ => {
                    ws.send(json!({ 
                        "type": "error",
                        "message": "Unknown command" 
                    })).await?;
                }
            }
        }
        Ok(())
    }
}

/// Example: P2P Client
/// 
/// This shows how to create a peer-to-peer SIP client that can make direct
/// calls without requiring a server.
pub mod p2p_client {
    use super::*;

    pub async fn run() -> crate::errors::Result<()> {
        // Create session manager in P2P mode
        let session_mgr = SessionManagerBuilder::new()
            .p2p_mode()
            .build()
            .await?;
        
        // Make a direct call
        let call = make_call_with_manager(
            &session_mgr,
            "sip:alice@192.168.1.100",
            "sip:bob@192.168.1.200"
        ).await?;
        
        println!("Call initiated: {}", call.id());
        
        // Wait for the call to be answered
        call.wait_for_answer().await?;
        println!("Call connected!");
        
        // Keep the call active for 30 seconds
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
        
        // Terminate the call
        call.terminate().await?;
        println!("Call ended");
        
        Ok(())
    }
}

/// Example: Call Center Queue
/// 
/// This shows how to implement a call center with queuing, where incoming
/// calls are queued when all agents are busy.
pub mod call_center {
    use super::*;

    #[derive(Debug)]
    pub struct CallCenterHandler {
        queue: Arc<QueueHandler>,
        agents_available: Arc<tokio::sync::Mutex<bool>>,
    }

    impl CallCenterHandler {
        pub fn new(max_queue_size: usize) -> Self {
            Self {
                queue: Arc::new(QueueHandler::new(max_queue_size)),
                agents_available: Arc::new(tokio::sync::Mutex::new(true)),
            }
        }

        pub async fn process_queue(&self) -> crate::errors::Result<()> {
            let (tx, mut rx) = mpsc::unbounded_channel();
            self.queue.set_notify_channel(tx);

            while let Some(call) = rx.recv().await {
                // Wait for an agent to become available
                let mut available = self.agents_available.lock().await;
                if *available {
                    *available = false;
                    drop(available);

                    println!("Connecting call {} to agent", call.id);
                    
                    // Accept the call
                    let session = call.accept().await?;
                    
                    // Simulate call handling
                    tokio::spawn({
                        let agents = Arc::clone(&self.agents_available);
                        async move {
                            // Simulate call duration
                            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
                            
                            // Agent becomes available again
                            *agents.lock().await = true;
                            
                            // End the call
                            let _ = session.terminate().await;
                        }
                    });
                }
            }
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl CallHandler for CallCenterHandler {
        async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
            let available = *self.agents_available.lock().await;
            
            if available {
                // Agent is available, accept immediately
                CallDecision::Accept
            } else {
                // No agent available, add to queue
                if self.queue.queue_size() < 10 {
                    self.queue.enqueue(call).await;
                    CallDecision::Defer
                } else {
                    CallDecision::Reject("Queue full".to_string())
                }
            }
        }

        async fn on_call_ended(&self, call: CallSession, reason: &str) {
            println!("Call center call {} ended: {}", call.id(), reason);
        }
    }

    pub async fn run() -> crate::errors::Result<()> {
        let handler = Arc::new(CallCenterHandler::new(10));
        
        let session_mgr = SessionManagerBuilder::new()
            .with_sip_port(5060)
            .with_handler(handler.clone())
            .build()
            .await?;

        // Start processing the queue
        let queue_processor = tokio::spawn({
            let handler = Arc::clone(&handler);
            async move {
                handler.process_queue().await
            }
        });

        // Start the session manager
        session_mgr.start().await?;
        println!("Call center running on port 5060");

        // Wait for shutdown
        tokio::signal::ctrl_c().await?;
        
        // Cleanup
        queue_processor.abort();
        
        Ok(())
    }
}

/// Example: SIP Gateway
/// 
/// This shows how to create a SIP gateway that routes calls between
/// different SIP networks or protocols.
pub mod sip_gateway {
    use super::*;

    #[derive(Debug)]
    pub struct GatewayHandler {
        routing: Arc<RoutingHandler>,
    }

    impl GatewayHandler {
        pub fn new() -> Self {
            let mut routing = RoutingHandler::new();
            
            // Add routing rules
            routing.add_route("1", "sip:gateway1.example.com"); // Route 1xxx to gateway1
            routing.add_route("2", "sip:gateway2.example.com"); // Route 2xxx to gateway2
            routing.set_default_action(CallDecision::Reject("No route".to_string()));
            
            Self {
                routing: Arc::new(routing),
            }
        }
    }

    #[async_trait::async_trait]
    impl CallHandler for GatewayHandler {
        async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
            self.routing.on_incoming_call(call).await
        }

        async fn on_call_ended(&self, call: CallSession, reason: &str) {
            println!("Gateway call {} ended: {}", call.id(), reason);
        }
    }

    pub async fn run() -> crate::errors::Result<()> {
        let session_mgr = SessionManagerBuilder::new()
            .with_sip_port(5060)
            .with_handler(Arc::new(GatewayHandler::new()))
            .build()
            .await?;
        
        session_mgr.start().await?;
        println!("SIP gateway running on port 5060");
        
        tokio::signal::ctrl_c().await?;
        Ok(())
    }
}

/// Example: Conference Bridge
/// 
/// This shows how to create a conference bridge where multiple calls
/// can be connected together.
pub mod conference_bridge {
    use super::*;

    #[derive(Debug)]
    pub struct ConferenceHandler {
        bridge_sessions: Arc<tokio::sync::Mutex<Vec<SessionId>>>,
    }

    impl ConferenceHandler {
        pub fn new() -> Self {
            Self {
                bridge_sessions: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            }
        }

        async fn add_to_conference(&self, session_id: SessionId) {
            let mut sessions = self.bridge_sessions.lock().await;
            sessions.push(session_id);
            println!("Added session to conference. Total participants: {}", sessions.len());
        }

        async fn remove_from_conference(&self, session_id: &SessionId) {
            let mut sessions = self.bridge_sessions.lock().await;
            sessions.retain(|id| id != session_id);
            println!("Removed session from conference. Remaining participants: {}", sessions.len());
        }
    }

    #[async_trait::async_trait]
    impl CallHandler for ConferenceHandler {
        async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
            self.add_to_conference(call.id.clone()).await;
            CallDecision::Accept
        }

        async fn on_call_ended(&self, call: CallSession, reason: &str) {
            self.remove_from_conference(&call.id).await;
            println!("Conference call {} ended: {}", call.id(), reason);
        }
    }

    pub async fn run() -> crate::errors::Result<()> {
        let session_mgr = SessionManagerBuilder::new()
            .with_sip_port(5060)
            .with_handler(Arc::new(ConferenceHandler::new()))
            .build()
            .await?;
        
        session_mgr.start().await?;
        println!("Conference bridge running on port 5060");
        
        tokio::signal::ctrl_c().await?;
        Ok(())
    }
} 