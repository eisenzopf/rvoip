//! Complete SIP Client Example
//!
//! This example demonstrates how to implement a complete SIP user agent
//! that can register with a SIP server and make/receive calls.

use bytes::Bytes;
use rvoip_sip_core::prelude::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Main function - starts the SIP client and runs the example
#[tokio::main]
async fn main() {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    info!("SIP Core Complete Client Example");
    
    // Create a new SIP client
    let mut client = SipClient::new(
        "alice",
        "example.com",
        "192.168.1.100",
        5060,
        Some("password123"),
    );
    
    // Start the client
    client.start().await;
    
    // Register with the SIP server
    client.register().await;
    
    // Simulate receiving an incoming call
    info!("Waiting for incoming calls (or simulating one in 3 seconds)...");
    sleep(Duration::from_secs(3)).await;
    
    // Simulate an incoming call (in a real application, this would come from the network)
    client.simulate_incoming_call("bob@example.com").await;
    
    // Wait a moment, then make an outgoing call
    sleep(Duration::from_secs(5)).await;
    
    // Make an outgoing call
    client.make_call("charlie@example.com").await;
    
    // Wait a bit to let the call complete
    sleep(Duration::from_secs(10)).await;
    
    // Unregister from the server
    client.unregister().await;
    
    // Stop the client
    client.stop().await;
    
    info!("SIP Client example completed successfully!");
}

/// Represents a SIP client application
struct SipClient {
    // User configuration
    username: String,
    domain: String,
    local_ip: String,
    local_port: u16,
    password: Option<String>,
    
    // Runtime state
    registered: bool,
    active_calls: Arc<Mutex<HashMap<String, Call>>>,
    registration_expires: u32,
    next_cseq: u32,
    
    // Communication channels
    tx: Option<Sender<SipEvent>>,
    
    // For simulation
    simulated_server_ip: String,
    simulated_server_port: u16,
}

/// Represents a SIP event in the client
enum SipEvent {
    Stop,
    Register,
    Unregister,
    MakeCall(String),
    EndCall(String),
    IncomingCall {
        call_id: String,
        from: String,
        to: String,
    },
    // More events would be added in a real implementation
}

/// Represents an active call
struct Call {
    call_id: String,
    from: String,
    to: String,
    local_tag: String,
    remote_tag: Option<String>,
    state: CallState,
    created_at: Instant,
}

/// Represents the state of a call
enum CallState {
    Idle,
    Invited,
    Ringing,
    Connecting,
    Connected,
    Disconnecting,
    Disconnected,
}

impl SipClient {
    /// Create a new SIP client
    fn new(
        username: &str,
        domain: &str,
        local_ip: &str,
        local_port: u16,
        password: Option<&str>,
    ) -> Self {
        Self {
            username: username.to_string(),
            domain: domain.to_string(),
            local_ip: local_ip.to_string(),
            local_port,
            password: password.map(|p| p.to_string()),
            registered: false,
            active_calls: Arc::new(Mutex::new(HashMap::new())),
            registration_expires: 3600,
            next_cseq: 1,
            tx: None,
            simulated_server_ip: "10.0.0.1".to_string(),
            simulated_server_port: 5060,
        }
    }
    
    /// Start the SIP client
    async fn start(&mut self) {
        info!("Starting SIP client for {}@{}", self.username, self.domain);
        
        // Create channels for communication
        let (tx, rx) = mpsc::channel::<SipEvent>(100);
        self.tx = Some(tx.clone());
        
        // Clone data for the task
        let username = self.username.clone();
        let domain = self.domain.clone();
        let local_ip = self.local_ip.clone();
        let local_port = self.local_port;
        let active_calls = Arc::clone(&self.active_calls);
        
        // Spawn a task to handle events
        tokio::spawn(async move {
            Self::event_loop(rx, username, domain, local_ip, local_port, active_calls).await;
        });
        
        info!("SIP client started successfully");
    }
    
