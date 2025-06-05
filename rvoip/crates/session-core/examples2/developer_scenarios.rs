//! Developer Scenarios for Session Manager
//!
//! This file shows simple, practical scenarios that developers want to create
//! using the session manager. Each scenario covers common real-world use cases
//! for both SIP CLIENTS and SIP SERVERS.

use rvoip_session_core::api::simple::*;
use std::sync::Arc;
use std::time::Duration;

// =============================================================================
// SIP SERVER SCENARIOS (Handle Incoming Calls)
// =============================================================================

// =============================================================================
// SCENARIO 1: Auto-Answer Server (Simplest Possible)
// =============================================================================

/// Just answer every call automatically
struct AutoAnswerServer;

impl CallHandler for AutoAnswerServer {
    async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
        println!("📞 Auto-answering call from {}", call.from());
        CallAction::Answer
    }
}

// Usage:
// session_manager.set_call_handler(Arc::new(AutoAnswerServer)).await?;

// =============================================================================
// SCENARIO 2: Voicemail Server
// =============================================================================

/// Answer calls, let them leave a message, then hang up
struct VoicemailServer;

impl CallHandler for VoicemailServer {
    async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
        println!("📞 Voicemail: Recording message from {}", call.from());
        CallAction::Answer
    }
    
    async fn on_call_state_changed(&self, call: &CallSession, _old: SessionState, new: SessionState) {
        if new == SessionState::Connected {
            println!("🎙️ Now recording voicemail for {}", call.id());
            
            // In real implementation: start recording
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            
            // Hang up after 30 seconds
            let _ = call.terminate().await;
            println!("📥 Voicemail saved");
        }
    }
}

// =============================================================================
// SCENARIO 3: Call Screening (Check Caller ID)
// =============================================================================

/// Only accept calls from allowed numbers
struct CallScreening {
    allowed_numbers: Vec<String>,
}

impl CallScreening {
    fn new(allowed: Vec<&str>) -> Self {
        Self {
            allowed_numbers: allowed.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl CallHandler for CallScreening {
    async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
        if let Some(username) = call.caller_username() {
            if self.allowed_numbers.contains(&username.to_string()) {
                println!("✅ Accepting call from allowed caller: {}", username);
                return CallAction::Answer;
            }
        }
        
        println!("❌ Blocking unknown caller: {}", call.from());
        CallAction::RejectWith {
            status: rvoip_sip_core::StatusCode::Forbidden,
            reason: "Not authorized".to_string(),
        }
    }
}

// Usage:
// let screener = CallScreening::new(vec!["alice", "bob", "carol"]);
// session_manager.set_call_handler(Arc::new(screener)).await?;

// =============================================================================
// SCENARIO 4: Business Hours Handler
// =============================================================================

/// Accept calls during business hours, reject after hours
struct BusinessHours {
    open_hour: u32,
    close_hour: u32,
}

impl BusinessHours {
    fn new(open: u32, close: u32) -> Self {
        Self { open_hour: open, close_hour: close }
    }
    
    fn is_open(&self) -> bool {
        let now = chrono::Local::now();
        let hour = now.hour();
        hour >= self.open_hour && hour < self.close_hour
    }
}

impl CallHandler for BusinessHours {
    async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
        if self.is_open() {
            println!("🏢 Business is open - accepting call from {}", call.from());
            CallAction::Answer
        } else {
            println!("🔒 Business is closed - rejecting call from {}", call.from());
            CallAction::RejectWith {
                status: rvoip_sip_core::StatusCode::TemporarilyUnavailable,
                reason: "Closed".to_string(),
            }
        }
    }
}

// Usage:
// let hours = BusinessHours::new(9, 17); // 9 AM to 5 PM
// session_manager.set_call_handler(Arc::new(hours)).await?;

// =============================================================================
// SCENARIO 5: Simple Conference Bridge
// =============================================================================

/// Connect all callers together in a conference
struct ConferenceBridge {
    participants: Arc<tokio::sync::Mutex<Vec<CallSession>>>,
}

impl ConferenceBridge {
    fn new() -> Self {
        Self {
            participants: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }
}

impl CallHandler for ConferenceBridge {
    async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
        println!("📞 Adding {} to conference", call.from());
        
