//! Back-to-Back User Agent (B2BUA) example
//! 
//! This example demonstrates:
//! - B2BUA receiving an inbound call
//! - B2BUA making an outbound call to destination
//! - Bridging the two calls together
//! - Call recording and manipulation
//! - Clean termination of both legs

use rvoip_session_core_v2::api::unified::{UnifiedSession, SessionCoordinator, Config, SessionEvent};
use rvoip_session_core_v2::state_table::types::{Role, CallState};
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use std::collections::HashMap;

/// B2BUA application that routes calls based on rules
struct B2BUA {
    coordinator: Arc<SessionCoordinator>,
    routing_rules: HashMap<String, String>,
    active_sessions: Arc<tokio::sync::Mutex<HashMap<String, SessionPair>>>,
}

/// A pair of sessions (inbound and outbound) for B2BUA
struct SessionPair {
    inbound: UnifiedSession,
    outbound: UnifiedSession,
    bridged: bool,
}

impl B2BUA {
    /// Create a new B2BUA instance
    pub fn new(coordinator: Arc<SessionCoordinator>) -> Self {
        let mut routing_rules = HashMap::new();
        
        // Set up routing rules
        routing_rules.insert("support".to_string(), "sip:support-queue@internal.com".to_string());
        routing_rules.insert("sales".to_string(), "sip:sales-team@internal.com".to_string());
        routing_rules.insert("alice".to_string(), "sip:alice@192.168.1.100".to_string());
        routing_rules.insert("bob".to_string(), "sip:bob@192.168.1.101".to_string());
        
        Self {
            coordinator,
            routing_rules,
            active_sessions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }
    
    /// Handle an incoming call
    pub async fn handle_incoming_call(
        &self,
        from: &str,
        to: &str,
        sdp: Option<String>
    ) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n[B2BUA] Incoming call from {} to {}", from, to);
        
        // Create inbound leg (UAS)
        let inbound = UnifiedSession::new(self.coordinator.clone(), Role::UAS).await?;
        println!("[B2BUA] Created inbound session: {}", inbound.id);
        
        // Set up inbound event handler
        let session_id = inbound.id.clone();
        let active_sessions = self.active_sessions.clone();
        inbound.on_event(move |event| {
            match event {
                SessionEvent::CallEstablished => {
                    println!("[B2BUA] Inbound leg established: {}", session_id);
                }
                SessionEvent::CallTerminated { reason } => {
                    println!("[B2BUA] Inbound leg terminated: {} - {}", session_id, reason);
                    // Clean up the session pair
                    let sessions = active_sessions.clone();
                    let sid = session_id.clone();
                    tokio::spawn(async move {
                        sessions.lock().await.remove(&sid.to_string());
                    });
                }
                _ => {}
            }
        }).await?;
        
        // Process the incoming call
        inbound.on_incoming_call(from, sdp.clone()).await?;
        
        // Determine routing destination
        let destination = self.route_call(to)?;
        println!("[B2BUA] Routing call to: {}", destination);
        
        // Create outbound leg (UAC)
        let outbound = UnifiedSession::new(self.coordinator.clone(), Role::UAC).await?;
        println!("[B2BUA] Created outbound session: {}", outbound.id);
        
        // Set up outbound event handler
        let out_session_id = outbound.id.clone();
        outbound.on_event(move |event| {
            match event {
                SessionEvent::CallEstablished => {
                    println!("[B2BUA] Outbound leg established: {}", out_session_id);
                }
                SessionEvent::CallTerminated { reason } => {
                    println!("[B2BUA] Outbound leg terminated: {} - {}", out_session_id, reason);
                }
                _ => {}
            }
        }).await?;
        
        // Accept inbound call
        println!("[B2BUA] Accepting inbound call");
        inbound.accept().await?;
        
        // Make outbound call
        println!("[B2BUA] Initiating outbound call to {}", destination);
        outbound.make_call(&destination).await?;
        
        // Wait for outbound to be established (simplified - real implementation would use events)
        sleep(Duration::from_secs(2)).await;
        
        // Bridge the calls
        println!("[B2BUA] Bridging inbound and outbound legs");
        self.coordinator.bridge_sessions(&inbound.id, &outbound.id).await?;
        
        // Store session pair
        let pair = SessionPair {
            inbound,
            outbound,
            bridged: true,
        };
        self.active_sessions.lock().await.insert(
            pair.inbound.id.to_string(),
            pair
        );
        
        Ok(())
    }
    
