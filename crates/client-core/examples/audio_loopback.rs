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
    device::AudioFrame,
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
    
    // Find compatible formats for input and output devices
    println!("\n🔧 Finding compatible audio formats...");
    let (input_format, output_format) = find_compatible_formats(&input_device, &output_device).await?;
    
    // Start audio capture
    println!("\n🎙️  Starting audio capture...");
    let mut audio_receiver = input_device.start_capture(input_format.clone()).await?;
    
    // Start audio playback
    println!("🔊 Starting audio playback...");
    let audio_sender = output_device.start_playback(output_format.clone()).await?;
    
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
                // Convert channel format if needed (e.g., mono mic → stereo speakers)
                let converted_frame = convert_audio_channels(audio_frame.clone(), output_format.channels);
                
                // Send the frame to speakers
                if let Err(e) = audio_sender.send(converted_frame).await {
                    eprintln!("❌ Failed to send audio frame: {}", e);
                    break;
                }
                
                // Update statistics (use original frame for stats)
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

/// Find compatible audio formats for input and output devices
/// Returns (input_format, output_format) that are compatible with each device
async fn find_compatible_formats(
    input_device: &Arc<dyn AudioDevice>, 
    output_device: &Arc<dyn AudioDevice>
) -> Result<(AudioFormat, AudioFormat), Box<dyn std::error::Error>> {
    let input_info = input_device.info();
    let output_info = output_device.info();
    
    // Find common sample rate (this MUST match for audio sync)
    let common_sample_rate = input_info.supported_sample_rates.iter()
        .find(|&rate| output_info.supported_sample_rates.contains(rate))
        .copied()
        .ok_or("No common sample rate found between input and output devices")?;
    
    // Input format: Use best supported channel count for input device
    let input_channels = if input_info.supported_channels.contains(&1) {
        1  // Prefer mono for microphones
    } else {
        *input_info.supported_channels.first()
            .ok_or("Input device has no supported channels")?
    };
    
    // Output format: Use best supported channel count for output device  
    let output_channels = if output_info.supported_channels.contains(&2) {
        2  // Prefer stereo for speakers
    } else if output_info.supported_channels.contains(&1) {
        1  // Fall back to mono
    } else {
        *output_info.supported_channels.first()
            .ok_or("Output device has no supported channels")?
    };
    
    let input_format = AudioFormat::new(common_sample_rate, input_channels, 16, 20);
    let output_format = AudioFormat::new(common_sample_rate, output_channels, 16, 20);
    
    println!("🔧 Using different formats for optimal quality:");
    println!("   Input:  {}Hz, {} channel(s) (microphone)", input_format.sample_rate, input_format.channels);
    println!("   Output: {}Hz, {} channel(s) (speakers)", output_format.sample_rate, output_format.channels);
    
    Ok((input_format, output_format))
}

/// Convert audio frame between different channel counts
fn convert_audio_channels(frame: AudioFrame, target_channels: u16) -> AudioFrame {
    
    if frame.format.channels == target_channels {
        return frame; // No conversion needed
    }
    
    let converted_samples = if frame.format.channels == 1 && target_channels == 2 {
        // Mono to Stereo: duplicate each sample to both channels
        let mut stereo_samples = Vec::with_capacity(frame.samples.len() * 2);
        for sample in &frame.samples {
            stereo_samples.push(*sample); // Left channel
            stereo_samples.push(*sample); // Right channel
        }
        stereo_samples
    } else if frame.format.channels == 2 && target_channels == 1 {
        // Stereo to Mono: average left and right channels
        let mut mono_samples = Vec::with_capacity(frame.samples.len() / 2);
        for chunk in frame.samples.chunks_exact(2) {
            let left = chunk[0] as i32;
            let right = chunk[1] as i32;
            let mono = ((left + right) / 2) as i16;
            mono_samples.push(mono);
        }
        mono_samples
    } else {
        // Unsupported conversion, just return original
        frame.samples
    };
    
    let mut converted_format = frame.format.clone();
    converted_format.channels = target_channels;
    
    AudioFrame::new(converted_samples, converted_format, frame.timestamp_ms)
} 