        // Add to participant list
        let mut participants = self.participants.lock().await;
        participants.push(call.call().clone());
        
        println!("👥 Conference now has {} participants", participants.len());
        
        CallAction::Answer
    }
    
    async fn on_call_ended(&self, call: &CallSession, _reason: &str) {
        println!("👋 {} left the conference", call.id());
        
        // Remove from participant list
        let mut participants = self.participants.lock().await;
        participants.retain(|p| p.id() != call.id());
        
        println!("👥 Conference now has {} participants", participants.len());
    }
}

// =============================================================================
// SCENARIO 6: Call Queue/ACD (Automatic Call Distributor)
// =============================================================================

/// Queue incoming calls and distribute to available agents
struct CallQueue {
    waiting_calls: Arc<tokio::sync::Mutex<Vec<CallSession>>>,
    available_agents: Arc<tokio::sync::Mutex<Vec<String>>>,
}

impl CallQueue {
    fn new(agents: Vec<&str>) -> Self {
        Self {
            waiting_calls: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            available_agents: Arc::new(tokio::sync::Mutex::new(
                agents.iter().map(|s| s.to_string()).collect()
            )),
        }
    }
}

impl CallHandler for CallQueue {
    async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
        let mut agents = self.available_agents.lock().await;
        
        if let Some(agent) = agents.pop() {
            println!("📞 Routing call from {} to agent {}", call.from(), agent);
            CallAction::Answer
        } else {
            println!("⏳ No agents available, queueing call from {}", call.from());
            let mut queue = self.waiting_calls.lock().await;
            queue.push(call.call().clone());
            CallAction::Answer // Answer and play hold music
        }
    }
}

// =============================================================================
// SCENARIO 7: Multi-Tenant Server
// =============================================================================

/// Handle calls for multiple tenants/companies
struct MultiTenantServer {
    tenant_configs: std::collections::HashMap<String, String>,
}

impl MultiTenantServer {
    fn new() -> Self {
        let mut configs = std::collections::HashMap::new();
        configs.insert("company1.example.com".to_string(), "Company 1 Config".to_string());
        configs.insert("company2.example.com".to_string(), "Company 2 Config".to_string());
        
        Self { tenant_configs: configs }
    }
    
    fn get_tenant_from_call(&self, call: &IncomingCall) -> Option<String> {
        // Extract tenant from To header domain
        call.to().split('@').nth(1).map(|s| s.to_string())
    }
}

impl CallHandler for MultiTenantServer {
    async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
        if let Some(tenant) = self.get_tenant_from_call(call) {
            if self.tenant_configs.contains_key(&tenant) {
                println!("📞 Handling call for tenant: {}", tenant);
                CallAction::Answer
            } else {
                println!("❌ Unknown tenant: {}", tenant);
                CallAction::RejectWith {
                    status: rvoip_sip_core::StatusCode::NotFound,
                    reason: "Tenant not found".to_string(),
                }
            }
        } else {
            println!("❌ No tenant specified in call");
            CallAction::Reject
        }
    }
}

// =============================================================================
// SIP CLIENT SCENARIOS (Make Outgoing Calls)
// =============================================================================

// =============================================================================
// SCENARIO 8: Simple Outgoing Call Client
// =============================================================================

/// Make a simple outgoing call
struct SimpleCallClient {
    session_manager: Arc<SessionManager>,
}

impl SimpleCallClient {
    fn new(session_manager: Arc<SessionManager>) -> Self {
        Self { session_manager }
    }
    
    async fn make_call(&self, from: &str, to: &str) -> Result<CallSession, Box<dyn std::error::Error>> {
        println!("📞 Making call from {} to {}", from, to);
        
        let call = self.session_manager.make_call(from, to, None).await?;
        
        // Wait for call to connect
        while call.is_connecting().await {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        
        if call.is_active().await {
            println!("✅ Call connected!");
        } else {
            println!("❌ Call failed to connect");
        }
        
        Ok(call)
    }
}

// Usage:
// let client = SimpleCallClient::new(session_manager.clone());
// let call = client.make_call("sip:alice@client.com", "sip:bob@server.com").await?;

// =============================================================================
// SCENARIO 9: Auto-Dialer Client
// =============================================================================

/// Automatically dial a list of numbers
struct AutoDialer {
    session_manager: Arc<SessionManager>,
    numbers_to_call: Vec<String>,
    call_duration: Duration,
}

impl AutoDialer {
    fn new(session_manager: Arc<SessionManager>, numbers: Vec<&str>) -> Self {
        Self {
            session_manager,
            numbers_to_call: numbers.iter().map(|s| s.to_string()).collect(),
            call_duration: Duration::from_secs(10),
        }
    }
    
