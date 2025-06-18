//! Integration tests for media operations
//! 
//! Tests media control, SDP handling, and audio operations.

use rvoip_client_core::{
    ClientBuilder, Client, ClientError,
    call::CallId,
};
use std::sync::Arc;
use std::time::Duration;

/// Test basic media operations
#[tokio::test]
async fn test_basic_media_operations() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("MediaTest/1.0")
        .build()
        .await
        .expect("Failed to build client");

    client.start().await.expect("Failed to start client");

    // Make a call to test media operations on
    let call_id = client.make_call(
        "sip:media_test@example.com".to_string(),
        "sip:remote@example.com".to_string(),
        None,
    ).await
    .expect("Failed to make call");

    // Test microphone mute/unmute
    client.set_microphone_mute(&call_id, true).await
        .expect("Failed to mute microphone");

    client.set_microphone_mute(&call_id, false).await
        .expect("Failed to unmute microphone");

    // Get media info
    let media_info = client.get_call_media_info(&call_id).await
        .expect("Failed to get media info");
    
    // Media info might be None if not established yet
    if let Some(info) = media_info {
        tracing::info!("Media info: {:?}", info);
        
        // Check basic fields
        assert!(info.local_rtp_port.is_some());
    }

    // Clean up
    client.hangup_call(&call_id).await
        .expect("Failed to hang up call");

    client.stop().await.expect("Failed to stop client");
}

/// Test SDP generation and handling
#[tokio::test]
async fn test_sdp_operations() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("SDPTest/1.0")
        .build()
        .await
        .expect("Failed to build client");

    client.start().await.expect("Failed to start client");

    let call_id = client.make_call(
        "sip:sdp_test@example.com".to_string(),
        "sip:remote@example.com".to_string(),
        None,
    ).await
    .expect("Failed to make call");

    // Generate SDP offer
    let sdp_offer = client.generate_sdp_offer(&call_id).await
        .expect("Failed to generate SDP offer");
    
    // Verify SDP contains expected elements
    assert!(sdp_offer.contains("v=0"));
    assert!(sdp_offer.contains("m=audio"));
    assert!(sdp_offer.contains("RTP/AVP"));
    
    tracing::info!("Generated SDP offer:\n{}", sdp_offer);

    // Test processing SDP answer (mock answer)
    let mock_answer = r#"v=0
o=- 0 0 IN IP4 127.0.0.1
s=-
c=IN IP4 127.0.0.1
t=0 0
m=audio 30000 RTP/AVP 0
a=rtpmap:0 PCMU/8000"#;

    client.process_sdp_answer(&call_id, mock_answer).await
        .expect("Failed to process SDP answer");

    // Clean up
    client.hangup_call(&call_id).await
        .expect("Failed to hang up call");

    client.stop().await.expect("Failed to stop client");
}

/// Test media session lifecycle
#[tokio::test]
async fn test_media_session_lifecycle() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("LifecycleTest/1.0")
        .build()
        .await
        .expect("Failed to build client");

    client.start().await.expect("Failed to start client");

    let call_id = client.make_call(
        "sip:lifecycle_test@example.com".to_string(),
        "sip:remote@example.com".to_string(),
        None,
    ).await
    .expect("Failed to make call");

    // Check media session is not active initially
    let is_active = client.is_media_session_active(&call_id).await
        .expect("Failed to check media session status");
    assert!(!is_active);

    // Start media session
    client.start_media_session(&call_id).await
        .expect("Failed to start media session");

    // Check it's now active
    let is_active = client.is_media_session_active(&call_id).await
        .expect("Failed to check media session status");
    assert!(is_active);

    // Stop media session
    client.stop_media_session(&call_id).await
        .expect("Failed to stop media session");

    // Check it's stopped
    let is_active = client.is_media_session_active(&call_id).await
        .expect("Failed to check media session status");
    assert!(!is_active);

    // Clean up
    client.hangup_call(&call_id).await
        .expect("Failed to hang up call");

    client.stop().await.expect("Failed to stop client");
}

/// Test audio transmission control
#[tokio::test]
async fn test_audio_transmission() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("AudioTest/1.0")
        .build()
        .await
        .expect("Failed to build client");

    client.start().await.expect("Failed to start client");

    let call_id = client.make_call(
        "sip:audio_test@example.com".to_string(),
        "sip:remote@example.com".to_string(),
        None,
    ).await
    .expect("Failed to make call");

    // Test audio transmission control
    client.start_audio_transmission(&call_id).await
        .expect("Failed to start audio transmission");

    // Check if transmission is active
    let is_transmitting = client.is_audio_transmission_active(&call_id).await
        .expect("Failed to check audio transmission status");
    assert!(is_transmitting);

    // Stop transmission
    client.stop_audio_transmission(&call_id).await
        .expect("Failed to stop audio transmission");

    // Check it's stopped
    let is_transmitting = client.is_audio_transmission_active(&call_id).await
        .expect("Failed to check audio transmission status");
    assert!(!is_transmitting);

    // Clean up
    client.hangup_call(&call_id).await
        .expect("Failed to hang up call");

    client.stop().await.expect("Failed to stop client");
}

