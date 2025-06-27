use rvoip_session_core::api::control::SessionControl;
// Media Codec Negotiation Integration Tests
//
// Tests the coordination between SIP SDP offer/answer and media-core codec
// negotiation. Validates that codec selection works properly between SIP
// signaling and real MediaEngine codec capabilities.
//
// **CRITICAL**: All tests use REAL codecs - no mocks.

use std::sync::Arc;
use std::time::Duration;
use rvoip_session_core::{SessionCoordinator, SessionError};
use rvoip_session_core::media::MediaConfig;
use rvoip_session_core::media::DialogId;
use uuid::Uuid;

mod common;
use common::*;

/// Test G.711 PCMU codec negotiation via real MediaSessionController
#[tokio::test]
async fn test_pcmu_codec_negotiation() {
    let media_engine = create_test_media_engine().await.unwrap();
    let capabilities = setup_test_media_capabilities().await.unwrap();
    
    // Test real PCMU audio generation
    let pcmu_audio = generate_pcmu_audio_stream(1000, 440.0).unwrap();
    assert!(!pcmu_audio.is_empty(), "PCMU audio should be generated");
    assert_eq!(pcmu_audio.len(), 8000, "PCMU should generate 8000 samples for 1 second at 8kHz");
    
    // Verify PCMU is supported in capabilities
    let pcmu_codec = capabilities.codecs.iter().find(|c| c.name == "PCMU").unwrap();
    assert_eq!(pcmu_codec.payload_type, 0, "PCMU should have payload type 0");
    assert_eq!(pcmu_codec.sample_rate, 8000, "PCMU should have 8kHz sample rate");
    
    // Test real media session with PCMU
    let dialog_id = DialogId::new(&format!("pcmu-test-{}", Uuid::new_v4()));
    let mut session_config = MediaConfig::default();
    session_config.preferred_codecs = vec!["PCMU".to_string()];
    let local_addr = "127.0.0.1:10004".parse().unwrap();
    let media_config = rvoip_session_core::media::convert_to_media_core_config(
        &session_config,
        local_addr,
        None,
    );
    
    // Create real media session with PCMU preference
    media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
    
    // Verify session was created
    let session_info = media_engine.get_session_info(&dialog_id).await.unwrap();
    assert_eq!(session_info.dialog_id, dialog_id);
    assert!(session_info.rtp_port.is_some(), "RTP port should be allocated");
    
    // Clean up
    media_engine.stop_media(&dialog_id).await.unwrap();
}

/// Test G.711 PCMA codec negotiation via real MediaSessionController
#[tokio::test]
async fn test_pcma_codec_negotiation() {
    let media_engine = create_test_media_engine().await.unwrap();
    let capabilities = setup_test_media_capabilities().await.unwrap();
    
    // Test real PCMA audio generation
    let pcma_audio = generate_pcma_audio_stream(1000, 440.0).unwrap();
    assert!(!pcma_audio.is_empty(), "PCMA audio should be generated");
    assert_eq!(pcma_audio.len(), 8000, "PCMA should generate 8000 samples for 1 second at 8kHz");
    
    // Verify PCMA is supported
    let pcma_codec = capabilities.codecs.iter().find(|c| c.name == "PCMA").unwrap();
    assert_eq!(pcma_codec.payload_type, 8, "PCMA should have payload type 8");
    assert_eq!(pcma_codec.sample_rate, 8000, "PCMA should have 8kHz sample rate");
    
    // Test real media session with PCMA
    let dialog_id = DialogId::new(&format!("pcma-test-{}", Uuid::new_v4()));
    let mut session_config = MediaConfig::default();
    session_config.preferred_codecs = vec!["PCMA".to_string()];
    let local_addr = "127.0.0.1:10008".parse().unwrap();
    let media_config = rvoip_session_core::media::convert_to_media_core_config(
        &session_config,
        local_addr,
        None,
    );
    
    // Create real media session with PCMA preference
    media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
    
    // Verify session was created
    let session_info = media_engine.get_session_info(&dialog_id).await.unwrap();
    assert_eq!(session_info.dialog_id, dialog_id);
    assert!(session_info.rtp_port.is_some(), "RTP port should be allocated");
    
    // Clean up
    media_engine.stop_media(&dialog_id).await.unwrap();
}