    async fn start_dialing(&self, from: &str) -> Result<(), Box<dyn std::error::Error>> {
        for number in &self.numbers_to_call {
            println!("📞 Auto-dialing {}", number);
            
            match self.session_manager.make_call(from, number, None).await {
                Ok(call) => {
                    // Wait for connection
                    let mut attempts = 0;
                    while call.is_connecting().await && attempts < 100 {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        attempts += 1;
                    }
                    
                    if call.is_active().await {
                        println!("✅ Connected to {}, talking for {:?}", number, self.call_duration);
                        tokio::time::sleep(self.call_duration).await;
                        call.terminate().await?;
                        println!("👋 Call to {} ended", number);
                    } else {
                        println!("❌ Failed to connect to {}", number);
                    }
                }
                Err(e) => println!("❌ Error calling {}: {}", number, e),
            }
            
            // Wait between calls
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        
        Ok(())
    }
}

// Usage:
// let dialer = AutoDialer::new(session_manager.clone(), vec!["sip:test1@example.com", "sip:test2@example.com"]);
// dialer.start_dialing("sip:client@mycompany.com").await?;

// =============================================================================
// SCENARIO 10: Softphone Client
// =============================================================================

/// Simple softphone with register/call/hangup functionality
struct SoftphoneClient {
    session_manager: Arc<SessionManager>,
    my_uri: String,
    active_call: Arc<tokio::sync::Mutex<Option<CallSession>>>,
}

impl SoftphoneClient {
    fn new(session_manager: Arc<SessionManager>, my_uri: String) -> Self {
        Self {
            session_manager,
            my_uri,
            active_call: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }
    
    async fn place_call(&self, to: &str) -> Result<(), Box<dyn std::error::Error>> {
        let mut active = self.active_call.lock().await;
        
        if active.is_some() {
            println!("❌ Already on a call");
            return Ok(());
        }
        
        println!("📞 Placing call to {}", to);
        let call = self.session_manager.make_call(&self.my_uri, to, None).await?;
        *active = Some(call);
        
        println!("🔔 Calling...");
        Ok(())
    }
    
    async fn hangup(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut active = self.active_call.lock().await;
        
        if let Some(call) = active.take() {
            println!("👋 Hanging up call");
            call.terminate().await?;
        } else {
            println!("❌ No active call");
        }
        
        Ok(())
    }
    
    async fn get_call_status(&self) -> String {
        let active = self.active_call.lock().await;
        
        if let Some(call) = &*active {
            let state = call.state().await;
            format!("Call status: {:?}", state)
        } else {
            "No active call".to_string()
        }
    }
}

impl CallHandler for SoftphoneClient {
    async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
        println!("📞 Incoming call from {}", call.from());
        
        // Check if already on a call
        let active = self.active_call.lock().await;
        if active.is_some() {
            println!("📞 Rejecting call - already busy");
            return CallAction::RejectWith {
                status: rvoip_sip_core::StatusCode::BusyHere,
                reason: "Busy".to_string(),
            };
        }
        
        println!("📞 Accepting incoming call");
        CallAction::Answer
    }
}

// =============================================================================
// SCENARIO 11: Load Testing Client
// =============================================================================

/// Generate load by making many concurrent calls
struct LoadTestClient {
    session_manager: Arc<SessionManager>,
    concurrent_calls: usize,
    calls_per_second: f64,
}

impl LoadTestClient {
    fn new(session_manager: Arc<SessionManager>, concurrent: usize, rate: f64) -> Self {
        Self {
            session_manager,
            concurrent_calls: concurrent,
            calls_per_second: rate,
        }
    }
    
