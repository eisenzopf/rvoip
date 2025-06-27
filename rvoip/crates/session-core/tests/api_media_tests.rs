//! Tests for the new MediaControl API methods
//! 
//! Verifies that the new methods added for UAS support work correctly
//! and can be used as alternatives to direct internal access.

use rvoip_session_core::api::*;
use std::sync::Arc;
use std::time::Duration;

/// Mock handler for testing
#[derive(Debug)]
struct TestHandler;

#[async_trait::async_trait]
impl CallHandler for TestHandler {
    async fn on_incoming_call(&self, _call: IncomingCall) -> CallDecision {
        CallDecision::Accept(None)
    }
    
    async fn on_call_established(&self, _session: CallSession, _local_sdp: Option<String>, _remote_sdp: Option<String>) {
        // Test handler
    }
    
    async fn on_call_ended(&self, _session: CallSession, _reason: &str) {
        // Test handler
    }
}

#[tokio::test]
async fn test_create_media_session() {
    // Build a test coordinator
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(15999)  // Use high port for testing
        .with_local_address("sip:test@127.0.0.1:15999")
        .with_media_ports(30000, 31000)
        .with_handler(Arc::new(TestHandler))
        .build()
        .await
        .expect("Failed to build coordinator");
    
    // Create a session ID
    let session_id = SessionId::new();
    
    // Test creating a media session
    let result = MediaControl::create_media_session(&coordinator, &session_id).await;
    assert!(result.is_ok(), "Failed to create media session: {:?}", result);
    
    // Verify the session was created by getting media info
    let media_info = MediaControl::get_media_info(&coordinator, &session_id).await
        .expect("Failed to get media info");
    
    assert!(media_info.is_some(), "Media session should exist after creation");
    assert!(media_info.unwrap().local_rtp_port.is_some(), "Should have allocated RTP port");
}

#[tokio::test]
async fn test_update_remote_sdp() {
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(16000)
        .with_local_address("sip:test@127.0.0.1:16000")
        .with_media_ports(30000, 31000)
        .with_handler(Arc::new(TestHandler))
        .build()
        .await
        .expect("Failed to build coordinator");
    
    let session_id = SessionId::new();
    
    // First create a media session
    MediaControl::create_media_session(&coordinator, &session_id).await
        .expect("Failed to create media session");
    
    // Test updating with remote SDP
    let remote_sdp = r#"v=0
o=test 123 456 IN IP4 192.168.1.100
s=Test Session
c=IN IP4 192.168.1.100
t=0 0
m=audio 5004 RTP/AVP 0 8
a=rtpmap:0 PCMU/8000
a=rtpmap:8 PCMA/8000"#;
    
    let result = MediaControl::update_remote_sdp(&coordinator, &session_id, remote_sdp).await;
    assert!(result.is_ok(), "Failed to update remote SDP: {:?}", result);
    
    // Verify the SDP was stored
    let media_info = MediaControl::get_media_info(&coordinator, &session_id).await
        .expect("Failed to get media info")
        .expect("Media info should exist");
    
    assert!(media_info.remote_sdp.is_some(), "Remote SDP should be stored");
}

#[tokio::test]
async fn test_generate_sdp_answer() {
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(16001)
        .with_local_address("sip:test@127.0.0.1:16001")
        .with_media_ports(30000, 31000)
        .with_handler(Arc::new(TestHandler))
        .build()
        .await
        .expect("Failed to build coordinator");
    
    let session_id = SessionId::new();
    
    // Test generating SDP answer from an offer
    let offer = r#"v=0
o=caller 123 456 IN IP4 192.168.1.50
s=Call Session
c=IN IP4 192.168.1.50
t=0 0
m=audio 5060 RTP/AVP 0 8 18
a=rtpmap:0 PCMU/8000
a=rtpmap:8 PCMA/8000
a=rtpmap:18 G729/8000"#;
    
    let answer = MediaControl::generate_sdp_answer(&coordinator, &session_id, offer).await
        .expect("Failed to generate SDP answer");
    
    // Verify the answer contains required elements
    assert!(answer.contains("v=0"), "Answer should contain version");
    assert!(answer.contains("m=audio"), "Answer should contain media line");
    assert!(answer.contains("c=IN IP4"), "Answer should contain connection line");
    
    // Verify the media session was created
    let media_info = MediaControl::get_media_info(&coordinator, &session_id).await
        .expect("Failed to get media info")
        .expect("Media info should exist");
    
    assert!(media_info.local_sdp.is_some(), "Local SDP should be stored");
    assert!(media_info.remote_sdp.is_some(), "Remote SDP should be stored");
    assert!(media_info.local_rtp_port.is_some(), "RTP port should be allocated");
}

