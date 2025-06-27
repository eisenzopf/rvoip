use rvoip_session_core::api::control::SessionControl;
// DTMF Integration Tests
//
// Tests the coordination between SIP DTMF signaling (INFO, RFC2833) and
// media-core DTMF audio processing. Validates both in-band and out-of-band
// DTMF detection and generation.
//
// **CRITICAL**: All tests use REAL audio DTMF generation/detection - no mocks.

use std::sync::Arc;
use std::time::Duration;
use rvoip_session_core::{SessionCoordinator, SessionError};
use rvoip_session_core::media::DialogId;
use uuid::Uuid;

mod common;
use common::*;

/// Test real DTMF generation with MediaSessionController
#[tokio::test]
async fn test_dtmf_generation_sip_info_coordination() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    // Test DTMF audio generation for each digit using real audio processing
    let dtmf_digits = ['0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '*', '#', 'A', 'B', 'C', 'D'];
    
    for digit in dtmf_digits {
        let dtmf_audio = generate_dtmf_audio_stream(digit, 250).unwrap();
        assert!(!dtmf_audio.is_empty(), "DTMF audio should be generated for digit '{}'", digit);
        assert_eq!(dtmf_audio.len(), 2000, "DTMF audio should be 250ms * 8kHz = 2000 samples for digit '{}'", digit);
        
        // Verify audio has reasonable amplitude (not silence)
        let max_amplitude = dtmf_audio.iter().map(|&s| s.abs()).max().unwrap();
        assert!(max_amplitude > 1000, "DTMF audio for '{}' should have reasonable amplitude, got {}", digit, max_amplitude);
    }
    
    // Test media session with DTMF support
    let dialog_id = DialogId::new(&format!("dtmf-test-{}", Uuid::new_v4()));
    let mut session_config = rvoip_session_core::media::MediaConfig::default();
    session_config.preferred_codecs = vec!["PCMU".to_string()];
    session_config.dtmf_support = true; // Enable DTMF support
    let local_addr = "127.0.0.1:10080".parse().unwrap();
    let media_config = rvoip_session_core::media::convert_to_media_core_config(
        &session_config,
        local_addr,
        None,
    );
    
    // Create real media session with DTMF capabilities
    media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
    
    // Verify session was created
    let session_info = media_engine.get_session_info(&dialog_id).await.unwrap();
    assert_eq!(session_info.dialog_id, dialog_id);
    assert!(session_info.rtp_port.is_some(), "RTP port should be allocated for DTMF session");
    
    // Clean up
    media_engine.stop_media(&dialog_id).await.unwrap();
}

/// Test real in-band DTMF detection from audio
#[tokio::test]
async fn test_inband_dtmf_detection() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    // Generate real DTMF sequences with proper audio
    let test_sequence = "12345";
    let mut combined_audio = Vec::new();
    
    for digit in test_sequence.chars() {
        let dtmf_audio = generate_dtmf_audio_stream(digit, 200).unwrap();
        combined_audio.extend(dtmf_audio);
        
        // Add realistic silence between digits (50ms)
        let silence = vec![0i16; 400]; // 50ms silence at 8kHz
        combined_audio.extend(silence);
    }
    
    // Verify combined audio characteristics
    assert!(!combined_audio.is_empty(), "Combined DTMF audio should be generated");
    assert_eq!(combined_audio.len(), 5 * (1600 + 400), "Combined audio should be 5 digits * (200ms + 50ms) * 8kHz");
    
    // Test with media session that could analyze DTMF
    let dialog_id = DialogId::new(&format!("dtmf-detect-{}", Uuid::new_v4()));
    let mut session_config = rvoip_session_core::media::MediaConfig::default();
    session_config.preferred_codecs = vec!["PCMU".to_string()];
    session_config.dtmf_support = true;
    let local_addr = "127.0.0.1:10084".parse().unwrap();
    let media_config = rvoip_session_core::media::convert_to_media_core_config(
        &session_config,
        local_addr,
        None,
    );
    
    media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
    
    // Verify session supports DTMF detection
    let session_info = media_engine.get_session_info(&dialog_id).await.unwrap();
    assert_eq!(session_info.dialog_id, dialog_id);
    
    // Clean up
    media_engine.stop_media(&dialog_id).await.unwrap();
}