    async fn start_load_test(&self, from: &str, to: &str, duration: Duration) -> Result<(), Box<dyn std::error::Error>> {
        println!("🔥 Starting load test: {} concurrent calls at {} calls/sec for {:?}", 
                 self.concurrent_calls, self.calls_per_second, duration);
        
        let call_interval = Duration::from_secs_f64(1.0 / self.calls_per_second);
        let start_time = std::time::Instant::now();
        let mut call_count = 0;
        
        while start_time.elapsed() < duration {
            for _ in 0..self.concurrent_calls {
                let session_manager = self.session_manager.clone();
                let from = from.to_string();
                let to = to.to_string();
                
                tokio::spawn(async move {
                    match session_manager.make_call(&from, &to, None).await {
                        Ok(call) => {
                            // Keep call alive for a few seconds
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            let _ = call.terminate().await;
                        }
                        Err(e) => println!("Call failed: {}", e),
                    }
                });
                
                call_count += 1;
                if call_count % 100 == 0 {
                    println!("📊 Made {} calls so far", call_count);
                }
            }
            
            tokio::time::sleep(call_interval).await;
        }
        
        println!("✅ Load test completed: {} total calls", call_count);
        Ok(())
    }
}

// =============================================================================
// SCENARIO 12: Call Quality Monitor
// =============================================================================

/// Monitor call quality and collect metrics
struct CallQualityMonitor {
    session_manager: Arc<SessionManager>,
    quality_metrics: Arc<tokio::sync::Mutex<Vec<CallMetrics>>>,
}

#[derive(Debug, Clone)]
struct CallMetrics {
    call_id: String,
    duration: Duration,
    start_time: std::time::Instant,
    quality_score: f64,
}

impl CallQualityMonitor {
    fn new(session_manager: Arc<SessionManager>) -> Self {
        Self {
            session_manager,
            quality_metrics: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }
    
    async fn start_monitoring_call(&self, from: &str, to: &str) -> Result<(), Box<dyn std::error::Error>> {
        println!("📊 Starting quality monitoring call from {} to {}", from, to);
        
        let call = self.session_manager.make_call(from, to, None).await?;
        let start_time = std::time::Instant::now();
        
        // Monitor call for quality
        while call.is_active().await {
            tokio::time::sleep(Duration::from_secs(1)).await;
            
            // In real implementation: collect audio quality metrics
            let quality_score = 0.85; // Simulated
            println!("📊 Call quality: {:.2}", quality_score);
        }
        
        let duration = start_time.elapsed();
        let metrics = CallMetrics {
            call_id: call.id().to_string(),
            duration,
            start_time,
            quality_score: 0.85,
        };
        
        self.quality_metrics.lock().await.push(metrics);
        println!("📊 Call ended - quality metrics collected");
        
        Ok(())
    }
    
    async fn get_average_quality(&self) -> f64 {
        let metrics = self.quality_metrics.lock().await;
        if metrics.is_empty() {
            return 0.0;
        }
        
        let total: f64 = metrics.iter().map(|m| m.quality_score).sum();
        total / metrics.len() as f64
    }
}

// =============================================================================
// SCENARIO 13: Emergency Dialer
// =============================================================================

/// Automatically dial emergency services with priority
struct EmergencyDialer {
    session_manager: Arc<SessionManager>,
    emergency_numbers: Vec<String>,
}

impl EmergencyDialer {
    fn new(session_manager: Arc<SessionManager>) -> Self {
        Self {
            session_manager,
            emergency_numbers: vec![
                "sip:911@emergency.local".to_string(),
                "sip:emergency@backup.local".to_string(),
            ],
        }
    }
    