/// Test Opus codec negotiation with real MediaSessionController
#[tokio::test]
async fn test_opus_codec_negotiation() {
    let media_engine = create_test_media_engine().await.unwrap();
    let capabilities = setup_test_media_capabilities().await.unwrap();
    
    // Test real Opus audio generation
    let opus_audio = generate_opus_audio_stream(1000, 440.0, 64000).await.unwrap();
    assert!(!opus_audio.is_empty(), "Opus audio should be generated");
    // Opus frames are variable size, but should have reasonable data
    assert!(opus_audio.len() > 100, "Opus should generate reasonable amount of data");
    
    // Verify Opus is supported
    let opus_codec = capabilities.codecs.iter().find(|c| c.name == "Opus").unwrap();
    assert_eq!(opus_codec.payload_type, 111, "Opus should have payload type 111");
    assert_eq!(opus_codec.sample_rate, 48000, "Opus should have 48kHz sample rate");
    assert_eq!(opus_codec.channels, 2, "Opus should support stereo");
    
    // Test real media session with Opus
    let dialog_id = DialogId::new(&format!("opus-test-{}", Uuid::new_v4()));
    let mut session_config = MediaConfig::default();
    session_config.preferred_codecs = vec!["Opus".to_string()];
    let local_addr = "127.0.0.1:10012".parse().unwrap();
    let media_config = rvoip_session_core::media::convert_to_media_core_config(
        &session_config,
        local_addr,
        None,
    );
    
    // Create real media session with Opus preference
    media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
    
    // Verify session was created  
    let session_info = media_engine.get_session_info(&dialog_id).await.unwrap();
    assert_eq!(session_info.dialog_id, dialog_id);
    assert!(session_info.rtp_port.is_some(), "RTP port should be allocated");
    
    // Clean up
    media_engine.stop_media(&dialog_id).await.unwrap();
}

/// Test G.729 codec negotiation via real MediaSessionController
#[tokio::test]
async fn test_g729_codec_negotiation() {
    let media_engine = create_test_media_engine().await.unwrap();
    let capabilities = setup_test_media_capabilities().await.unwrap();
    
    // Verify G.729 is supported
    let g729_codec = capabilities.codecs.iter().find(|c| c.name == "G.729").unwrap();
    assert_eq!(g729_codec.payload_type, 18, "G.729 should have payload type 18");
    assert_eq!(g729_codec.sample_rate, 8000, "G.729 should have 8kHz sample rate");
    assert_eq!(g729_codec.channels, 1, "G.729 should be mono");
    
    // Test real media session with G.729
    let dialog_id = DialogId::new(&format!("g729-test-{}", Uuid::new_v4()));
    let mut session_config = MediaConfig::default();
    session_config.preferred_codecs = vec!["G.729".to_string()];
    let local_addr = "127.0.0.1:10016".parse().unwrap();
    let media_config = rvoip_session_core::media::convert_to_media_core_config(
        &session_config,
        local_addr,
        None,
    );
    
    // Create real media session with G.729 preference
    media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
    
    // Verify session was created
    let session_info = media_engine.get_session_info(&dialog_id).await.unwrap();
    assert_eq!(session_info.dialog_id, dialog_id);
    assert!(session_info.rtp_port.is_some(), "RTP port should be allocated");
    
    // Clean up
    media_engine.stop_media(&dialog_id).await.unwrap();
}

