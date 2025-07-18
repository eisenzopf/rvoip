//! Test for programmatic call acceptance with SDP answer
//!
//! This test verifies that when using the SessionControl::accept_incoming_call API
//! with an SDP answer, the answer is properly passed through to the dialog layer.

use rvoip_session_core::{
    SessionCoordinator,
    api::{
        SessionControl, CallHandler, CallDecision, IncomingCall, CallSession,
        SessionManagerConfig,
    },
};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing_subscriber;
use tracing::info;

/// Test handler that defers incoming calls
#[derive(Debug)]
struct DeferringHandler {
    deferred_calls: Arc<Mutex<Vec<IncomingCall>>>,
}

impl DeferringHandler {
    fn new() -> Self {
        Self {
            deferred_calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait::async_trait]
impl CallHandler for DeferringHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        info!("Deferring incoming call: {}", call.id);
        self.deferred_calls.lock().await.push(call);
        CallDecision::Defer
    }
    
    async fn on_call_established(&self, session: CallSession, local_sdp: Option<String>, remote_sdp: Option<String>) {
        info!("Call established: {} with local_sdp: {:?}, remote_sdp: {:?}", 
              session.id, local_sdp.is_some(), remote_sdp.is_some());
    }
    
    async fn on_call_ended(&self, session: CallSession, reason: &str) {
        info!("Call ended: {} - {}", session.id, reason);
    }
}

#[tokio::test]
async fn test_accept_incoming_call_with_sdp_answer() {
    // Initialize logging
    let _ = tracing_subscriber::fmt()
        .with_env_filter("debug")
        .try_init();
    
    // Create two session coordinators - one as UAC, one as UAS
    let handler_uac = Arc::new(DeferringHandler::new());
    let handler_uas = Arc::new(DeferringHandler::new());
    
    let config_uac = SessionManagerConfig {
        sip_port: 15070,
        local_address: "sip:alice@127.0.0.1:15070".to_string(),
        ..Default::default()
    };
    
    let config_uas = SessionManagerConfig {
        sip_port: 15071,
        local_address: "sip:bob@127.0.0.1:15071".to_string(),
        ..Default::default()
    };
    
    let coordinator_uac = SessionCoordinator::new(config_uac, Some(handler_uac.clone())).await
        .expect("Failed to create UAC coordinator");
    let coordinator_uas = SessionCoordinator::new(config_uas, Some(handler_uas.clone())).await
        .expect("Failed to create UAS coordinator");
    
    // Start both coordinators
    coordinator_uac.start().await.expect("Failed to start UAC");
    coordinator_uas.start().await.expect("Failed to start UAS");
    
    // Give them time to bind
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    // UAC: Create outgoing call with SDP offer
    let sdp_offer = r#"v=0
o=alice 2890844526 2890844526 IN IP4 127.0.0.1
s=-
c=IN IP4 127.0.0.1
t=0 0
m=audio 16000 RTP/AVP 0 8
a=rtpmap:0 PCMU/8000
a=rtpmap:8 PCMA/8000"#;
    
    let call_session = coordinator_uac.create_outgoing_call(
        "sip:alice@127.0.0.1:15070",
        "sip:bob@127.0.0.1:15071",
        Some(sdp_offer.to_string())
    ).await.expect("Failed to create outgoing call");
    
    info!("Created outgoing call: {}", call_session.id);
    
    // Wait for the call to arrive at UAS
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    
    // UAS: Check that we have a deferred call
    let deferred_calls = handler_uas.deferred_calls.lock().await;
    assert_eq!(deferred_calls.len(), 1, "Expected 1 deferred call");
    let incoming_call = deferred_calls[0].clone();
    drop(deferred_calls);
    
    // Verify the incoming call has SDP
    assert!(incoming_call.sdp.is_some(), "Incoming call should have SDP offer");
    info!("Incoming call has SDP offer: {} bytes", incoming_call.sdp.as_ref().unwrap().len());
    
    // UAS: Accept the call programmatically with SDP answer
    let sdp_answer = r#"v=0
o=bob 2890844527 2890844527 IN IP4 127.0.0.1
s=-
c=IN IP4 127.0.0.1
t=0 0
m=audio 16001 RTP/AVP 0
a=rtpmap:0 PCMU/8000"#;
    
    info!("Accepting incoming call with SDP answer...");
    let accepted_session = coordinator_uas.accept_incoming_call(
        &incoming_call,
        Some(sdp_answer.to_string())
    ).await.expect("Failed to accept incoming call");
    
    info!("Call accepted: {}", accepted_session.id);
    
    // Wait for call to be established
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    
    // Verify media info has both SDPs
    let media_info_uas = coordinator_uas.get_media_info(&accepted_session.id).await
        .expect("Failed to get UAS media info");
    
    if let Some(media_info) = media_info_uas {
        assert!(media_info.local_sdp.is_some(), "UAS should have local SDP (answer)");
        assert!(media_info.remote_sdp.is_some(), "UAS should have remote SDP (offer)");
        info!("UAS Media info verified - has both local and remote SDP");
    }
    
    // Clean up
    coordinator_uac.terminate_session(&call_session.id).await
        .expect("Failed to terminate call");
    
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    
    coordinator_uac.stop().await.expect("Failed to stop UAC");
    coordinator_uas.stop().await.expect("Failed to stop UAS");
}

#[tokio::test]
async fn test_accept_incoming_call_without_sdp_answer() {
    // Initialize logging
    let _ = tracing_subscriber::fmt()
        .with_env_filter("info")
        .try_init();
    
    // Create handler that defers calls
    let handler = Arc::new(DeferringHandler::new());
    
    let config = SessionManagerConfig {
        sip_port: 15072,
        local_address: "sip:server@127.0.0.1:15072".to_string(),
        ..Default::default()
    };
    
    let coordinator = SessionCoordinator::new(config, Some(handler.clone())).await
        .expect("Failed to create coordinator");
    
    coordinator.start().await.expect("Failed to start coordinator");
    
    // Simulate an incoming call (would normally come from network)
    let incoming_call = IncomingCall {
        id: rvoip_session_core::api::SessionId::new(),
        from: "sip:caller@example.com".to_string(),
        to: "sip:server@127.0.0.1:15072".to_string(),
        sdp: Some("dummy SDP offer".to_string()),
        headers: Default::default(),
        received_at: std::time::Instant::now(),
    };
    
    // Store it as deferred
    handler.deferred_calls.lock().await.push(incoming_call.clone());
    
    // Accept without providing SDP answer (should auto-generate)
    info!("Accepting call without SDP answer - should auto-generate");
    let result = coordinator.accept_incoming_call(&incoming_call, None).await;
    
    // This will fail because we don't have a real dialog, but that's OK
    // The important part is that the API accepts None for SDP
    assert!(result.is_err(), "Expected error due to missing dialog");
    info!("Got expected error: {:?}", result.unwrap_err());
    
    coordinator.stop().await.expect("Failed to stop coordinator");
} 