    /// Stop the SIP client
    async fn stop(&self) {
        info!("Stopping SIP client");
        
        if let Some(tx) = &self.tx {
            let _ = tx.send(SipEvent::Stop).await;
        }
        
        // Wait for a moment to allow the event loop to process the stop event
        sleep(Duration::from_millis(100)).await;
        
        info!("SIP client stopped");
    }
    
    /// Register with the SIP server
    async fn register(&self) {
        info!("Registering with SIP server");
        
        if let Some(tx) = &self.tx {
            let _ = tx.send(SipEvent::Register).await;
            
            // In a real client, we'd wait for a response from the server
            // For this example, we'll simulate a successful registration
            info!("Simulating successful registration");
        }
    }
    
    /// Unregister from the SIP server
    async fn unregister(&self) {
        info!("Unregistering from SIP server");
        
        if let Some(tx) = &self.tx {
            let _ = tx.send(SipEvent::Unregister).await;
            
            // In a real client, we'd wait for a response from the server
            info!("Simulating successful unregistration");
        }
    }
    
    /// Make an outgoing call
    async fn make_call(&self, target: &str) {
        info!("Making call to {}", target);
        
        if let Some(tx) = &self.tx {
            let _ = tx.send(SipEvent::MakeCall(target.to_string())).await;
        }
    }
    
    /// Simulate receiving an incoming call
    async fn simulate_incoming_call(&self, from: &str) {
        info!("Simulating incoming call from {}", from);
        
        // Generate a call ID
        let call_id = format!("{}@{}", Uuid::new_v4().to_string(), self.local_ip);
        
        if let Some(tx) = &self.tx {
            let _ = tx.send(SipEvent::IncomingCall {
                call_id,
                from: from.to_string(),
                to: format!("{}@{}", self.username, self.domain),
            }).await;
        }
    }
    
