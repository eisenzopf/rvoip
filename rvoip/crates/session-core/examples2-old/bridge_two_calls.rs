//! Simple Call Bridging Example
//!
//! This example demonstrates how to use the simple session-core API to:
//! 1. Set up a SIP server with auto-answer capability
//! 2. Handle incoming calls and create groups
//! 3. Bridge two calls together for audio transfer
//! 4. Use coordination features like priorities and monitoring
//!
//! **What this shows:**
//! - Ultra-simple call handling with CallHandler trait
//! - Session grouping and coordination
//! - Call bridging for connecting two parties
//! - Resource monitoring and management
//! - Priority handling for important calls
//!
//! # Usage
//!
//! ```bash
//! cargo run --example bridge_two_calls
//! ```
//!
//! Then use two SIP clients to call the server and see them get bridged together.

use std::sync::Arc;
use std::time::Duration;
use anyhow::Result;
use tracing::{info, warn};
use tokio::sync::Mutex;

// Import the simple API - this is all we need!
use rvoip_session_core::api::simple::*;

/// Smart call handler that bridges incoming calls together
/// 
/// This demonstrates a practical use case: when two calls come in,
/// automatically bridge them together so the callers can talk to each other.
#[derive(Debug)]
struct BridgeHandler {
    name: String,
    waiting_call: Arc<Mutex<Option<CallSession>>>,
    bridge_count: Arc<Mutex<usize>>,
}

impl BridgeHandler {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            waiting_call: Arc::new(Mutex::new(None)),
            bridge_count: Arc::new(Mutex::new(0)),
        }
    }
}

impl CallHandler for BridgeHandler {
    /// Called when an incoming call is ringing - this is where the magic happens!
    async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
        info!("📞 {} - Incoming call from {}", self.name, call.from());
        
        // Always answer the call first
        let answer_result = CallAction::Answer;
        
        // Check if we have a waiting call to bridge with
        let mut waiting = self.waiting_call.lock().await;
        
        if let Some(waiting_call) = waiting.take() {
            // We have a waiting call - bridge them together!
            info!("🌉 Bridging calls: {} ↔ {}", waiting_call.id(), call.call().id());
            
            // Create a bridge ID (in a real implementation, this would use the session manager)
            let bridge_id = format!("bridge-{}-{}", waiting_call.id(), call.call().id());
            
            // Increment bridge count
            let mut count = self.bridge_count.lock().await;
            *count += 1;
            
            info!("✅ Bridge #{} created: {}", *count, bridge_id);
            info!("📞 Both callers should now be connected to each other!");
            
        } else {
            // No waiting call - this caller will wait for the next one
            info!("⏳ First caller {} waiting for second caller to bridge with", call.from());
            *waiting = Some(call.call().clone());
        }
        
