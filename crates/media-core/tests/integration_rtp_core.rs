//! Integration tests for media-core ↔ rtp-core
//!
//! These tests verify that media-core correctly integrates with rtp-core
//! for RTP transport, packet handling, and codec compatibility.

use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{timeout, Duration};
use std::net::SocketAddr;

// Import rtp-core types
use rvoip_rtp_core::{
    MediaTransportClient, ClientFactory, ClientConfig, ClientConfigBuilder,
    MediaFrame, MediaFrameType, MediaTransportEvent,
    RtpPacket, PayloadType
};

// Import media-core types  
use rvoip_media_core::{
    MediaEngine, MediaEngineConfig, MediaSessionParams,
    MediaSessionId, DialogId, AudioFrame, SampleRate,
    prelude::{G711Codec, G711Variant, G711Config, Transcoder},
    integration::{RtpBridge, RtpBridgeConfig, events::RtpParameters},
    processing::format::FormatConverter,
    codec::AudioCodec,
};

/// Test helper to create a configured media engine
async fn create_test_media_engine() -> Arc<MediaEngine> {
    let config = MediaEngineConfig::default();
    let engine = MediaEngine::new(config).await.expect("Failed to create MediaEngine");
    engine.start().await.expect("Failed to start MediaEngine");
    engine
}

/// Test helper to create RTP transport client
async fn create_test_rtp_client() -> Box<dyn MediaTransportClient> {
    let remote_addr: SocketAddr = "127.0.0.1:5006".parse().unwrap();
    
    let config = ClientConfigBuilder::sip()
        .remote_address(remote_addr)
        .default_payload_type(0)
        .clock_rate(8000)
        .jitter_buffer_size(50)
        .ssrc(12345)
        .build();
    
    Box::new(ClientFactory::create_client(config).await
        .expect("Failed to create RTP client"))
}

#[tokio::test]
async fn test_basic_rtp_transport_integration() {
    // Setup: Create media engine and RTP client
    let media_engine = create_test_media_engine().await;
    let rtp_client = create_test_rtp_client().await;
    
    // Test: Create media session
    let dialog_id = DialogId::new("test-dialog-rtp-001");
    let params = MediaSessionParams::audio_only()
        .with_preferred_codec(0); // PCMU
        
    let session_handle = media_engine.create_media_session(dialog_id.clone(), params).await
        .expect("Failed to create media session");
    
    // Test: Set up RTP bridge integration
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let rtp_bridge = RtpBridge::new(RtpBridgeConfig::default(), event_tx);
    
    // Test: Register RTP session
    let session_id = MediaSessionId::new("rtp-session-001");
    let rtp_params = RtpParameters {
        local_port: 5004,
        remote_address: "127.0.0.1".to_string(),
        remote_port: 5006,
        payload_type: 0, // PCMU
        ssrc: 12345,
    };
    
    rtp_bridge.register_session(session_id.clone(), rtp_params).await
        .expect("Failed to register RTP session");
    
    // Test: Verify event was sent
    let event = timeout(Duration::from_millis(100), event_rx.recv()).await
        .expect("Timeout waiting for event")
        .expect("Event channel closed");
        
    println!("✅ RTP session registration event: {:?}", event);
    
    // Cleanup
    rtp_bridge.unregister_session(&session_id).await
        .expect("Failed to unregister RTP session");
    media_engine.destroy_media_session(dialog_id).await
        .expect("Failed to destroy media session");
}