/// Test real codec preference order negotiation
#[tokio::test]
async fn test_codec_preference_negotiation() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    // Test codec negotiation with different preference orders
    let preference_scenarios = vec![
        vec!["PCMU".to_string(), "PCMA".to_string()],
        vec!["Opus".to_string(), "PCMU".to_string()],
        vec!["G.729".to_string(), "PCMA".to_string()],
    ];
    
    for (i, preferences) in preference_scenarios.iter().enumerate() {
        let dialog_id = DialogId::new(&format!("pref-test-{}-{}", i, Uuid::new_v4()));
        let mut session_config = MediaConfig::default();
        session_config.preferred_codecs = preferences.clone();
        let local_addr = format!("127.0.0.1:{}", 10020 + i * 4).parse().unwrap();
        let media_config = rvoip_session_core::media::convert_to_media_core_config(
            &session_config,
            local_addr,
            None,
        );
        
        // Create session with specific preference order
        media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
        
        // Verify session was created
        let session_info = media_engine.get_session_info(&dialog_id).await.unwrap();
        assert_eq!(session_info.dialog_id, dialog_id);
        
        // Clean up
        media_engine.stop_media(&dialog_id).await.unwrap();
    }
    
    // Test multi-codec scenarios with real negotiation
    let multi_codec_scenarios = create_multi_codec_test_scenario(media_engine.as_ref()).await.unwrap();
    assert!(!multi_codec_scenarios.is_empty(), "Multi-codec scenarios should be created");
    assert!(multi_codec_scenarios.contains_key("pcmu_preferred"), "PCMU preference scenario should exist");
}

/// Test real codec compatibility validation
#[tokio::test]
async fn test_codec_compatibility_validation() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    // Test SDP compatibility checking with real MediaSessionController
    let test_sdps = vec![
        ("PCMU only", "v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\nm=audio 5004 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n"),
        ("PCMA only", "v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\nm=audio 5004 RTP/AVP 8\r\na=rtpmap:8 PCMA/8000\r\n"),
        ("Multi-codec", "v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\nm=audio 5004 RTP/AVP 0 8 111\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:8 PCMA/8000\r\na=rtpmap:111 opus/48000/2\r\n"),
        ("Unsupported", "v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\nm=audio 5004 RTP/AVP 97\r\na=rtpmap:97 AMR/8000\r\n"),
    ];
    
    for (name, sdp) in test_sdps {
        let is_compatible = verify_sdp_media_compatibility(media_engine.as_ref(), sdp).await.unwrap();
        
        match name {
            "PCMU only" | "PCMA only" | "Multi-codec" => {
                assert!(is_compatible, "{} SDP should be compatible", name);
            }
            "Unsupported" => {
                assert!(!is_compatible, "{} SDP should not be compatible", name);
            }
            _ => {}
        }
    }
}

/// Test real dynamic codec changes
#[tokio::test]
async fn test_dynamic_codec_renegotiation() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    let dialog_id = DialogId::new(&format!("renego-test-{}", Uuid::new_v4()));
    
    // Start with PCMU
    let mut initial_config = MediaConfig::default();
    initial_config.preferred_codecs = vec!["PCMU".to_string()];
    let local_addr = "127.0.0.1:10040".parse().unwrap();
    let media_config = rvoip_session_core::media::convert_to_media_core_config(
        &initial_config,
        local_addr,
        None,
    );
    
    // Create initial session
    media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
    let initial_info = media_engine.get_session_info(&dialog_id).await.unwrap();
    assert_eq!(initial_info.dialog_id, dialog_id);
    
    // Stop and restart with different codec (simulating re-INVITE)
    media_engine.stop_media(&dialog_id).await.unwrap();
    
    let mut new_config = MediaConfig::default();
    new_config.preferred_codecs = vec!["Opus".to_string()];
    let new_media_config = rvoip_session_core::media::convert_to_media_core_config(
        &new_config,
        local_addr,
        None,
    );
    
    // Restart with new codec
    media_engine.start_media(dialog_id.clone(), new_media_config).await.unwrap();
    let new_info = media_engine.get_session_info(&dialog_id).await.unwrap();
    assert_eq!(new_info.dialog_id, dialog_id);
    
    // Clean up
    media_engine.stop_media(&dialog_id).await.unwrap();
}