#[tokio::test]
async fn test_sdp_parsing_utilities() {
    // Test the parse_sdp_connection helper
    let sdp = r#"v=0
o=test 123 456 IN IP4 10.0.0.1
s=Test
c=IN IP4 10.0.0.1
t=0 0
m=audio 9999 RTP/AVP 0 8
a=rtpmap:0 PCMU/8000
a=rtpmap:8 PCMA/8000"#;
    
    let sdp_info = parse_sdp_connection(sdp)
        .expect("Failed to parse SDP");
    
    assert_eq!(sdp_info.ip, "10.0.0.1");
    assert_eq!(sdp_info.port, 9999);
    assert!(sdp_info.codecs.contains(&"0".to_string()));
    assert!(sdp_info.codecs.contains(&"8".to_string()));
    assert!(sdp_info.codecs.contains(&"PCMU".to_string()));
    assert!(sdp_info.codecs.contains(&"PCMA".to_string()));
}

#[tokio::test]
async fn test_api_workflow_for_uas() {
    // This test demonstrates the complete UAS workflow using only the public API
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(16002)
        .with_local_address("sip:uas@127.0.0.1:16002")
        .with_media_ports(30000, 31000)
        .with_handler(Arc::new(TestHandler))
        .build()
        .await
        .expect("Failed to build coordinator");
    
    // Simulate incoming call with SDP offer
    let session_id = SessionId::new();
    let offer = r#"v=0
o=uac 987 654 IN IP4 172.16.0.100
s=UAC Call
c=IN IP4 172.16.0.100
t=0 0
m=audio 7070 RTP/AVP 0
a=rtpmap:0 PCMU/8000"#;
    
    // UAS generates answer (this would happen in on_incoming_call)
    let answer = MediaControl::generate_sdp_answer(&coordinator, &session_id, offer).await
        .expect("Failed to generate answer");
    
    // Verify answer is valid
    assert!(answer.contains("m=audio"), "Answer should contain media");
    
    // Parse the offer to get remote endpoint
    let remote_info = parse_sdp_connection(offer)
        .expect("Failed to parse offer");
    
    // Establish media flow (this would happen in on_call_established)
    let remote_addr = format!("{}:{}", remote_info.ip, remote_info.port);
    MediaControl::establish_media_flow(&coordinator, &session_id, &remote_addr).await
        .expect("Failed to establish media flow");
    
    // Verify everything is set up correctly
    let media_info = MediaControl::get_media_info(&coordinator, &session_id).await
        .expect("Failed to get media info")
        .expect("Media should be established");
    
    assert!(media_info.local_rtp_port.is_some());
    assert!(media_info.remote_rtp_port.is_some());
    assert_eq!(media_info.remote_rtp_port, Some(7070));
}

#[tokio::test]
async fn test_no_internal_access_needed() {
    // This test proves that UAS servers no longer need to access coordinator.media_manager
    // Everything can be done through the public API
    
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(16003)
        .with_local_address("sip:test@127.0.0.1:16003")
        .with_media_ports(30000, 31000)
        .with_handler(Arc::new(TestHandler))
        .build()
        .await
        .expect("Failed to build coordinator");
    
    // All these operations that previously required internal access now work through the API:
    
    let session_id = SessionId::new();
    
    // 1. Create media session (replaces coordinator.media_manager.create_media_session)
    MediaControl::create_media_session(&coordinator, &session_id).await
        .expect("API method should work");
    
    // 2. Update with SDP (replaces coordinator.media_manager.update_media_session)
    MediaControl::update_remote_sdp(&coordinator, &session_id, "v=0\r\nc=IN IP4 1.2.3.4\r\nm=audio 5000 RTP/AVP 0").await
        .expect("API method should work");
    
    // 3. Generate SDP (replaces coordinator.generate_sdp_offer)
    MediaControl::generate_sdp_offer(&coordinator, &session_id).await
        .expect("API method should work");
    
    // Success! No internal access needed
} 