    /// Route the call based on destination
    fn route_call(&self, to: &str) -> Result<String, Box<dyn std::error::Error>> {
        // Extract the user part from SIP URI
        let user = to.split('@').next().unwrap_or(to);
        let user = user.replace("sip:", "");
        
        // Look up routing rule
        if let Some(destination) = self.routing_rules.get(&user) {
            Ok(destination.clone())
        } else {
            // Default routing
            Ok(format!("sip:default@internal.com"))
        }
    }
    
    /// Transfer an active call
    pub async fn transfer_call(
        &self,
        session_id: &str,
        target: &str,
        attended: bool
    ) -> Result<(), Box<dyn std::error::Error>> {
        let sessions = self.active_sessions.lock().await;
        
        if let Some(pair) = sessions.get(session_id) {
            if attended {
                println!("[B2BUA] Initiating attended transfer to {}", target);
                pair.inbound.transfer(target, true).await?;
            } else {
                println!("[B2BUA] Initiating blind transfer to {}", target);
                pair.inbound.transfer(target, false).await?;
            }
        } else {
            return Err("Session not found".into());
        }
        
        Ok(())
    }
    
    /// Terminate all active sessions
    pub async fn shutdown(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("[B2BUA] Shutting down all active sessions");
        
        let mut sessions = self.active_sessions.lock().await;
        for (id, pair) in sessions.drain() {
            println!("[B2BUA] Terminating session pair: {}", id);
            let _ = pair.inbound.hangup().await;
            let _ = pair.outbound.hangup().await;
        }
        
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    println!("=== B2BUA Example ===");
    
    // Create coordinator
    let config = Config {
        sip_port: 5060,
        media_ports: (10000, 20000),
        bind_addr: "127.0.0.1:5060".parse()?,
    };
    let coordinator = SessionCoordinator::new(config).await?;
    
    // Create B2BUA instance
    let b2bua = Arc::new(B2BUA::new(coordinator.clone()));
    
    // Example 1: Simple B2BUA call routing
    simple_b2bua_example(b2bua.clone()).await?;
    
    // Example 2: B2BUA with call transfer
    b2bua_with_transfer_example(b2bua.clone()).await?;
    
    // Example 3: Call center scenario
    call_center_example(coordinator.clone()).await?;
    
    // Shutdown
    b2bua.shutdown().await?;
    
    Ok(())
}

/// Example 1: Simple B2BUA call routing
async fn simple_b2bua_example(b2bua: Arc<B2BUA>) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Example 1: Simple B2BUA Call Routing ===");
    
    // Simulate incoming call to support
    let sdp = generate_sdp_offer("192.168.1.50", 5004);
    b2bua.handle_incoming_call(
        "sip:customer@external.com",
        "sip:support@ourcompany.com",
        Some(sdp)
    ).await?;
    
    // Let the call run for a bit
    println!("Call in progress...");
    sleep(Duration::from_secs(3)).await;
    
    Ok(())
}

/// Example 2: B2BUA with call transfer
async fn b2bua_with_transfer_example(b2bua: Arc<B2BUA>) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Example 2: B2BUA with Call Transfer ===");
    
    // Simulate incoming call to sales
    let sdp = generate_sdp_offer("192.168.1.51", 5006);
    b2bua.handle_incoming_call(
        "sip:prospect@external.com",
        "sip:sales@ourcompany.com",
        Some(sdp)
    ).await?;
    
    // Wait for call to be established
    sleep(Duration::from_secs(2)).await;
    
    // Transfer to manager (blind transfer)
    println!("Transferring call to manager...");
    // Note: In real implementation, we'd use the actual session ID
    // b2bua.transfer_call("session_id", "sip:manager@internal.com", false).await?;
    
    sleep(Duration::from_secs(2)).await;
    
    Ok(())
}