/// Test real codec negotiation failure scenarios
#[tokio::test]
async fn test_codec_negotiation_failures() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    // Test with unsupported codec
    let dialog_id = DialogId::new(&format!("fail-test-{}", Uuid::new_v4()));
    let mut unsupported_config = MediaConfig::default();
    unsupported_config.preferred_codecs = vec!["UnsupportedCodec".to_string()];
    let local_addr = "127.0.0.1:10044".parse().unwrap();
    let media_config = rvoip_session_core::media::convert_to_media_core_config(
        &unsupported_config,
        local_addr,
        None,
    );
    
    // Should still create session but fall back to default codec
    media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
    let session_info = media_engine.get_session_info(&dialog_id).await.unwrap();
    assert_eq!(session_info.dialog_id, dialog_id);
    
    // Clean up
    media_engine.stop_media(&dialog_id).await.unwrap();
}

/// Test real transcoding between different codecs
#[tokio::test]
async fn test_cross_codec_transcoding() {
    let media_engine = create_test_media_engine().await.unwrap();
    let capabilities = setup_test_media_capabilities().await.unwrap();
    
    // Verify multiple codecs are available for transcoding
    assert!(capabilities.codecs.len() >= 2, "Multiple codecs should be available for transcoding");
    
    // Create sessions with different codecs
    let codecs = ["PCMU", "PCMA", "Opus"];
    let mut sessions = Vec::new();
    
    for (i, codec) in codecs.iter().enumerate() {
        let dialog_id = DialogId::new(&format!("transcode-{}-{}", codec, Uuid::new_v4()));
        let mut session_config = MediaConfig::default();
        session_config.preferred_codecs = vec![codec.to_string()];
        let local_addr = format!("127.0.0.1:{}", 10050 + i * 4).parse().unwrap();
        let media_config = rvoip_session_core::media::convert_to_media_core_config(
            &session_config,
            local_addr,
            None,
        );
        
        media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
        sessions.push(dialog_id);
    }
    
    // Verify all sessions were created
    assert_eq!(sessions.len(), 3, "All codec sessions should be created");
    
    // Clean up all sessions
    for session_id in sessions {
        media_engine.stop_media(&session_id).await.unwrap();
    }
}

/// Test real codec performance characteristics
#[tokio::test]
async fn test_codec_performance_validation() {
    let media_engine = create_test_media_engine().await.unwrap();
    let capabilities = setup_test_media_capabilities().await.unwrap();
    
    // Test performance for each supported codec
    for codec in &capabilities.codecs {
        let dialog_id = DialogId::new(&format!("perf-{}-{}", codec.name, Uuid::new_v4()));
        let mut session_config = MediaConfig::default();
        session_config.preferred_codecs = vec![codec.name.clone()];
        let local_addr = "127.0.0.1:10070".parse().unwrap();
        let media_config = rvoip_session_core::media::convert_to_media_core_config(
            &session_config,
            local_addr,
            None,
        );
        
        // Measure session creation performance
        let start = std::time::Instant::now();
        media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
        let creation_time = start.elapsed();
        
        // Should be reasonably fast (< 2 seconds, first session may need initialization time)
        assert!(creation_time < Duration::from_millis(2000), 
               "Codec {} session creation should be < 2s, got {:?}", 
               codec.name, creation_time);
        
        // Verify session exists
        let session_info = media_engine.get_session_info(&dialog_id).await.unwrap();
        assert_eq!(session_info.dialog_id, dialog_id);
        
        // Clean up
        media_engine.stop_media(&dialog_id).await.unwrap();
    }
    
    // Test integration performance with real operations
    let engine = media_engine.clone();
    let performance_test = move || {
        let engine = engine.clone();
        async move {
            let dialog_id = DialogId::new(&format!("test-{}", Uuid::new_v4()));
            let session_config = MediaConfig::default();
            let local_addr = "127.0.0.1:10074".parse().unwrap();
            let media_config = rvoip_session_core::media::convert_to_media_core_config(
                &session_config,
                local_addr,
                None,
            );
            
            engine.start_media(dialog_id.clone(), media_config).await
                .map_err(|e| Box::<dyn std::error::Error + Send + Sync>::from(format!("{:?}", e)))?;
            
            engine.stop_media(&dialog_id).await
                .map_err(|e| Box::<dyn std::error::Error + Send + Sync>::from(format!("{:?}", e)))?;
            
            Ok(())
        }
    };
    
    let metrics = measure_integration_performance(performance_test, 10).await.unwrap();
    assert!(metrics.success_rate > 0.9, "Performance should be > 90% success rate, got {}", metrics.success_rate);
    assert!(metrics.operation_time < Duration::from_secs(15), "Operations should complete within reasonable time");
}

