//! Peer-to-peer calling example using the unified API
//! 
//! This example demonstrates:
//! - UAC (caller) making an outbound call
//! - UAS (callee) receiving and accepting the call
//! - Bidirectional audio flow
//! - Clean call termination

use rvoip_session_core_v2::api::unified::{UnifiedSession, SessionCoordinator, Config, SessionEvent};
use rvoip_session_core_v2::state_table::types::{Role, CallState};
use std::sync::Arc;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    // Create coordinator with configuration
    let config = Config {
        sip_port: 5060,
        media_ports: (10000, 20000),
        bind_addr: "127.0.0.1:5060".parse()?,
    };
    let coordinator = SessionCoordinator::new(config).await?;
    
    // Example 1: Simple UAC (Outbound Call)
    simple_uac_example(coordinator.clone()).await?;
    
    // Example 2: Simple UAS (Inbound Call)
    simple_uas_example(coordinator.clone()).await?;
    
    // Example 3: Full peer-to-peer call flow
    full_p2p_example(coordinator.clone()).await?;
    
    Ok(())
}

/// Example 1: UAC making an outbound call
async fn simple_uac_example(coordinator: Arc<SessionCoordinator>) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Example 1: UAC (Outbound Call) ===");
    
    // Create a UAC session
    let uac = UnifiedSession::new(coordinator.clone(), Role::UAC).await?;
    println!("Created UAC session: {}", uac.id);
    
    // Subscribe to events
    uac.on_event(|event| {
        match event {
            SessionEvent::StateChanged { from, to } => {
                println!("UAC state changed: {:?} -> {:?}", from, to);
            }
            SessionEvent::CallEstablished => {
                println!("UAC: Call established!");
            }
            SessionEvent::MediaFlowEstablished { local_addr, remote_addr } => {
                println!("UAC: Media flow established - Local: {}, Remote: {}", local_addr, remote_addr);
            }
            SessionEvent::CallTerminated { reason } => {
                println!("UAC: Call terminated - {}", reason);
            }
            _ => {}
        }
    }).await?;
    
    // Make a call
    println!("UAC: Making call to bob@example.com");
    uac.make_call("sip:bob@example.com").await?;
    
    // Wait for call to be established (in real scenario, would wait for answer)
    sleep(Duration::from_secs(2)).await;
    
    // Check state
    let state = uac.state().await?;
    println!("UAC current state: {:?}", state);
    
    // Send DTMF
    println!("UAC: Sending DTMF digits");
    uac.send_dtmf("1234").await?;
    
    // Hold the call
    println!("UAC: Putting call on hold");
    uac.hold().await?;
    sleep(Duration::from_secs(1)).await;
    
    // Resume the call
    println!("UAC: Resuming call");
    uac.resume().await?;
    sleep(Duration::from_secs(1)).await;
    
    // Hangup
    println!("UAC: Hanging up");
    uac.hangup().await?;
    
    Ok(())
}

/// Example 2: UAS receiving an inbound call
async fn simple_uas_example(coordinator: Arc<SessionCoordinator>) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Example 2: UAS (Inbound Call) ===");
    
    // Create a UAS session
    let uas = UnifiedSession::new(coordinator.clone(), Role::UAS).await?;
    println!("Created UAS session: {}", uas.id);
    
    // Subscribe to events
    uas.on_event(|event| {
        match event {
            SessionEvent::StateChanged { from, to } => {
                println!("UAS state changed: {:?} -> {:?}", from, to);
            }
            SessionEvent::CallEstablished => {
                println!("UAS: Call established!");
            }
            SessionEvent::MediaFlowEstablished { local_addr, remote_addr } => {
                println!("UAS: Media flow established - Local: {}, Remote: {}", local_addr, remote_addr);
            }
            SessionEvent::DtmfReceived { digit } => {
                println!("UAS: Received DTMF digit: {}", digit);
            }
            SessionEvent::CallTerminated { reason } => {
                println!("UAS: Call terminated - {}", reason);
            }
            _ => {}
        }
    }).await?;
    
    // Simulate incoming call
    println!("UAS: Receiving incoming call from alice@example.com");
    let sdp_offer = r#"v=0
o=alice 2890844526 2890844526 IN IP4 192.168.1.100
s=-
c=IN IP4 192.168.1.100
t=0 0
m=audio 49170 RTP/AVP 0
a=rtpmap:0 PCMU/8000"#;
    
    uas.on_incoming_call("sip:alice@example.com", Some(sdp_offer.to_string())).await?;
    
    // Accept the call
    println!("UAS: Accepting the call");
    uas.accept().await?;
    
    // Wait for media to be established
    sleep(Duration::from_secs(2)).await;
    
    // Play an audio file (announcement)
    println!("UAS: Playing welcome message");
    uas.play_audio("welcome.wav").await?;
    
    // Start recording
    println!("UAS: Starting call recording");
    uas.start_recording().await?;
    
    sleep(Duration::from_secs(3)).await;
    
    // Stop recording
    println!("UAS: Stopping recording");
    uas.stop_recording().await?;
    
    // Hangup
    println!("UAS: Hanging up");
    uas.hangup().await?;
    
    Ok(())
}