/// Test establishing media flow
#[tokio::test]
async fn test_establish_media_flow() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("FlowTest/1.0")
        .build()
        .await
        .expect("Failed to build client");

    client.start().await.expect("Failed to start client");

    let call_id = client.make_call(
        "sip:flow_test@example.com".to_string(),
        "sip:remote@example.com".to_string(),
        None,
    ).await
    .expect("Failed to make call");

    // Establish media flow to a mock remote address
    let remote_addr = "127.0.0.1:30000";
    client.establish_media(&call_id, remote_addr).await
        .expect("Failed to establish media flow");

    // Verify media info is updated
    let media_info = client.get_call_media_info(&call_id).await
        .expect("Failed to get media info");
    
    if let Some(info) = media_info {
        assert_eq!(info.remote_rtp_port, Some(30000));
    }

    // Clean up
    client.hangup_call(&call_id).await
        .expect("Failed to hang up call");

    client.stop().await.expect("Failed to stop client");
}

/// Test codec enumeration
#[tokio::test]
async fn test_codec_enumeration() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("CodecTest/1.0")
        .build()
        .await
        .expect("Failed to build client");

    client.start().await.expect("Failed to start client");

    // Get supported audio codecs
    let codecs = client.get_supported_audio_codecs().await;
    
    // Should have at least PCMU (G.711)
    assert!(!codecs.is_empty());
    assert!(codecs.iter().any(|c| c.name == "PCMU"));
    
    // Verify codec properties
    for codec in &codecs {
        assert!(codec.payload_type >= 0);
        assert!(codec.clock_rate > 0);
        tracing::info!("Supported codec: {} (PT: {}, Rate: {})", 
                      codec.name, codec.payload_type, codec.clock_rate);
    }

    client.stop().await.expect("Failed to stop client");
}

/// Test media statistics (currently returns None)
#[tokio::test]
async fn test_media_statistics() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("StatsTest/1.0")
        .build()
        .await
        .expect("Failed to build client");

    client.start().await.expect("Failed to start client");

    let call_id = client.make_call(
        "sip:stats_test@example.com".to_string(),
        "sip:remote@example.com".to_string(),
        None,
    ).await
    .expect("Failed to make call");

    // Try to get RTP statistics
    let rtp_stats = client.get_rtp_statistics(&call_id).await
        .expect("Failed to get RTP statistics");
    
    // Currently returns None (see Phase 6.1 notes)
    assert!(rtp_stats.is_none());

    // Try to get media statistics
    let media_stats = client.get_media_statistics(&call_id).await
        .expect("Failed to get media statistics");
    
    // Currently returns None (see Phase 6.1 notes)
    assert!(media_stats.is_none());

    // Clean up
    client.hangup_call(&call_id).await
        .expect("Failed to hang up call");

    client.stop().await.expect("Failed to stop client");
}

/// Test error handling for media operations
#[tokio::test]
async fn test_media_error_handling() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("MediaErrorTest/1.0")
        .build()
        .await
        .expect("Failed to build client");

    client.start().await.expect("Failed to start client");

    // Try media operations on non-existent call
    let fake_call_id = CallId::new();
    
    // Should fail with CallNotFound
    let result = client.set_microphone_mute(&fake_call_id, true).await;
    assert!(matches!(result, Err(ClientError::CallNotFound { .. })));

    let result = client.get_call_media_info(&fake_call_id).await;
    assert!(matches!(result, Err(ClientError::CallNotFound { .. })));

    let result = client.establish_media(&fake_call_id, "127.0.0.1:30000").await;
    assert!(matches!(result, Err(ClientError::CallNotFound { .. })));

    client.stop().await.expect("Failed to stop client");
}

/// Test media capabilities
#[tokio::test]
async fn test_media_capabilities() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("CapabilitiesTest/1.0")
        .build()
        .await
        .expect("Failed to build client");

    client.start().await.expect("Failed to start client");

    // Get media capabilities
    let capabilities = client.get_media_capabilities().await;
    
    // Verify basic capabilities
    assert!(capabilities.audio_codecs.len() > 0);
    assert!(!capabilities.supports_dtmf);  // Not yet implemented
    assert!(!capabilities.supports_video); // Not yet implemented
    
    // Check codec details
    for codec in &capabilities.audio_codecs {
        assert!(!codec.is_empty());
    }
    
    tracing::info!("Media capabilities: {:?}", capabilities);

    client.stop().await.expect("Failed to stop client");
} 