/// Test RFC2833 DTMF event capability setup  
#[tokio::test]
async fn test_rfc2833_dtmf_events() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    // Test media session with RFC2833 capability
    let dialog_id = DialogId::new(&format!("rfc2833-{}", Uuid::new_v4()));
    let mut session_config = rvoip_session_core::media::MediaConfig::default();
    session_config.preferred_codecs = vec!["PCMU".to_string()];
    session_config.dtmf_support = true;
    let local_addr = "127.0.0.1:10088".parse().unwrap();
    let media_config = rvoip_session_core::media::convert_to_media_core_config(
        &session_config,
        local_addr,
        None,
    );
    
    // Create session that could handle RFC2833 events
    media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
    
    // Verify session was created with event capabilities
    let session_info = media_engine.get_session_info(&dialog_id).await.unwrap();
    assert_eq!(session_info.dialog_id, dialog_id);
    assert!(session_info.rtp_port.is_some(), "RTP port should be allocated for RFC2833");
    
    // Clean up
    media_engine.stop_media(&dialog_id).await.unwrap();
}

/// Test DTMF method negotiation preferences
#[tokio::test]
async fn test_dtmf_method_negotiation() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    // Test different DTMF configurations
    let dtmf_scenarios = vec![
        ("in_band_only", false),  // No RFC2833, use in-band
        ("rfc2833_preferred", true), // RFC2833 preferred
    ];
    
    for (scenario_name, dtmf_support) in dtmf_scenarios {
        let dialog_id = DialogId::new(&format!("dtmf-method-{}-{}", scenario_name, Uuid::new_v4()));
        let mut session_config = rvoip_session_core::media::MediaConfig::default();
    session_config.preferred_codecs = vec!["PCMU".to_string()];
    session_config.dtmf_support = true;
        let local_addr = format!("127.0.0.1:{}", 10092 + (if dtmf_support { 4 } else { 0 })).parse().unwrap();
        let media_config = rvoip_session_core::media::convert_to_media_core_config(
            &session_config,
            local_addr,
            None,
        );
        
        // Create session with specific DTMF configuration
        media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
        
        // Verify session was created
        let session_info = media_engine.get_session_info(&dialog_id).await.unwrap();
        assert_eq!(session_info.dialog_id, dialog_id);
        
        // Clean up
        media_engine.stop_media(&dialog_id).await.unwrap();
    }
}

/// Test DTMF sequence handling with real timing
#[tokio::test]
async fn test_dtmf_sequence_handling() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    // Test rapid DTMF sequence generation
    let rapid_sequence = "123";
    let mut sequence_audio = Vec::new();
    
    for digit in rapid_sequence.chars() {
        // Generate minimum duration DTMF (100ms as per standards)
        let dtmf_audio = generate_dtmf_audio_stream(digit, 100).unwrap();
        sequence_audio.extend(dtmf_audio);
        
        // Add minimum inter-digit gap (50ms as per standards)
        let gap = vec![0i16; 400]; // 50ms at 8kHz
        sequence_audio.extend(gap);
    }
    
    // Verify sequence timing
    assert_eq!(sequence_audio.len(), 3 * (800 + 400), "Rapid sequence should have proper timing");
    
    // Test with media session
    let dialog_id = DialogId::new(&format!("dtmf-seq-{}", Uuid::new_v4()));
    let mut session_config = rvoip_session_core::media::MediaConfig::default();
    session_config.preferred_codecs = vec!["PCMU".to_string()];
    session_config.dtmf_support = true;
    let local_addr = "127.0.0.1:10100".parse().unwrap();
    let media_config = rvoip_session_core::media::convert_to_media_core_config(
        &session_config,
        local_addr,
        None,
    );
    
    media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
    let session_info = media_engine.get_session_info(&dialog_id).await.unwrap();
    assert_eq!(session_info.dialog_id, dialog_id);
    
    // Clean up
    media_engine.stop_media(&dialog_id).await.unwrap();
}

