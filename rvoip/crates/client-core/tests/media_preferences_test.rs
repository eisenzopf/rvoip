//! Test media preferences integration between client-core and session-core

use rvoip_client_core::{ClientBuilder, ClientResult};
use std::sync::Arc;

#[tokio::test]
async fn test_media_preferences_passed_to_session_core() -> ClientResult<()> {
    // Create client with specific media preferences
    let client = ClientBuilder::new()
        .local_address("127.0.0.1:15060".parse().unwrap())
        .media_address("127.0.0.1:40000".parse().unwrap())
        .with_media(|m| m
            .codecs(vec!["opus", "G722", "PCMU"])
            .echo_cancellation(true)
            .noise_suppression(true)
            .auto_gain_control(false)
            .dtmf(true)
            .max_bandwidth_kbps(256)
            .ptime(30)
            .custom_attribute("a=sendrecv", "")
        )
        .build()
        .await?;
    
    // Start the client
    client.start().await?;
    
    // Verify media configuration was stored
    assert_eq!(client.media_config.preferred_codecs, vec!["opus", "G722", "PCMU"]);
    assert!(client.media_config.echo_cancellation);
    assert!(client.media_config.noise_suppression);
    assert!(!client.media_config.auto_gain_control);
    assert!(client.media_config.dtmf_enabled);
    assert_eq!(client.media_config.max_bandwidth_kbps, Some(256));
    assert_eq!(client.media_config.preferred_ptime, Some(30));
    assert!(client.media_config.custom_sdp_attributes.contains_key("a=sendrecv"));
    
    // The real test is that the coordinator was created successfully
    // with these preferences passed through
    assert!(client.is_running().await);
    
    // Clean up
    client.stop().await?;
    
    Ok(())
}

#[tokio::test]
async fn test_media_preset_integration() -> ClientResult<()> {
    use rvoip_client_core::client::config::MediaPreset;
    
    // Create client with a media preset
    let client = ClientBuilder::new()
        .local_address("127.0.0.1:15061".parse().unwrap())
        .media_preset(MediaPreset::VoiceOptimized)
        .build()
        .await?;
    
    // Start the client
    client.start().await?;
    
    // Verify preset was applied
    let capabilities = client.get_media_capabilities().await;
    assert!(capabilities.can_mute_microphone);
    assert!(capabilities.can_send_dtmf);
    
    // Clean up
    client.stop().await?;
    
    Ok(())
}

#[tokio::test]
async fn test_simple_codec_configuration() -> ClientResult<()> {
    // Create client with just codec preferences
    let client = ClientBuilder::new()
        .local_address("127.0.0.1:15062".parse().unwrap())
        .codecs(vec!["PCMU", "PCMA"])  // Simple codec list
        .echo_cancellation(false)  // Disable echo cancellation
        .build()
        .await?;
    
    // Start the client
    client.start().await?;
    
    // Verify configuration
    assert_eq!(client.media_config.preferred_codecs, vec!["PCMU", "PCMA"]);
    assert!(!client.media_config.echo_cancellation);
    
    // Default audio processing should still be enabled
    assert!(client.media_config.noise_suppression);
    assert!(client.media_config.auto_gain_control);
    
    // Clean up
    client.stop().await?;
    
    Ok(())
}

#[tokio::test]
async fn test_call_with_media_preferences() -> ClientResult<()> {
    // This test would verify that when making/receiving calls,
    // the media preferences are actually used in SDP generation
    // For now, just verify the client can be created with preferences
    
    let client = ClientBuilder::new()
        .local_address("127.0.0.1:15063".parse().unwrap())
        .with_media(|m| m
            .codecs(vec!["opus", "PCMU"])
            .require_srtp(false)  // Disable SRTP for this test
            .audio_processing(true)  // Enable all audio processing
        )
        .build()
        .await?;
    
    // Get supported codecs to verify they match our preferences
    let codecs = client.get_supported_audio_codecs().await;
    assert!(!codecs.is_empty());
    
    // The first codecs should be our preferred ones
    // (actual order might depend on session-core implementation)
    let codec_names: Vec<String> = codecs.iter()
        .map(|c| c.name.clone())
        .collect();
    assert!(codec_names.contains(&"opus".to_string()));
    assert!(codec_names.contains(&"PCMU".to_string()));
    
    Ok(())
} 