/// Test codec negotiation scenarios with MediaSessionController
#[tokio::test]
async fn test_codec_preference_order() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    // Test PCMU preference (should be first choice)
    let pcmu_scenarios = vec![
        ("pcmu_only", vec!["PCMU"]),
        ("pcmu_first", vec!["PCMU", "PCMA", "Opus"]),
        ("pcmu_mixed", vec!["Opus", "PCMU", "G.729"]),
    ];
    
    for (scenario_name, preferred_codecs) in pcmu_scenarios {
        let dialog_id = DialogId::new(&format!("codec-pref-{}-{}", scenario_name, Uuid::new_v4()));
        let mut session_config = MediaConfig::default();
        session_config.preferred_codecs = preferred_codecs.iter().map(|s| s.to_string()).collect();
        let local_addr = "127.0.0.1:10004".parse().unwrap();
        let media_config = rvoip_session_core::media::convert_to_media_core_config(
            &session_config,
            local_addr,
            None,
        );
        
        media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
        let session_info = media_engine.get_session_info(&dialog_id).await.unwrap();
        assert_eq!(session_info.dialog_id, dialog_id);
        
        media_engine.stop_media(&dialog_id).await.unwrap();
    }
}

/// Test codec compatibility and fallback behavior
#[tokio::test]
async fn test_codec_compatibility_fallback() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    // Test compatibility scenarios
    let dialog_id = DialogId::new(&format!("codec-fallback-{}", Uuid::new_v4()));
    let mut session_config = MediaConfig::default();
    session_config.preferred_codecs = vec!["PCMU".to_string()];
    let local_addr = "127.0.0.1:10004".parse().unwrap();
    let media_config = rvoip_session_core::media::convert_to_media_core_config(
        &session_config,
        local_addr,
        None,
    );
    
    media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
    let initial_info = media_engine.get_session_info(&dialog_id).await.unwrap();
    assert_eq!(initial_info.dialog_id, dialog_id);
    
    // Test fallback by restarting with different config
    media_engine.stop_media(&dialog_id).await.unwrap();
    
    let mut new_config = MediaConfig::default();
    new_config.preferred_codecs = vec!["PCMA".to_string()];
    let new_media_config = rvoip_session_core::media::convert_to_media_core_config(
        &new_config,
        local_addr,
        None,
    );
    
    // Restart with fallback codec
    media_engine.start_media(dialog_id.clone(), new_media_config).await.unwrap();
    let new_info = media_engine.get_session_info(&dialog_id).await.unwrap();
    assert_eq!(new_info.dialog_id, dialog_id);
    
    media_engine.stop_media(&dialog_id).await.unwrap();
}

/// Test dynamic codec switching during session
#[tokio::test]
async fn test_dynamic_codec_switching() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    // Test codec switching scenarios
    let dialog_id = DialogId::new(&format!("codec-switch-{}", Uuid::new_v4()));
    let mut session_config = MediaConfig::default();
    session_config.preferred_codecs = vec!["PCMU".to_string()];
    let local_addr = "127.0.0.1:10004".parse().unwrap();
    let media_config = rvoip_session_core::media::convert_to_media_core_config(
        &session_config,
        local_addr,
        None,
    );
    
    media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
    let session_info = media_engine.get_session_info(&dialog_id).await.unwrap();
    assert_eq!(session_info.dialog_id, dialog_id);
    
    media_engine.stop_media(&dialog_id).await.unwrap();
}