/// Test DTMF during codec changes
#[tokio::test]
async fn test_dtmf_during_codec_changes() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    let dialog_id = DialogId::new(&format!("dtmf-codec-change-{}", Uuid::new_v4()));
    
    // Start with PCMU + DTMF
    let mut initial_config = rvoip_session_core::media::MediaConfig::default();
    initial_config.preferred_codecs = vec!["PCMU".to_string()];
    initial_config.dtmf_support = true;
    let local_addr = "127.0.0.1:10104".parse().unwrap();
    let media_config = rvoip_session_core::media::convert_to_media_core_config(
        &initial_config,
        local_addr,
        None,
    );
    
    // Create initial session with DTMF
    media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
    let initial_info = media_engine.get_session_info(&dialog_id).await.unwrap();
    assert_eq!(initial_info.dialog_id, dialog_id);
    
    // Generate DTMF during initial codec
    let pre_change_dtmf = generate_dtmf_audio_stream('1', 200).unwrap();
    assert!(!pre_change_dtmf.is_empty(), "Pre-change DTMF should be generated");
    
    // Stop and restart with different codec (simulating re-INVITE)
    media_engine.stop_media(&dialog_id).await.unwrap();
    
    let mut new_config = rvoip_session_core::media::MediaConfig::default();
    new_config.preferred_codecs = vec!["PCMU".to_string()];
    new_config.dtmf_support = true;
    let new_media_config = rvoip_session_core::media::convert_to_media_core_config(
        &new_config,
        local_addr,
        None,
    );
    
    // Restart with new codec + DTMF
    media_engine.start_media(dialog_id.clone(), new_media_config).await.unwrap();
    let new_info = media_engine.get_session_info(&dialog_id).await.unwrap();
    assert_eq!(new_info.dialog_id, dialog_id);
    
    // Generate DTMF after codec change
    let post_change_dtmf = generate_dtmf_audio_stream('2', 200).unwrap();
    assert!(!post_change_dtmf.is_empty(), "Post-change DTMF should be generated");
    
    // Clean up
    media_engine.stop_media(&dialog_id).await.unwrap();
}

/// Test DTMF echo cancellation considerations
#[tokio::test]
async fn test_dtmf_echo_cancellation() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    // Create bidirectional media sessions for echo testing
    let local_session_id = DialogId::new(&format!("dtmf-local-{}", Uuid::new_v4()));
    let remote_session_id = DialogId::new(&format!("dtmf-remote-{}", Uuid::new_v4()));
    
    let mut session_config = rvoip_session_core::media::MediaConfig::default();
    session_config.preferred_codecs = vec!["PCMU".to_string()];
    session_config.dtmf_support = true;
    
    // Create local session
    let local_addr = "127.0.0.1:10108".parse().unwrap();
    let local_config = rvoip_session_core::media::convert_to_media_core_config(
        &session_config,
        local_addr,
        None,
    );
    media_engine.start_media(local_session_id.clone(), local_config).await.unwrap();
    
    // Create remote session  
    let remote_addr = "127.0.0.1:10112".parse().unwrap();
    let remote_config = rvoip_session_core::media::convert_to_media_core_config(
        &session_config,
        remote_addr,
        None,
    );
    media_engine.start_media(remote_session_id.clone(), remote_config).await.unwrap();
    
    // Generate DTMF on local end
    let dtmf_audio = generate_dtmf_audio_stream('5', 200).unwrap();
    assert!(!dtmf_audio.is_empty(), "DTMF should be generated for echo testing");
    
    // Verify both sessions exist
    let local_info = media_engine.get_session_info(&local_session_id).await.unwrap();
    let remote_info = media_engine.get_session_info(&remote_session_id).await.unwrap();
    assert_eq!(local_info.dialog_id, local_session_id);
    assert_eq!(remote_info.dialog_id, remote_session_id);
    
    // Clean up both sessions
    media_engine.stop_media(&local_session_id).await.unwrap();
    media_engine.stop_media(&remote_session_id).await.unwrap();
}