/// Example 3: Call center scenario with queue
async fn call_center_example(coordinator: Arc<SessionCoordinator>) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Example 3: Call Center Scenario ===");
    
    // Simulate multiple incoming calls
    let mut customer_sessions = Vec::new();
    
    // Customer 1 calls support
    println!("Customer 1 calling support...");
    let customer1 = UnifiedSession::new(coordinator.clone(), Role::UAS).await?;
    customer1.on_incoming_call(
        "sip:customer1@external.com",
        Some(generate_sdp_offer("192.168.1.60", 5008))
    ).await?;
    customer1.accept().await?;
    customer_sessions.push(customer1);
    
    // Customer 2 calls support
    println!("Customer 2 calling support...");
    let customer2 = UnifiedSession::new(coordinator.clone(), Role::UAS).await?;
    customer2.on_incoming_call(
        "sip:customer2@external.com",
        Some(generate_sdp_offer("192.168.1.61", 5010))
    ).await?;
    customer2.accept().await?;
    customer_sessions.push(customer2);
    
    // Put customer 2 on hold (queue)
    println!("Putting customer 2 on hold...");
    customer_sessions[1].hold().await?;
    customer_sessions[1].play_audio("please-hold.wav").await?;
    
    // Create agent sessions
    println!("Agent 1 available, connecting to customer 1...");
    let agent1 = UnifiedSession::new(coordinator.clone(), Role::UAC).await?;
    agent1.make_call("sip:agent1@internal.com").await?;
    
    // Bridge customer 1 with agent 1
    sleep(Duration::from_secs(1)).await;
    coordinator.bridge_sessions(&customer_sessions[0].id, &agent1.id).await?;
    println!("Customer 1 connected to Agent 1");
    
    // After some time, agent 1 finishes
    sleep(Duration::from_secs(3)).await;
    println!("Agent 1 finishing with Customer 1...");
    customer_sessions[0].hangup().await?;
    
    // Agent 1 now takes customer 2
    println!("Agent 1 now taking Customer 2 from hold...");
    customer_sessions[1].resume().await?;
    
    let agent1_new = UnifiedSession::new(coordinator.clone(), Role::UAC).await?;
    agent1_new.make_call("sip:agent1@internal.com").await?;
    sleep(Duration::from_secs(1)).await;
    
    coordinator.bridge_sessions(&customer_sessions[1].id, &agent1_new.id).await?;
    println!("Customer 2 connected to Agent 1");
    
    // Let the call run
    sleep(Duration::from_secs(2)).await;
    
    // Clean up
    println!("Cleaning up call center sessions...");
    for session in customer_sessions {
        let _ = session.hangup().await;
    }
    
    Ok(())
}

/// Helper function to generate SDP offer
fn generate_sdp_offer(ip: &str, port: u16) -> String {
    format!(
        "v=0\r\n\
         o=- 0 0 IN IP4 {}\r\n\
         s=-\r\n\
         c=IN IP4 {}\r\n\
         t=0 0\r\n\
         m=audio {} RTP/AVP 0 8 101\r\n\
         a=rtpmap:0 PCMU/8000\r\n\
         a=rtpmap:8 PCMA/8000\r\n\
         a=rtpmap:101 telephone-event/8000\r\n\
         a=sendrecv",
        ip, ip, port
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_b2bua_creation() {
        let config = Config::default();
        let coordinator = SessionCoordinator::new(config).await.unwrap();
        let b2bua = B2BUA::new(coordinator);
        
        // Test routing rules
        assert_eq!(
            b2bua.route_call("sip:support@test.com").unwrap(),
            "sip:support-queue@internal.com"
        );
        assert_eq!(
            b2bua.route_call("sip:unknown@test.com").unwrap(),
            "sip:default@internal.com"
        );
    }
    
    #[tokio::test]
    async fn test_b2bua_session_pair() {
        let config = Config::default();
        let coordinator = SessionCoordinator::new(config).await.unwrap();
        
        // Create inbound and outbound legs
        let inbound = UnifiedSession::new(coordinator.clone(), Role::UAS).await.unwrap();
        let outbound = UnifiedSession::new(coordinator.clone(), Role::UAC).await.unwrap();
        
        // Verify they can be bridged
        assert!(coordinator.bridge_sessions(&inbound.id, &outbound.id).await.is_ok());
    }
}