/// Example 3: Full peer-to-peer call flow with both UAC and UAS
async fn full_p2p_example(coordinator: Arc<SessionCoordinator>) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Example 3: Full P2P Call Flow ===");
    
    // Create Alice (UAC - caller)
    let alice = UnifiedSession::new(coordinator.clone(), Role::UAC).await?;
    println!("Created Alice (UAC): {}", alice.id);
    
    // Create Bob (UAS - callee)
    let bob = UnifiedSession::new(coordinator.clone(), Role::UAS).await?;
    println!("Created Bob (UAS): {}", bob.id);
    
    // Set up Alice's event handler
    let alice_events = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let alice_events_clone = alice_events.clone();
    alice.on_event(move |event| {
        let events = alice_events_clone.clone();
        tokio::spawn(async move {
            events.lock().await.push(format!("Alice: {:?}", event));
        });
    }).await?;
    
    // Set up Bob's event handler
    let bob_events = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let bob_events_clone = bob_events.clone();
    bob.on_event(move |event| {
        let events = bob_events_clone.clone();
        tokio::spawn(async move {
            events.lock().await.push(format!("Bob: {:?}", event));
        });
    }).await?;
    
    // Alice calls Bob
    println!("\n1. Alice initiates call to Bob");
    alice.make_call("sip:bob@example.com").await?;
    
    // Bob receives the call
    println!("2. Bob receives incoming call");
    let sdp_offer = generate_sdp_offer("192.168.1.100", 5004);
    bob.on_incoming_call("sip:alice@example.com", Some(sdp_offer)).await?;
    
    // Bob accepts the call
    println!("3. Bob accepts the call");
    bob.accept().await?;
    
    // Wait for media establishment
    sleep(Duration::from_secs(1)).await;
    
    // Check states
    let alice_state = alice.state().await?;
    let bob_state = bob.state().await?;
    println!("4. Call states - Alice: {:?}, Bob: {:?}", alice_state, bob_state);
    
    // Simulate conversation
    println!("5. Call in progress...");
    
    // Alice sends DTMF
    println!("6. Alice sends DTMF: 1234");
    alice.send_dtmf("1234").await?;
    
    // Bob puts Alice on hold
    println!("7. Bob puts Alice on hold");
    bob.hold().await?;
    sleep(Duration::from_secs(1)).await;
    
    // Bob resumes the call
    println!("8. Bob resumes the call");
    bob.resume().await?;
    sleep(Duration::from_secs(1)).await;
    
    // Alice hangs up
    println!("9. Alice hangs up");
    alice.hangup().await?;
    
    // Bob's session should also terminate
    sleep(Duration::from_secs(1)).await;
    
    // Print all events
    println!("\n=== Event Summary ===");
    for event in alice_events.lock().await.iter() {
        println!("{}", event);
    }
    for event in bob_events.lock().await.iter() {
        println!("{}", event);
    }
    
    Ok(())
}

/// Helper function to generate a simple SDP offer
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
    async fn test_uac_session_creation() {
        let config = Config::default();
        let coordinator = SessionCoordinator::new(config).await.unwrap();
        
        let uac = UnifiedSession::new(coordinator, Role::UAC).await.unwrap();
        assert_eq!(uac.role(), Role::UAC);
        assert_eq!(uac.state().await.unwrap(), CallState::Idle);
    }
    
    #[tokio::test]
    async fn test_uas_session_creation() {
        let config = Config::default();
        let coordinator = SessionCoordinator::new(config).await.unwrap();
        
        let uas = UnifiedSession::new(coordinator, Role::UAS).await.unwrap();
        assert_eq!(uas.role(), Role::UAS);
        assert_eq!(uas.state().await.unwrap(), CallState::Idle);
    }
    
    #[tokio::test]
    async fn test_p2p_call_flow() {
        let config = Config::default();
        let coordinator = SessionCoordinator::new(config).await.unwrap();
        
        // Create UAC and UAS
        let uac = UnifiedSession::new(coordinator.clone(), Role::UAC).await.unwrap();
        let uas = UnifiedSession::new(coordinator.clone(), Role::UAS).await.unwrap();
        
        // UAC makes call
        assert!(uac.make_call("sip:test@example.com").await.is_ok());
        
        // UAS receives call
        assert!(uas.on_incoming_call("sip:caller@example.com", None).await.is_ok());
        
        // UAS accepts
        assert!(uas.accept().await.is_ok());
        
        // Both can hang up
        assert!(uac.hangup().await.is_ok());
        assert!(uas.hangup().await.is_ok());
    }
}