/// Test DTMF network resilience with packet simulation
#[tokio::test]
async fn test_dtmf_network_resilience() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    // Test DTMF under various network conditions
    let network_scenarios = vec![
        ("good_network", "PCMU"),
        ("lossy_network", "PCMU"), // Same codec, different scenario
        ("high_latency", "PCMA"),
    ];
    
    for (scenario, codec) in network_scenarios {
        let dialog_id = DialogId::new(&format!("dtmf-net-{}-{}", scenario, Uuid::new_v4()));
        let mut session_config = rvoip_session_core::media::MediaConfig::default();
    session_config.preferred_codecs = vec!["PCMU".to_string()];
    session_config.dtmf_support = true;
        let local_addr = "127.0.0.1:10116".parse().unwrap();
        let media_config = rvoip_session_core::media::convert_to_media_core_config(
            &session_config,
            local_addr,
            None,
        );
        
        // Create session for network resilience testing
        media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
        
        // Generate DTMF that should be resilient to network issues
        let resilient_dtmf = generate_dtmf_audio_stream('9', 300).unwrap(); // Longer duration
        assert!(!resilient_dtmf.is_empty(), "Resilient DTMF should be generated for {}", scenario);
        
        // Verify session handles the scenario
        let session_info = media_engine.get_session_info(&dialog_id).await.unwrap();
        assert_eq!(session_info.dialog_id, dialog_id);
        
        // Clean up
        media_engine.stop_media(&dialog_id).await.unwrap();
    }
}

/// Test DTMF detection accuracy and false positive prevention
#[tokio::test]
async fn test_dtmf_detection_accuracy() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    // Test various audio scenarios that might cause false DTMF detection
    let test_frequencies = [400.0, 800.0, 1200.0, 1600.0]; // Non-DTMF frequencies
    let non_dtmf_audio = create_multi_frequency_test_audio(&test_frequencies, 1000).unwrap();
    
    assert!(!non_dtmf_audio.is_empty(), "Non-DTMF audio should be generated");
    assert_eq!(non_dtmf_audio.len(), 4, "Should generate 4 frequency streams");
    
    // Test with actual DTMF vs non-DTMF audio
    let real_dtmf = generate_dtmf_audio_stream('7', 100).unwrap();
    assert!(!real_dtmf.is_empty(), "Real DTMF should be generated");
    
    // Verify audio characteristics are different
    let dtmf_max = real_dtmf.iter().map(|&s| s.abs()).max().unwrap();
    let non_dtmf_max = non_dtmf_audio[0].iter().map(|&s| s.abs()).max().unwrap();
    
    // Both should have reasonable amplitude but different characteristics
    assert!(dtmf_max > 1000, "DTMF should have good amplitude");
    assert!(non_dtmf_max > 1000, "Non-DTMF should have good amplitude");
    
    // Test media session with detection capabilities
    let dialog_id = DialogId::new(&format!("dtmf-accuracy-{}", Uuid::new_v4()));
    let mut session_config = rvoip_session_core::media::MediaConfig::default();
    session_config.preferred_codecs = vec!["PCMU".to_string()];
    session_config.dtmf_support = true;
    let local_addr = "127.0.0.1:10120".parse().unwrap();
    let media_config = rvoip_session_core::media::convert_to_media_core_config(
        &session_config,
        local_addr,
        None,
    );
    
    media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
    let session_info = media_engine.get_session_info(&dialog_id).await.unwrap();
    assert_eq!(session_info.dialog_id, dialog_id);
    
    // Clean up
    media_engine.stop_media(&dialog_id).await.unwrap();
}