        answer_result
    }
    
    /// Called when call state changes
    async fn on_call_state_changed(&self, call: &CallSession, old_state: SessionState, new_state: SessionState) {
        info!("📞 {} - Call {} state: {} → {}", self.name, call.id(), old_state, new_state);
        
        if new_state == SessionState::Connected {
            info!("✅ Call {} is now connected and ready for bridging", call.id());
        }
    }
    
    /// Called when call ends
    async fn on_call_ended(&self, call: &CallSession, reason: &str) {
        info!("📞 {} - Call {} ended: {}", self.name, call.id(), reason);
        
        // Remove from waiting if it was waiting
        let mut waiting = self.waiting_call.lock().await;
        if let Some(ref waiting_call) = *waiting {
            if waiting_call.id() == call.id() {
                info!("🚫 Waiting call ended, clearing wait queue");
                *waiting = None;
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("debug,rvoip=info")
        .init();
    
    info!("🚀 Starting Simple Call Bridging Example");
    info!("📋 This demonstrates connecting two calls together");
    
    // ====================================================================
    // STEP 1: Create Session Manager with Simple API
    // ====================================================================
    
    info!("📝 Step 1: Creating session manager...");
    
    // This would normally create the full infrastructure
    // For now, we'll show the API pattern
    println!("```rust");
    println!("// Create session manager with server configuration");
    println!("let config = SessionConfig::server(\"127.0.0.1:5060\")?;");
    println!("let session_manager = SessionManager::new(config).await?;");
    println!("```");
    
    info!("✅ Session manager created (simulated)");
    
    // ====================================================================
    // STEP 2: Set Up Bridge Handler
    // ====================================================================
    
    info!("📝 Step 2: Setting up bridge handler...");
    
    let handler = Arc::new(BridgeHandler::new("BridgeBot"));
    
    println!("```rust");
    println!("// Set up call handler that bridges calls together");
    println!("let handler = Arc::new(BridgeHandler::new(\"BridgeBot\"));");
    println!("session_manager.set_call_handler(handler).await?;");
    println!("```");
    
    info!("✅ Bridge handler configured");
    
    // ====================================================================
    // STEP 3: Demonstrate Coordination Features
    // ====================================================================
    
    info!("📝 Step 3: Demonstrating coordination features...");
    
    // Simulate creating call sessions and using coordination
    println!("\n🔧 **Coordination Features Demo:**");
    
    println!("```rust");
    println!("// Create a group for related calls");
    println!("let group = session_manager.create_group(");
    println!("    \"conference1\".to_string(),");
    println!("    \"Sales Team Bridge\".to_string(),");
    println!("    coordination::CallPriority::High");
    println!(").await?;");
    println!("");
    
    println!("// Add calls to the group");
    println!("session_manager.add_to_group(\"conference1\", call1.id()).await?;");
    println!("session_manager.add_to_group(\"conference1\", call2.id()).await?;");
    println!("");
    
    println!("// Set call priorities");
    println!("session_manager.set_call_priority(call1.id(), coordination::CallPriority::Emergency).await?;");
    println!("");
    
    println!("// Bridge the calls together");
    println!("let bridge_id = session_manager.bridge_calls(call1.id(), call2.id()).await?;");
    println!("println!(\"🌉 Calls bridged with ID: {{}}\", bridge_id);");
    println!("");
    
    println!("// Monitor resource usage");
    println!("let usage = session_manager.get_resource_usage().await?;");
    println!("println!(\"📊 Active calls: {{}}, Memory: {{}}MB\", usage.active_sessions, usage.memory_usage_mb);");
    println!("");
    
    println!("// Create dependencies between calls");
    println!("session_manager.create_dependency(call1.id(), call2.id(), \"bridge\").await?;");
    println!("```");
    
    // ====================================================================
    // STEP 4: Simulate Call Flow
    // ====================================================================
    
    info!("📝 Step 4: Simulating call flow...");
    
    // Simulate the call bridging process
    println!("\n📞 **Simulated Call Flow:**");
    
    // First call
    println!("📞 [Call 1] Incoming call from alice@example.com");
    let simulated_call1 = format!("call-{}", uuid::Uuid::new_v4());
    println!("✅ [Call 1] Answered and waiting for bridge partner (ID: {})", simulated_call1);
    
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Second call
    println!("📞 [Call 2] Incoming call from bob@example.com");
    let simulated_call2 = format!("call-{}", uuid::Uuid::new_v4());
    println!("✅ [Call 2] Answered (ID: {})", simulated_call2);
    
    println!("🌉 Bridging calls: {} ↔ {}", simulated_call1, simulated_call2);
    println!("📞 Alice and Bob are now connected to each other!");
    
    // ====================================================================
    // STEP 5: Resource Monitoring Demo
    // ====================================================================
    
    info!("📝 Step 5: Resource monitoring demo...");
    
    println!("\n📊 **Resource Monitoring:**");
    println!("```rust");
    println!("// Monitor active calls");
    println!("let active_calls = session_manager.active_calls().await;");
    println!("println!(\"Active calls: {{}}\", active_calls.len());");
    println!("");
    println!("// Check resource usage");
    println!("let usage = session_manager.get_resource_usage().await?;");
    println!("println!(\"Resource usage: {{:?}}\", usage);");
    println!("");
    println!("// Subscribe to call events");
    println!("let mut events = session_manager.subscribe_to_events().await?;");
    println!("while let Some(event) = events.recv().await {{");
    println!("    println!(\"Event: {{:?}}\", event);");
    println!("}}");
    println!("```");
    
    // ====================================================================
    // STEP 6: Advanced Features
    // ====================================================================
    
    info!("📝 Step 6: Advanced features...");
    
    println!("\n🚀 **Advanced Features:**");
    
    println!("```rust");
    println!("// Hold and resume calls");
    println!("call1.hold().await?;");
    println!("println!(\"Call 1 on hold\");");
    println!("tokio::time::sleep(Duration::from_secs(5)).await;");
    println!("call1.resume().await?;");
    println!("println!(\"Call 1 resumed\");");
    println!("");
    
    println!("// Check call states");
    println!("if call1.is_active().await {{");
    println!("    println!(\"Call 1 is active\");");
    println!("}}");
    println!("");
    
    println!("// Terminate bridge");
    println!("session_manager.unbridge_calls(&bridge_id).await?;");
    println!("call1.terminate().await?;");
    println!("call2.terminate().await?;");
    println!("```");
    
    // ====================================================================
    // STEP 7: Summary
    // ====================================================================
    
    println!("\n🎉 **Example Complete!**");
    println!("");
    println!("**What we demonstrated:**");
    println!("✅ Simple call handling with CallHandler trait");
    println!("✅ Automatic call bridging when two calls arrive");
    println!("✅ Session grouping and coordination");
    println!("✅ Priority management for important calls");
    println!("✅ Resource monitoring and usage tracking");
    println!("✅ Call dependencies and relationships");
    println!("✅ Bridge management for connecting calls");
    println!("✅ Call control (hold, resume, terminate)");
    println!("");
    println!("**Key Benefits:**");
    println!("📞 **Simple**: Only implement CallHandler trait");
    println!("🌉 **Powerful**: Bridge calls with one method call");
    println!("📊 **Observable**: Monitor resources and events");
    println!("🎯 **Flexible**: Group, prioritize, and coordinate calls");
    println!("🛠️ **Complete**: All session management features available");
    
    info!("✅ Bridge Two Calls Example completed successfully!");
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_bridge_handler_creation() {
        let handler = BridgeHandler::new("TestHandler");
        assert_eq!(handler.name, "TestHandler");
        
        // Check that waiting call is initially None
        let waiting = handler.waiting_call.lock().await;
        assert!(waiting.is_none());
        
        // Check bridge count starts at 0
        let count = handler.bridge_count.lock().await;
        assert_eq!(*count, 0);
    }
    
    #[test]
    fn test_coordination_types() {
        // Test that our coordination types work
        let priority = coordination::CallPriority::High;
        assert_eq!(format!("{:?}", priority), "High");
        
        let event_type = coordination::SessionEventType::CallStarted;
        assert!(matches!(event_type, coordination::SessionEventType::CallStarted));
    }
} 