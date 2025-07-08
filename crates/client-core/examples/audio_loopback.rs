//! Audio Loopback Example
//!
//! This example demonstrates real-time audio capture and playback by creating
//! a loopback from the default microphone to the default speakers.
//!
//! **WARNING**: This may cause feedback if your speakers and microphone are close!
//! Use headphones or keep the volume low.
//!
//! Run with: cargo run --example audio_loopback

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::timeout;
use rvoip_client_core::audio::{
    AudioDeviceManager, AudioDirection, AudioFormat, AudioDevice,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for better output
    tracing_subscriber::fmt::init();
    
    println!("🎵 Audio Loopback Example");
    println!("=========================");
    println!("⚠️  WARNING: This may cause feedback! Use headphones or keep volume low.");
    println!("📝 This example captures audio from your microphone and plays it back through speakers.\n");
    
    // Create audio device manager
    let manager = AudioDeviceManager::new().await?;
    
    // Get default input and output devices
    println!("🔍 Getting default audio devices...");
    let input_device = manager.get_default_device(AudioDirection::Input).await?;
    let output_device = manager.get_default_device(AudioDirection::Output).await?;
    
    println!("🎤 Input Device: {} ({})", input_device.info().name, input_device.info().id);
    println!("🔊 Output Device: {} ({})", output_device.info().name, output_device.info().id);
    
    // Find a compatible format for both devices
    println!("\n🔧 Finding compatible audio format...");
    let format = find_compatible_format(&input_device, &output_device).await?;
    println!("✅ Using format: {}Hz, {} channels, {}ms frames", 
             format.sample_rate, format.channels, format.frame_size_ms);
    
    // Start audio capture
    println!("\n🎙️  Starting audio capture...");
    let mut audio_receiver = input_device.start_capture(format.clone()).await?;
    
    // Start audio playback
    println!("🔊 Starting audio playback...");
    let audio_sender = output_device.start_playback(format.clone()).await?;
    
    println!("\n▶️  Audio loopback is now active!");
    println!("   Press Ctrl+C to stop...");
    
    // Statistics tracking
    let start_time = Instant::now();
    let mut frames_processed = 0u64;
    let mut total_samples = 0u64;
    let mut last_stats_time = start_time;
    
    // Main audio processing loop
    loop {
        // Receive audio frame from microphone
        match timeout(Duration::from_millis(100), audio_receiver.recv()).await {
            Ok(Some(audio_frame)) => {
                // Send the frame to speakers
                if let Err(e) = audio_sender.send(audio_frame.clone()).await {
                    eprintln!("❌ Failed to send audio frame: {}", e);
                    break;
                }
                
                // Update statistics
                frames_processed += 1;
                total_samples += audio_frame.samples.len() as u64;
                
                // Print statistics every 5 seconds
                let now = Instant::now();
                if now.duration_since(last_stats_time) >= Duration::from_secs(5) {
                    let elapsed = now.duration_since(start_time);
                    let fps = frames_processed as f64 / elapsed.as_secs_f64();
                    let sample_rate = total_samples as f64 / elapsed.as_secs_f64();
                    
                    println!("📊 Stats: {:.1} frames/sec, {:.0} samples/sec ({:.1}s elapsed)", 
                             fps, sample_rate, elapsed.as_secs_f64());
                    last_stats_time = now;
                }
            }
            Ok(None) => {
                println!("📡 Audio stream ended");
                break;
            }
            Err(_) => {
                // Timeout - this is normal, just continue
                continue;
            }
        }
    }
    
    // Clean shutdown
    println!("\n🛑 Stopping audio devices...");
    input_device.stop_capture().await?;
    output_device.stop_playback().await?;
    
    // Final statistics
    let total_elapsed = start_time.elapsed();
    println!("📈 Final stats:");
    println!("   Total frames: {}", frames_processed);
    println!("   Total samples: {}", total_samples);
    println!("   Total time: {:.1}s", total_elapsed.as_secs_f64());
    println!("   Average FPS: {:.1}", frames_processed as f64 / total_elapsed.as_secs_f64());
    
    println!("\n✨ Loopback example complete!");
    Ok(())
}

/// Find a compatible audio format that both devices support
async fn find_compatible_format(
    input_device: &Arc<dyn AudioDevice>, 
    output_device: &Arc<dyn AudioDevice>
) -> Result<AudioFormat, Box<dyn std::error::Error>> {
    let test_formats = vec![
        AudioFormat::new(48000, 1, 16, 20),     // 48kHz mono
        AudioFormat::new(44100, 1, 16, 20),     // 44.1kHz mono
        AudioFormat::wideband_voip(),           // 16kHz mono
        AudioFormat::default_voip(),            // 8kHz mono
        AudioFormat::new(48000, 2, 16, 20),     // 48kHz stereo
        AudioFormat::new(44100, 2, 16, 20),     // 44.1kHz stereo
    ];
    
    for format in test_formats {
        if input_device.supports_format(&format) && output_device.supports_format(&format) {
            return Ok(format);
        }
    }
    
    // If no common format found, create one from device capabilities
    let input_info = input_device.info();
    let output_info = output_device.info();
    
    // Find common sample rate
    let common_sample_rate = input_info.supported_sample_rates.iter()
        .find(|&rate| output_info.supported_sample_rates.contains(rate))
        .copied()
        .ok_or("No common sample rate found between input and output devices")?;
    
    // Find common channel count (prefer mono for VoIP)
    let common_channels = if input_info.supported_channels.contains(&1) && 
                            output_info.supported_channels.contains(&1) {
        1
    } else {
        input_info.supported_channels.iter()
            .find(|&channels| output_info.supported_channels.contains(channels))
            .copied()
            .ok_or("No common channel count found between input and output devices")?
    };
    
    Ok(AudioFormat::new(common_sample_rate, common_channels, 16, 20))
} 