    /// Main event loop for the SIP client
    async fn event_loop(
        mut rx: Receiver<SipEvent>,
        username: String,
        domain: String,
        local_ip: String,
        local_port: u16,
        active_calls: Arc<Mutex<HashMap<String, Call>>>,
    ) {
        info!("SIP client event loop started");
        
        // In a real implementation, we would have a UDP socket for SIP traffic
        // and we would process incoming messages from the network
        
        while let Some(event) = rx.recv().await {
            match event {
                SipEvent::Stop => {
                    info!("Received stop event");
                    break;
                }
                SipEvent::Register => {
                    // In a real client, we would:
                    // 1. Create a REGISTER request
                    // 2. Send it to the server
                    // 3. Handle the response (401, 200, etc.)
                    // 4. Set up retransmission timers
                    
                    let request = Self::create_register_request(
                        &username, &domain, &local_ip, local_port, 3600
                    );
                    
                    info!("Created REGISTER request:\n{}", 
                        std::str::from_utf8(&request.to_bytes()).unwrap());
                    
                    // Simulate receiving a 200 OK response
                    info!("Simulating 200 OK response for REGISTER");
                }
                SipEvent::Unregister => {
                    // In a real client, we would:
                    // 1. Create a REGISTER request with expires=0
                    // 2. Send it to the server
                    // 3. Handle the response
                    
                    let request = Self::create_register_request(
                        &username, &domain, &local_ip, local_port, 0
                    );
                    
                    info!("Created REGISTER request with expires=0:\n{}", 
                        std::str::from_utf8(&request.to_bytes()).unwrap());
                    
                    // Simulate receiving a 200 OK response
                    info!("Simulating 200 OK response for REGISTER");
                }
                SipEvent::MakeCall(target) => {
                    // In a real client, we would:
                    // 1. Create an INVITE request
                    // 2. Send it to the server
                    // 3. Handle the response
                    // 4. Set up dialog state
                    
                    let call_id = format!("{}@{}", Uuid::new_v4().to_string(), local_ip);
                    let local_tag = Uuid::new_v4().to_string().split('-').next().unwrap().to_string();
                    
                    let request = Self::create_invite_request(
                        &username, &domain, &target, &local_ip, local_port, &call_id, &local_tag
                    );
                    
                    info!("Created INVITE request for {}:\n{}", 
                        target, std::str::from_utf8(&request.to_bytes()).unwrap());
                    
                    // Create call state
                    let call = Call {
                        call_id: call_id.clone(),
                        from: format!("{}@{}", username, domain),
                        to: target.clone(),
                        local_tag,
                        remote_tag: None,
                        state: CallState::Invited,
                        created_at: Instant::now(),
                    };
                    
                    // Store the call
                    active_calls.lock().unwrap().insert(call_id.clone(), call);
                    
                    // Simulate receiving a 180 Ringing response
                    info!("Simulating 180 Ringing response");
                    
                    // Simulate receiving a 200 OK response after 2 seconds
                    let active_calls_clone = Arc::clone(&active_calls);
                    let call_id_clone = call_id.clone();
                    tokio::spawn(async move {
                        sleep(Duration::from_secs(2)).await;
                        
                        // Update call state
                        if let Some(call) = active_calls_clone.lock().unwrap().get_mut(&call_id_clone) {
                            call.state = CallState::Connected;
                            call.remote_tag = Some("remote-tag-1234".to_string());
                            info!("Call connected: {}", call_id_clone);
                        }
                        
                        // Simulate call ending after 5 seconds
                        let active_calls_clone = Arc::clone(&active_calls_clone);
                        let call_id_clone = call_id_clone.clone();
                        tokio::spawn(async move {
                            sleep(Duration::from_secs(5)).await;
                            
                            // Update call state
                            if let Some(call) = active_calls_clone.lock().unwrap().get_mut(&call_id_clone) {
                                call.state = CallState::Disconnected;
                                info!("Call ended: {}", call_id_clone);
                            }
                        });
                    });
                }
                SipEvent::EndCall(call_id) => {
                    // In a real client, we would:
                    // 1. Create a BYE request
                    // 2. Send it to the server
                    // 3. Update call state
                    
                    info!("Ending call: {}", call_id);
                    
                    // Get call information
                    let call_opt = {
                        let mut calls = active_calls.lock().unwrap();
                        calls.remove(&call_id)
                    };
                    
                    if let Some(call) = call_opt {
                        info!("Call terminated: {} -> {}", call.from, call.to);
                    } else {
                        warn!("Attempted to end unknown call: {}", call_id);
                    }
                }
                SipEvent::IncomingCall { call_id, from, to } => {
                    // In a real client, we would:
                    // 1. Create a 180 Ringing response
                    // 2. Notify the user
                    // 3. Wait for user to accept/reject
                    // 4. Send appropriate final response
                    
                    info!("Incoming call from {} to {}, call_id: {}", from, to, call_id);
                    
                    // Create call state
                    let local_tag = Uuid::new_v4().to_string().split('-').next().unwrap().to_string();
                    
                    let call = Call {
                        call_id: call_id.clone(),
                        from: from.clone(),
                        to: to.clone(),
                        local_tag,
                        remote_tag: Some("remote-tag-5678".to_string()),
                        state: CallState::Ringing,
                        created_at: Instant::now(),
                    };
                    
                    // Store the call
                    active_calls.lock().unwrap().insert(call_id.clone(), call);
                    
                    // Simulate accepting the call after 2 seconds
                    let active_calls_clone = Arc::clone(&active_calls);
                    let call_id_clone = call_id.clone();
                    tokio::spawn(async move {
                        sleep(Duration::from_secs(2)).await;
                        
                        // Update call state
                        if let Some(call) = active_calls_clone.lock().unwrap().get_mut(&call_id_clone) {
                            call.state = CallState::Connected;
                            info!("Accepted incoming call: {}", call_id_clone);
                        }
                        
                        // Simulate call ending after 5 seconds
                        let active_calls_clone = Arc::clone(&active_calls_clone);
                        let call_id_clone = call_id_clone.clone();
                        tokio::spawn(async move {
                            sleep(Duration::from_secs(5)).await;
                            
                            // Update call state
                            if let Some(call) = active_calls_clone.lock().unwrap().get_mut(&call_id_clone) {
                                call.state = CallState::Disconnected;
                                info!("Call ended: {}", call_id_clone);
                            }
                        });
                    });
                }
            }
        }
        
        info!("SIP client event loop terminated");
    }
    