#[tokio::test]  
async fn test_codec_compatibility_with_rtp() {
    // Test that our codec implementations work with rtp-core payload formats
    
    // Test G.711 PCMU compatibility
    let mut g711_codec = G711Codec::new(
        SampleRate::Rate8000,
        1,
        G711Config {
            variant: G711Variant::MuLaw,
            sample_rate: 8000,
            channels: 1,
            frame_size_ms: 10.0,
        }
    ).expect("Failed to create G.711 codec");
    
    // Create test audio frame
    let test_samples: Vec<i16> = (0..80).map(|i| (i * 100) as i16).collect();
    let audio_frame = AudioFrame::new(test_samples, 8000, 1, 1000);
    
    // Test encoding
    let encoded = g711_codec.encode(&audio_frame)
        .expect("Failed to encode audio frame");
    assert_eq!(encoded.len(), 80, "G.711 encoded size should be 80 bytes for 10ms frame");
    
    // Test decoding  
    let decoded_frame = g711_codec.decode(&encoded)
        .expect("Failed to decode audio frame");
    assert_eq!(decoded_frame.samples.len(), 80, "Decoded frame should have 80 samples");
    assert_eq!(decoded_frame.sample_rate, 8000, "Sample rate should be preserved");
    
    println!("✅ G.711 codec compatible with RTP payload format");
    
    // TODO: Add similar tests for G.729 and Opus when rtp-core payload formats are available
}

#[tokio::test]
async fn test_media_frame_conversion() {
    // Test converting between media-core AudioFrame and rtp-core MediaFrame
    
    // Create media-core AudioFrame
    let samples: Vec<i16> = vec![100, 200, 300, 400]; 
    let audio_frame = AudioFrame::new(samples.clone(), 8000, 1, 2000);
    
    // Convert to rtp-core MediaFrame format (simulation)
    let media_frame_data = samples.iter()
        .flat_map(|&sample| sample.to_le_bytes())
        .collect::<Vec<u8>>();
    
    let media_frame = MediaFrame {
        frame_type: MediaFrameType::Audio,
        data: media_frame_data.into(),
        timestamp: audio_frame.timestamp,
        sequence: 1,
        marker: false,
        payload_type: 0,
        ssrc: 12345,
        csrcs: vec![],
    };
    
    // Verify conversion
    assert_eq!(media_frame.data.len(), 8, "MediaFrame should have 8 bytes for 4 samples");
    assert_eq!(media_frame.timestamp, 2000, "Timestamp should be preserved");
    
    // Convert back to AudioFrame
    let converted_samples: Vec<i16> = media_frame.data
        .chunks_exact(2)
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();
    
    let converted_frame = AudioFrame::new(converted_samples, 8000, 1, media_frame.timestamp);
    
    // Verify round-trip conversion
    assert_eq!(converted_frame.samples, audio_frame.samples, "Samples should match after conversion");
    assert_eq!(converted_frame.timestamp, audio_frame.timestamp, "Timestamp should match");
    
    println!("✅ AudioFrame ↔ MediaFrame conversion working");
}

#[tokio::test]
async fn test_rtp_bridge_packet_routing() {
    // Test that RtpBridge correctly routes packets between media-core and rtp-core
    
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let rtp_bridge = RtpBridge::new(RtpBridgeConfig::default(), event_tx);
    
    // Set up packet channels (simulation)
    let (incoming_packet_tx, incoming_packet_rx) = mpsc::unbounded_channel();
    let (outgoing_packet_tx, mut outgoing_packet_rx) = mpsc::unbounded_channel();
    
    rtp_bridge.setup_channels(incoming_packet_rx, outgoing_packet_tx).await;
    
    // Register a session
    let session_id = MediaSessionId::new("packet-routing-test");
    let rtp_params = RtpParameters {
        local_port: 5008,
        remote_address: "127.0.0.1".to_string(), 
        remote_port: 5010,
        payload_type: 0,
        ssrc: 54321,
    };
    
    rtp_bridge.register_session(session_id.clone(), rtp_params).await
        .expect("Failed to register session");
    
    // Test outgoing packet routing
    let test_encoded_data = vec![0xFF; 80]; // PCMU silence
    let test_timestamp = 8000;
    
    rtp_bridge.send_media_packet(&session_id, test_encoded_data.clone(), test_timestamp).await
        .expect("Failed to send media packet");
    
    // Verify packet was routed to outgoing channel
    let (routed_session_id, routed_data, routed_timestamp) = timeout(
        Duration::from_millis(100), 
        outgoing_packet_rx.recv()
    ).await
        .expect("Timeout waiting for outgoing packet")
        .expect("Outgoing packet channel closed");
    
    assert_eq!(routed_session_id, session_id, "Session ID should match");
    assert_eq!(routed_data, test_encoded_data, "Packet data should match");
    assert_eq!(routed_timestamp, test_timestamp, "Timestamp should match");
    
    println!("✅ RTP bridge packet routing working");
    
    // Test statistics
    let stats = rtp_bridge.get_session_stats(&session_id).await
        .expect("Failed to get session stats");
    assert_eq!(stats.packets_sent, 1, "Should have sent 1 packet");
    assert_eq!(stats.bytes_sent, test_encoded_data.len() as u64, "Byte count should match");
    
    println!("✅ RTP bridge statistics working");
}

