//! Audio Streaming Demo
//! 
//! This example demonstrates how to use the real-time audio streaming API
//! to capture and play audio frames during a SIP call.

use rvoip_client_core::{
    ClientBuilder, ClientEvent, CallState,
    AudioFrame, AudioStreamConfig, AudioFrameSubscriber
};
use std::sync::Arc;
use tokio::time::{Duration, sleep};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    println!("🎵 Real-Time Audio Streaming Demo");
    println!("=================================");
    
    // Create client with audio streaming capabilities
    let client = ClientBuilder::new()
        .local_address("127.0.0.1:5080".parse()?)
        .user_agent("Audio Streaming Demo v1.0")
        .with_media(|m| m
            .codecs(vec!["Opus".to_string(), "PCMU".to_string()])
            .echo_cancellation(true)
            .noise_suppression(true)
            .auto_gain_control(true)
        )
        .build()
        .await?;
    
    // Start the client
    client.start().await?;
    println!("✅ Client started on {}", client.get_client_stats().await.local_sip_addr);
    
    // Make a call (in real usage, this would connect to another SIP endpoint)
    let call_id = Uuid::new_v4();
    println!("📞 Simulating call setup...");
    
    // Configure high-quality audio stream
    let config = AudioStreamConfig {
        sample_rate: 48000,
        channels: 1,
        codec: "Opus".to_string(),
        frame_size_ms: 20,
        enable_aec: true,
        enable_agc: true,
        enable_vad: true,
    };
    
    println!("🔧 Configuring audio stream:");
    println!("   Sample Rate: {}Hz", config.sample_rate);
    println!("   Channels: {}", config.channels);
    println!("   Codec: {}", config.codec);
    println!("   Frame Size: {}ms", config.frame_size_ms);
    
    // Note: In a real scenario, you would first make a call and wait for it to connect
    // For this demo, we'll show how the streaming functions would be used
    
    println!("\n🎛️  Audio Streaming Functions Available:");
    println!("   ✅ client.set_audio_stream_config(&call_id, config)");
    println!("   ✅ client.start_audio_stream(&call_id)");
    println!("   ✅ client.subscribe_to_audio_frames(&call_id)");
    println!("   ✅ client.send_audio_frame(&call_id, frame)");
    println!("   ✅ client.get_audio_stream_config(&call_id)");
    println!("   ✅ client.stop_audio_stream(&call_id)");
    
    // Demonstrate the API (these calls would work with an active call)
    demonstrate_streaming_api(&client, call_id).await?;
    
    println!("\n📖 Usage Pattern:");
    println!("1. Make a call and wait for it to connect");
    println!("2. Configure audio stream with set_audio_stream_config()");
    println!("3. Start streaming with start_audio_stream()");
    println!("4. Subscribe to incoming frames with subscribe_to_audio_frames()");
    println!("5. Send outgoing frames with send_audio_frame()");
    println!("6. Stop streaming with stop_audio_stream()");
    
    println!("\n🎯 Integration Points:");
    println!("• Microphone Input: Capture audio → create AudioFrame → send_audio_frame()");
    println!("• Speaker Output: subscribe_to_audio_frames() → receive AudioFrame → play audio");
    println!("• Audio Processing: Apply effects/filters between capture and transmission");
    println!("• Device Integration: Use with cpal, portaudio, or other audio libraries");
    
    client.stop().await?;
    println!("\n✅ Demo completed successfully!");
    
    Ok(())
}

async fn demonstrate_streaming_api(
    client: &rvoip_client_core::ClientManager, 
    call_id: Uuid
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n🔬 API Demonstration:");
    
    // This demonstrates how the streaming API would be used
    // Note: These functions require an active call in practice
    
    // 1. Audio stream configuration
    let config = AudioStreamConfig {
        sample_rate: 8000,
        channels: 1, 
        codec: "PCMU".to_string(),
        frame_size_ms: 20,
        enable_aec: true,
        enable_agc: true,
        enable_vad: true,
    };
    
    println!("   📋 Audio Stream Configuration:");
    println!("      Sample Rate: {}Hz", config.sample_rate);
    println!("      Channels: {}", config.channels);
    println!("      Codec: {}", config.codec);
    
    // 2. Audio frame creation example
    let samples = create_test_audio_frame(config.sample_rate, config.frame_size_ms);
    let timestamp = get_rtp_timestamp();
    let audio_frame = AudioFrame::new(samples, config.sample_rate, config.channels as u8, timestamp);
    
    println!("   🎵 Sample Audio Frame:");
    println!("      Samples: {}", audio_frame.samples.len());
    println!("      Sample Rate: {}Hz", audio_frame.sample_rate);
    println!("      Channels: {}", audio_frame.channels);
    println!("      Timestamp: {}", audio_frame.timestamp);
    
    // 3. Real-time processing pattern
    println!("   🔄 Real-Time Processing Pattern:");
    println!("      Microphone → Capture → AudioFrame → send_audio_frame()");
    println!("      subscribe_to_audio_frames() → AudioFrame → Speaker");
    
    Ok(())
}

fn create_test_audio_frame(sample_rate: u32, frame_size_ms: u32) -> Vec<i16> {
    let samples_per_frame = (sample_rate * frame_size_ms) / 1000;
    let frequency = 440.0; // A4 note
    let mut samples = Vec::with_capacity(samples_per_frame as usize);
    
    for i in 0..samples_per_frame {
        let t = i as f32 / sample_rate as f32;
        let sample = (2.0 * std::f32::consts::PI * frequency * t).sin();
        samples.push((sample * 32767.0) as i16);
    }
    
    samples
}

fn get_rtp_timestamp() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u32
} 