    /// Create a REGISTER request
    fn create_register_request(
        username: &str,
        domain: &str,
        local_ip: &str,
        local_port: u16,
        expires: u32,
    ) -> Request {
        let register_uri = format!("sip:{}", domain).parse::<Uri>().unwrap();
        let from_uri = format!("sip:{}@{}", username, domain).parse::<Uri>().unwrap();
        let contact_uri = format!("sip:{}@{}:{}", username, local_ip, local_port).parse::<Uri>().unwrap();
        let call_id = format!("{}@{}", Uuid::new_v4().to_string().split('-').next().unwrap(), local_ip);
        let branch = format!("z9hG4bK{}", Uuid::new_v4().to_string().split('-').next().unwrap());
        let tag = Uuid::new_v4().to_string().split('-').next().unwrap().to_string();
        
        sip! {
            method: Method::Register,
            uri: register_uri.to_string(),
            headers: {
                Via: format!("SIP/2.0/UDP {}:{};branch={}", local_ip, local_port, branch),
                MaxForwards: 70,
                To: format!("<sip:{}@{}>", username, domain),
                From: format!("<sip:{}@{}>;tag={}", username, domain, tag),
                CallId: call_id,
                CSeq: format!("1 REGISTER"),
                Contact: format!("<sip:{}@{}:{}>", username, local_ip, local_port),
                Expires: expires,
                UserAgent: "RVOIP SIP Client Example/0.1",
                ContentLength: 0
            }
        }
    }
    
    /// Create an INVITE request
    fn create_invite_request(
        username: &str,
        domain: &str,
        target: &str,
        local_ip: &str,
        local_port: u16,
        call_id: &str,
        local_tag: &str,
    ) -> Request {
        let target_uri = format!("sip:{}", target).parse::<Uri>().unwrap();
        let from_uri = format!("sip:{}@{}", username, domain).parse::<Uri>().unwrap();
        let contact_uri = format!("sip:{}@{}:{}", username, local_ip, local_port).parse::<Uri>().unwrap();
        let branch = format!("z9hG4bK{}", Uuid::new_v4().to_string().split('-').next().unwrap());
        
        // Create a simple SDP body
        let sdp_body = format!(
            "v=0\r\n\
             o={} 1234567890 1234567890 IN IP4 {}\r\n\
             s=SIP Call\r\n\
             c=IN IP4 {}\r\n\
             t=0 0\r\n\
             m=audio 49170 RTP/AVP 0 8\r\n\
             a=rtpmap:0 PCMU/8000\r\n\
             a=rtpmap:8 PCMA/8000\r\n",
            username, local_ip, local_ip
        );
        
        sip! {
            method: Method::Invite,
            uri: target_uri.to_string(),
            headers: {
                Via: format!("SIP/2.0/UDP {}:{};branch={}", local_ip, local_port, branch),
                MaxForwards: 70,
                To: format!("<{}>", target_uri),
                From: format!("<{}>;tag={}", from_uri, local_tag),
                CallId: call_id,
                CSeq: "1 INVITE",
                Contact: format!("<{}>", contact_uri),
                ContentType: "application/sdp",
                ContentLength: sdp_body.len(),
                UserAgent: "RVOIP SIP Client Example/0.1"
            },
            body: sdp_body.into_bytes()
        }
    }
}

/// Implementation of Display for CallState
impl std::fmt::Display for CallState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallState::Idle => write!(f, "Idle"),
            CallState::Invited => write!(f, "Invited"),
            CallState::Ringing => write!(f, "Ringing"),
            CallState::Connecting => write!(f, "Connecting"),
            CallState::Connected => write!(f, "Connected"),
            CallState::Disconnecting => write!(f, "Disconnecting"),
            CallState::Disconnected => write!(f, "Disconnected"),
        }
    }
} 