    async fn dial_emergency(&self, from: &str, location: &str) -> Result<CallSession, Box<dyn std::error::Error>> {
        println!("🚨 EMERGENCY CALL from {} at location: {}", from, location);
        
        for (i, number) in self.emergency_numbers.iter().enumerate() {
            println!("🚨 Trying emergency number {} (attempt {})", number, i + 1);
            
            match self.session_manager.make_call(from, number, None).await {
                Ok(call) => {
                    // Try to connect quickly
                    for _ in 0..50 { // 5 second timeout
                        if call.is_active().await {
                            println!("🚨 EMERGENCY CALL CONNECTED to {}", number);
                            
                            // Set high priority
                            let _ = self.session_manager.set_call_priority(call.id(), coordination::CallPriority::Emergency).await;
                            
                            return Ok(call);
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                    
                    // This number didn't work, try next
                    let _ = call.terminate().await;
                }
                Err(e) => {
                    println!("🚨 Failed to reach {}: {}", number, e);
                }
            }
        }
        
        Err("All emergency numbers failed".into())
    }
}

// =============================================================================
// SCENARIO 14: Callback Service Client
// =============================================================================

/// Initiate callback calls when requested
struct CallbackService {
    session_manager: Arc<SessionManager>,
    callback_queue: Arc<tokio::sync::Mutex<Vec<CallbackRequest>>>,
}

#[derive(Debug, Clone)]
struct CallbackRequest {
    customer_number: String,
    agent_number: String,
    scheduled_time: std::time::SystemTime,
    priority: coordination::CallPriority,
}

impl CallbackService {
    fn new(session_manager: Arc<SessionManager>) -> Self {
        Self {
            session_manager,
            callback_queue: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }
    
    async fn schedule_callback(&self, customer: &str, agent: &str, delay: Duration) {
        let request = CallbackRequest {
            customer_number: customer.to_string(),
            agent_number: agent.to_string(),
            scheduled_time: std::time::SystemTime::now() + delay,
            priority: coordination::CallPriority::Normal,
        };
        
        self.callback_queue.lock().await.push(request);
        println!("📞 Callback scheduled for {} in {:?}", customer, delay);
    }
    
    async fn process_callbacks(&self) -> Result<(), Box<dyn std::error::Error>> {
        loop {
            let mut queue = self.callback_queue.lock().await;
            let now = std::time::SystemTime::now();
            
            // Find callbacks that are ready
            let ready_callbacks: Vec<_> = queue
                .iter()
                .enumerate()
                .filter(|(_, cb)| cb.scheduled_time <= now)
                .map(|(i, cb)| (i, cb.clone()))
                .collect();
            
            // Remove processed callbacks
            for (index, _) in ready_callbacks.iter().rev() {
                queue.remove(*index);
            }
            drop(queue);
            
            // Process ready callbacks
            for (_, callback) in ready_callbacks {
                println!("📞 Processing callback: {} -> {}", callback.agent_number, callback.customer_number);
                
                // First call the agent
                match self.session_manager.make_call(&callback.agent_number, "sip:callback@system.local", None).await {
                    Ok(agent_call) => {
                        // Wait for agent to answer
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        
                        if agent_call.is_active().await {
                            // Now call the customer and bridge them
                            match self.session_manager.make_call("sip:system@callback.local", &callback.customer_number, None).await {
                                Ok(customer_call) => {
                                    // Bridge the calls
                                    let _ = self.session_manager.bridge_calls(agent_call.id(), customer_call.id()).await;
                                    println!("🌉 Callback connected: agent and customer bridged");
                                }
                                Err(e) => {
                                    println!("❌ Failed to reach customer: {}", e);
                                    let _ = agent_call.terminate().await;
                                }
                            }
                        } else {
                            println!("❌ Agent didn't answer callback");
                            let _ = agent_call.terminate().await;
                        }
                    }
                    Err(e) => println!("❌ Failed to reach agent: {}", e),
                }
            }
            
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

// =============================================================================
// PEER-TO-PEER SIP CLIENT SCENARIOS (Direct Client-to-Client)
// =============================================================================

// =============================================================================
// SCENARIO 15: Peer-to-Peer Direct Call
// =============================================================================

/// Simple peer-to-peer calling between two SIP clients
struct PeerToPeerClient {
    session_manager: Arc<SessionManager>,
    my_uri: String,
    my_address: String,
}

impl PeerToPeerClient {
    fn new(session_manager: Arc<SessionManager>, my_uri: String, my_address: String) -> Self {
        Self { session_manager, my_uri, my_address }
    }
    
    async fn call_peer(&self, peer_uri: &str) -> Result<CallSession, Box<dyn std::error::Error>> {
        println!("📞 {} calling peer directly: {}", self.my_uri, peer_uri);
        
        // Direct peer-to-peer call (no registrar/proxy)
        let call = self.session_manager.make_call(&self.my_uri, peer_uri, None).await?;
        
        // Wait for connection
        let mut attempts = 0;
        while call.is_connecting().await && attempts < 100 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            attempts += 1;
        }
        
        if call.is_active().await {
            println!("✅ Peer-to-peer call connected!");
        } else {
            println!("❌ Peer-to-peer call failed");
        }
        
        Ok(call)
    }
    
    async fn start_listening(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("👂 {} listening for peer calls on {}", self.my_uri, self.my_address);
        self.session_manager.start_server(&self.my_address).await?;
        Ok(())
    }
}

impl CallHandler for PeerToPeerClient {
    async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
        println!("📞 Direct peer call from: {}", call.from());
        println!("📞 Accepting peer-to-peer call");
        CallAction::Answer
    }
    
    async fn on_call_state_changed(&self, call: &CallSession, _old: SessionState, new: SessionState) {
        if new == SessionState::Connected {
            println!("🎉 Peer-to-peer call established with {}", call.id());
        }
    }
}

// Usage:
// let client_a = PeerToPeerClient::new(manager_a, "sip:alice@192.168.1.10:5060", "192.168.1.10:5060");
// let client_b = PeerToPeerClient::new(manager_b, "sip:bob@192.168.1.11:5060", "192.168.1.11:5060");
// client_b.start_listening().await?;
// let call = client_a.call_peer("sip:bob@192.168.1.11:5060").await?;

// =============================================================================
// SCENARIO 16: Mesh Network Communication
// =============================================================================

/// Clients in a mesh network that can discover and call each other
struct MeshNetworkClient {
    session_manager: Arc<SessionManager>,
    my_uri: String,
    my_address: String,
    known_peers: Arc<tokio::sync::RwLock<Vec<String>>>,
    active_peer_calls: Arc<tokio::sync::Mutex<std::collections::HashMap<String, CallSession>>>,
}

impl MeshNetworkClient {
    fn new(session_manager: Arc<SessionManager>, my_uri: String, my_address: String) -> Self {
        Self {
            session_manager,
            my_uri,
            my_address,
            known_peers: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            active_peer_calls: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }
    
    async fn join_mesh(&self, bootstrap_peers: Vec<&str>) -> Result<(), Box<dyn std::error::Error>> {
        println!("🌐 {} joining mesh network", self.my_uri);
        
        // Add bootstrap peers
        let mut peers = self.known_peers.write().await;
        for peer in bootstrap_peers {
            peers.push(peer.to_string());
        }
        drop(peers);
        
        // Start listening for peer connections
        self.session_manager.set_call_handler(Arc::new(self.clone())).await?;
        self.session_manager.start_server(&self.my_address).await?;
        
        println!("🌐 {} joined mesh network with {} peers", self.my_uri, bootstrap_peers.len());
        Ok(())
    }
    
    async fn call_mesh_peer(&self, peer_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let peers = self.known_peers.read().await;
        
        if let Some(peer_uri) = peers.iter().find(|p| p.contains(peer_id)) {
            println!("📞 Calling mesh peer: {}", peer_uri);
            
            match self.session_manager.make_call(&self.my_uri, peer_uri, None).await {
                Ok(call) => {
                    let mut active_calls = self.active_peer_calls.lock().await;
                    active_calls.insert(peer_id.to_string(), call);
                    println!("✅ Mesh call initiated to {}", peer_id);
                }
                Err(e) => println!("❌ Failed to call mesh peer {}: {}", peer_id, e),
            }
        } else {
            println!("❌ Peer {} not found in mesh", peer_id);
        }
        
        Ok(())
    }
    
    async fn broadcast_to_mesh(&self, message: &str) -> Result<(), Box<dyn std::error::Error>> {
        println!("📢 Broadcasting to mesh: {}", message);
        
        let peers = self.known_peers.read().await;
        for peer_uri in peers.iter() {
            // In real implementation: send SIP MESSAGE or make brief call
            println!("📤 Sending to {}: {}", peer_uri, message);
        }
        
        Ok(())
    }
    
    async fn leave_mesh(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("👋 {} leaving mesh network", self.my_uri);
        
        // Hang up all active peer calls
        let mut active_calls = self.active_peer_calls.lock().await;
        for (peer_id, call) in active_calls.drain() {
            println!("👋 Ending call with mesh peer {}", peer_id);
            let _ = call.terminate().await;
        }
        
        Ok(())
    }
}

impl Clone for MeshNetworkClient {
    fn clone(&self) -> Self {
        Self {
            session_manager: self.session_manager.clone(),
            my_uri: self.my_uri.clone(),
            my_address: self.my_address.clone(),
            known_peers: self.known_peers.clone(),
            active_peer_calls: self.active_peer_calls.clone(),
        }
    }
}

impl CallHandler for MeshNetworkClient {
    async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
        println!("📞 Mesh peer call from: {}", call.from());
        
        // Check if this peer is in our mesh
        let peers = self.known_peers.read().await;
        let is_known_peer = peers.iter().any(|p| call.from().contains(p));
        
        if is_known_peer {
            println!("✅ Accepting call from known mesh peer");
            CallAction::Answer
        } else {
            println!("❓ Unknown peer, adding to mesh and accepting");
            // Add new peer to mesh
            drop(peers);
            let mut peers_write = self.known_peers.write().await;
            peers_write.push(call.from().to_string());
            CallAction::Answer
        }
    }
    
    async fn on_call_ended(&self, call: &CallSession, reason: &str) {
        println!("👋 Mesh peer call ended: {} (reason: {})", call.id(), reason);
        
        // Remove from active calls
        let mut active_calls = self.active_peer_calls.lock().await;
        active_calls.retain(|_, active_call| active_call.id() != call.id());
    }
}

// =============================================================================
// SCENARIO 17: Distributed Softphone Network
// =============================================================================

/// Multiple softphones that can call each other without a central server
struct DistributedSoftphone {
    session_manager: Arc<SessionManager>,
    my_identity: String,
    my_address: String,
    contacts: Arc<tokio::sync::RwLock<std::collections::HashMap<String, String>>>,
    current_call: Arc<tokio::sync::Mutex<Option<CallSession>>>,
}

impl DistributedSoftphone {
    fn new(session_manager: Arc<SessionManager>, name: String, address: String) -> Self {
        let my_identity = format!("sip:{}@{}", name, address);
        
        Self {
            session_manager,
            my_identity,
            my_address: address,
            contacts: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            current_call: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }
    
    async fn add_contact(&self, name: &str, uri: &str) {
        let mut contacts = self.contacts.write().await;
        contacts.insert(name.to_string(), uri.to_string());
        println!("📇 Added contact: {} -> {}", name, uri);
    }
    
    async fn call_contact(&self, contact_name: &str) -> Result<(), Box<dyn std::error::Error>> {
        let mut current = self.current_call.lock().await;
        
        if current.is_some() {
            println!("❌ Already on a call");
            return Ok(());
        }
        
        let contacts = self.contacts.read().await;
        if let Some(contact_uri) = contacts.get(contact_name) {
            println!("📞 Calling contact {} at {}", contact_name, contact_uri);
            
            match self.session_manager.make_call(&self.my_identity, contact_uri, None).await {
                Ok(call) => {
                    *current = Some(call);
                    println!("📞 Calling {}...", contact_name);
                }
                Err(e) => println!("❌ Failed to call {}: {}", contact_name, e),
            }
        } else {
            println!("❌ Contact '{}' not found", contact_name);
        }
        
        Ok(())
    }
    
    async fn hangup(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut current = self.current_call.lock().await;
        
        if let Some(call) = current.take() {
            println!("👋 Hanging up call");
            call.terminate().await?;
        } else {
            println!("❌ No active call to hang up");
        }
        
        Ok(())
    }
    
    async fn get_status(&self) -> String {
        let current = self.current_call.lock().await;
        let contacts = self.contacts.read().await;
        
        if let Some(call) = &*current {
            let state = call.state().await;
            format!("📞 Call status: {:?} | Contacts: {}", state, contacts.len())
        } else {
            format!("📴 Idle | Contacts: {}", contacts.len())
        }
    }
    
    async fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("🚀 Starting distributed softphone: {}", self.my_identity);
        self.session_manager.set_call_handler(Arc::new(self.clone())).await?;
        self.session_manager.start_server(&self.my_address).await?;
        Ok(())
    }
}

impl Clone for DistributedSoftphone {
    fn clone(&self) -> Self {
        Self {
            session_manager: self.session_manager.clone(),
            my_identity: self.my_identity.clone(),
            my_address: self.my_address.clone(),
            contacts: self.contacts.clone(),
            current_call: self.current_call.clone(),
        }
    }
}

impl CallHandler for DistributedSoftphone {
    async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
        let current = self.current_call.lock().await;
        
        if current.is_some() {
            println!("📞 Incoming call from {} - BUSY", call.from());
            return CallAction::RejectWith {
                status: rvoip_sip_core::StatusCode::BusyHere,
                reason: "Busy".to_string(),
            };
        }
        
        println!("📞 Incoming call from {} - ACCEPTING", call.from());
        CallAction::Answer
    }
    
    async fn on_call_state_changed(&self, call: &CallSession, _old: SessionState, new: SessionState) {
        match new {
            SessionState::Connected => {
                println!("✅ Call connected: {}", call.id());
                let mut current = self.current_call.lock().await;
                *current = Some(call.clone());
            }
            SessionState::Terminated => {
                println!("👋 Call ended: {}", call.id());
                let mut current = self.current_call.lock().await;
                *current = None;
            }
            _ => {}
        }
    }
}

// Usage example for distributed softphones:
// let alice = DistributedSoftphone::new(manager_a, "alice", "192.168.1.10:5060");
// let bob = DistributedSoftphone::new(manager_b, "bob", "192.168.1.11:5060");
// 
// alice.add_contact("Bob", "sip:bob@192.168.1.11:5060").await;
// bob.add_contact("Alice", "sip:alice@192.168.1.10:5060").await;
// 
// alice.start().await?;
// bob.start().await?;
// 
// alice.call_contact("Bob").await?;

// =============================================================================
// HOW TO USE THESE SCENARIOS
// =============================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🎯 **Session Manager Developer Scenarios**");
    println!("");
    println!("📋 **SIP SERVER Scenarios (Handle Incoming Calls):**");
    println!("1. Auto-Answer Server - Just answer every call");
    println!("2. Voicemail Server - Record messages and hang up");
    println!("3. Call Screening - Only accept allowed callers");
    println!("4. Business Hours - Accept 9-5, reject after hours");
    println!("5. Conference Bridge - Connect all callers together");
    println!("6. Call Queue/ACD - Queue calls for available agents");
    println!("7. Multi-Tenant Server - Handle multiple companies");
    println!("");
    println!("📱 **SIP CLIENT Scenarios (Make Outgoing Calls):**");
    println!("8. Simple Call Client - Make basic outgoing calls");
    println!("9. Auto-Dialer - Call a list of numbers automatically");
    println!("10. Softphone Client - Full softphone with call management");
    println!("11. Load Testing Client - Generate high call volume");
    println!("12. Call Quality Monitor - Monitor and measure call quality");
    println!("13. Emergency Dialer - High-priority emergency calling");
    println!("14. Callback Service - Schedule and manage callbacks");
    println!("");
    println!("📋 **PEER-TO-PEER SIP CLIENT Scenarios (Direct Client-to-Client):**");
    println!("15. Peer-to-Peer Direct Call - Simple peer-to-peer calling");
    println!("16. Mesh Network Communication - Clients in a mesh network");
    println!("17. Distributed Softphone Network - Multiple softphones");
    println!("");
    
    println!("🔧 **Usage Patterns:**");
    println!("");
    println!("```rust");
    println!("// SIP SERVER: Implement CallHandler");
    println!("session_manager.set_call_handler(Arc::new(AutoAnswerServer)).await?;");
    println!("");
    println!("// SIP CLIENT: Use make_call()");
    println!("let call = session_manager.make_call(from, to, None).await?;");
    println!("");
    println!("// COORDINATION: Use simple methods");
    println!("session_manager.bridge_calls(call1.id(), call2.id()).await?;");
    println!("session_manager.set_call_priority(call.id(), CallPriority::Emergency).await?;");
    println!("```");
    
    println!("");
    println!("**That's it!** Each scenario shows the pattern for:");
    println!("📞 **Servers**: Implement CallHandler trait → handle incoming calls");
    println!("📱 **Clients**: Use make_call() → handle outgoing calls"); 
    println!("🌉 **Coordination**: Use simple methods → bridge, group, prioritize");
    
    Ok(())
} 