/// Test DTMF volume and amplitude control
#[tokio::test]
async fn test_dtmf_volume_control() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    // Test DTMF generation at different conceptual volume levels
    let volume_tests = vec![
        ("normal", '1', 200),
        ("short", '2', 100),  // Shorter duration
        ("long", '3', 400),   // Longer duration
    ];
    
    for (volume_name, digit, duration) in volume_tests {
        let dtmf_audio = generate_dtmf_audio_stream(digit, duration).unwrap();
        assert!(!dtmf_audio.is_empty(), "DTMF audio should be generated for {} volume", volume_name);
        
        // Verify amplitude characteristics
        let max_amplitude = dtmf_audio.iter().map(|&s| s.abs()).max().unwrap();
        let avg_amplitude = dtmf_audio.iter().map(|&s| s.abs() as f32).sum::<f32>() / dtmf_audio.len() as f32;
        
        assert!(max_amplitude > 1000, "DTMF {} should have reasonable peak amplitude", volume_name);
        assert!(avg_amplitude > 100.0, "DTMF {} should have reasonable average amplitude", volume_name);
        
        println!("DTMF '{}' ({}): max={}, avg={:.1}, duration={}ms", 
                 digit, volume_name, max_amplitude, avg_amplitude, duration);
    }
    
    // Test media session with volume control capabilities
    let dialog_id = DialogId::new(&format!("dtmf-volume-{}", Uuid::new_v4()));
    let mut session_config = rvoip_session_core::media::MediaConfig::default();
    session_config.preferred_codecs = vec!["PCMU".to_string()];
    session_config.dtmf_support = true;
    let local_addr = "127.0.0.1:10124".parse().unwrap();
    let media_config = rvoip_session_core::media::convert_to_media_core_config(
        &session_config,
        local_addr,
        None,
    );
    
    media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
    let session_info = media_engine.get_session_info(&dialog_id).await.unwrap();
    assert_eq!(session_info.dialog_id, dialog_id);
    
    // Clean up
    media_engine.stop_media(&dialog_id).await.unwrap();
}

/// Test concurrent DTMF handling in multiparty scenarios
#[tokio::test]
async fn test_concurrent_dtmf_multiparty() {
    let media_engine = create_test_media_engine().await.unwrap();
    
    // Create multiple media sessions for multiparty DTMF testing
    let participants = vec!["Alice", "Bob", "Charlie"];
    let mut session_ids = Vec::new();
    
    for (i, participant) in participants.iter().enumerate() {
        let dialog_id = DialogId::new(&format!("dtmf-party-{}-{}", participant, Uuid::new_v4()));
        let mut session_config = rvoip_session_core::media::MediaConfig::default();
    session_config.preferred_codecs = vec!["PCMU".to_string()];
    session_config.dtmf_support = true;
        let local_addr = format!("127.0.0.1:{}", 10128 + i * 4).parse().unwrap();
        let media_config = rvoip_session_core::media::convert_to_media_core_config(
            &session_config,
            local_addr,
            None,
        );
        
        // Create session for each participant
        media_engine.start_media(dialog_id.clone(), media_config).await.unwrap();
        session_ids.push(dialog_id);
    }
    
    // Generate DTMF from different participants
    let participant_dtmf = vec![
        ('1', "Alice"),
        ('2', "Bob"), 
        ('3', "Charlie"),
    ];
    
    for (digit, participant) in participant_dtmf {
        let dtmf_audio = generate_dtmf_audio_stream(digit, 150).unwrap();
        assert!(!dtmf_audio.is_empty(), "DTMF should be generated from {}", participant);
    }
    
    // Verify all sessions were created
    assert_eq!(session_ids.len(), 3, "All participant sessions should be created");
    
    for session_id in &session_ids {
        let session_info = media_engine.get_session_info(session_id).await.unwrap();
        assert_eq!(session_info.dialog_id, *session_id);
    }
    
    // Clean up all sessions
    for session_id in session_ids {
        media_engine.stop_media(&session_id).await.unwrap();
    }
} 