#[tokio::test]
async fn test_transcoding_over_rtp() {
    // Test that transcoding works in the context of RTP transport
    
    let format_converter = Arc::new(RwLock::new(FormatConverter::new()));
    let mut transcoder = Transcoder::new(format_converter);
    
    // Create G.711 PCMU test data
    let pcmu_data = vec![0xFF; 80]; // 10ms of PCMU silence
    
    // Test transcoding to G.729
    let g729_data = transcoder.pcmu_to_g729(&pcmu_data).await
        .expect("Failed to transcode PCMU to G.729");
    
    assert_eq!(g729_data.len(), 10, "G.729 frame should be 10 bytes");
    
    // Test reverse transcoding
    let pcmu_result = transcoder.g729_to_pcmu(&g729_data).await
        .expect("Failed to transcode G.729 to PCMU");
    
    assert_eq!(pcmu_result.len(), 80, "PCMU frame should be 80 bytes");
    
    println!("✅ Transcoding compatible with RTP payload sizes");
    
    // Verify transcoding statistics
    let stats = transcoder.get_stats(0, 18) // PCMU to G.729
        .expect("Failed to get transcoding stats");
    assert_eq!(stats.frames_transcoded, 1, "Should have transcoded 1 frame");
    
    let reverse_stats = transcoder.get_stats(18, 0) // G.729 to PCMU  
        .expect("Failed to get reverse transcoding stats");
    assert_eq!(reverse_stats.frames_transcoded, 1, "Should have reverse transcoded 1 frame");
    
    println!("✅ Transcoding statistics working in RTP context");
}

#[tokio::test]
async fn test_integration_cleanup() {
    // Test that resources are properly cleaned up in integration scenarios
    
    let (event_tx, _event_rx) = mpsc::unbounded_channel();
    let rtp_bridge = RtpBridge::new(RtpBridgeConfig::default(), event_tx);
    
    // Create multiple sessions
    for i in 0..5 {
        let session_id = MediaSessionId::new(&format!("cleanup-test-{}", i));
        let rtp_params = RtpParameters {
            local_port: 5020 + i,
            remote_address: "127.0.0.1".to_string(),
            remote_port: 5030 + i,
            payload_type: 0,
            ssrc: 60000 + i as u32,
        };
        
        rtp_bridge.register_session(session_id, rtp_params).await
            .expect("Failed to register session");
    }
    
    // Verify sessions are active
    let active_sessions = rtp_bridge.get_active_sessions().await;
    assert_eq!(active_sessions.len(), 5, "Should have 5 active sessions");
    
    // Test cleanup
    rtp_bridge.cleanup_expired_sessions().await
        .expect("Failed to cleanup sessions");
    
    // Since sessions were just created, they shouldn't be expired yet
    let active_sessions_after = rtp_bridge.get_active_sessions().await;
    assert_eq!(active_sessions_after.len(), 5, "Sessions should still be active");
    
    println!("✅ Integration cleanup working");
} 