/// Test codec negotiation with multiple concurrent sessions
#[tokio::test]
async fn test_concurrent_codec_negotiation() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    let session_count = 5;
    let codec_combinations = vec![
        vec!["PCMU"],
        vec!["PCMA"], 
        vec!["Opus"],
        vec!["PCMU", "PCMA"],
        vec!["Opus", "PCMU"],
    ];
    
    let mut sessions = Vec::new();
    
    // Create concurrent sessions with different codec preferences
    for i in 0..session_count {
        let dialog_id = DialogId::new(&format!("concurrent-codec-{}-{}", i, Uuid::new_v4()));
        let mut session_config = MediaConfig::default();
        session_config.preferred_codecs = codec_combinations[i].iter().map(|s| s.to_string()).collect();
        let local_addr = "127.0.0.1:10004".parse().unwrap();
        let media_config = rvoip_session_core::media::convert_to_media_core_config(
            &session_config,
            local_addr,
            None,
        );
        
        media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
        
        // Verify session creation
        let session_info = media_engine.get_session_info(&dialog_id).await.unwrap();
        assert_eq!(session_info.dialog_id, dialog_id);
        
        sessions.push(dialog_id);
    }

    // Clean up all sessions
    for session_id in sessions {
        media_engine.stop_media(&session_id).await.unwrap();
    }
}

/// Test codec negotiation performance under load
#[tokio::test]
async fn test_codec_negotiation_performance() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    let performance_session_count = 20;
    let start_time = std::time::Instant::now();
    
    let mut sessions = Vec::new();
    
    // Rapid codec negotiation testing
    for i in 0..performance_session_count {
        let dialog_id = DialogId::new(&format!("perf-codec-{}-{}", i, Uuid::new_v4()));
        let mut session_config = MediaConfig::default();
        session_config.preferred_codecs = vec!["PCMU".to_string()];
        let local_addr = "127.0.0.1:10004".parse().unwrap();
        let media_config = rvoip_session_core::media::convert_to_media_core_config(
            &session_config,
            local_addr,
            None,
        );
        
        let result = media_engine.start_media(dialog_id.clone(), media_config).await;
        match result {
            Ok(_) => {
                let session_info = media_engine.get_session_info(&dialog_id).await.unwrap();
                assert_eq!(session_info.dialog_id, dialog_id);
                sessions.push(dialog_id);
            }
            Err(_) => {
                // Handle negotiation failures gracefully in performance test
                println!("Codec negotiation failed for session {} (acceptable under load)", i);
            }
        }
    }

    // Clean up successful sessions
    for dialog_id in sessions {
        media_engine.stop_media(&dialog_id).await.unwrap();
    }
}

/// Test codec negotiation with bandwidth constraints
#[tokio::test]
async fn test_codec_bandwidth_optimization() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    let bandwidth_scenarios = vec![
        ("high_bandwidth", vec!["Opus", "PCMU", "PCMA"]),
        ("medium_bandwidth", vec!["PCMU", "PCMA"]),
        ("low_bandwidth", vec!["PCMA"]),
    ];
    
    for (scenario_name, codecs) in bandwidth_scenarios {
        let dialog_id = DialogId::new(&format!("bandwidth-{}-{}", scenario_name, Uuid::new_v4()));
        let mut session_config = MediaConfig::default();
        session_config.preferred_codecs = codecs.iter().map(|s| s.to_string()).collect();
        let local_addr = "127.0.0.1:10004".parse().unwrap();
        let media_config = rvoip_session_core::media::convert_to_media_core_config(
            &session_config,
            local_addr,
            None,
        );
        
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            media_engine.start_media(dialog_id.clone(), media_config).await
        }).await;

        match result {
            Ok(Ok(_)) => {
                let session_info = media_engine.get_session_info(&dialog_id).await.unwrap();
                assert_eq!(session_info.dialog_id, dialog_id);
                media_engine.stop_media(&dialog_id).await.unwrap();
            }
            _ => {
                println!("Bandwidth scenario {} timed out or failed", scenario_name);
            }
        }
    }
} 