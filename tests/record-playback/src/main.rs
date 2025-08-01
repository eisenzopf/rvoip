//! Record-Playback Test for Audio Implementation
//! 
//! This test verifies that audio capture and playback are working correctly by:
//! 1. Recording audio from the microphone for a specified duration
//! 2. Playing back the recorded audio through the speakers
//! 3. Optionally saving the recording to a file

use anyhow::Result;
use clap::Parser;
use rvoip_audio_core::{
    AudioDeviceManager, AudioDirection, AudioFormat, AudioFrame,
    pipeline::AudioPipelineBuilder,
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::error;

#[derive(Parser, Debug)]
#[command(name = "record-playback")]
#[command(about = "Test audio recording and playback")]
struct Args {
    /// Duration to record in seconds
    #[arg(short, long, default_value = "5")]
    duration: u64,
    
    /// Play back the recording immediately after capture
    #[arg(short, long, default_value = "true")]
    playback: bool,
    
    /// Save recording to file (WAV format)
    #[arg(short, long)]
    save: Option<String>,
    
    /// List available audio devices
    #[arg(short, long)]
    list_devices: bool,
    
    /// Use specific input device (by name or ID)
    #[arg(short, long)]
    input_device: Option<String>,
    
    /// Use specific output device (by name or ID)
    #[arg(short, long)]
    output_device: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("record_playback=info,rvoip_audio_core=debug")
        .init();
    
    let args = Args::parse();
    
    // Create audio device manager
    let audio_manager = AudioDeviceManager::new().await?;
    
    // List devices if requested
    if args.list_devices {
        list_audio_devices(&audio_manager).await?;
        return Ok(());
    }
    
    // Run the record-playback test
    run_record_playback_test(&audio_manager, &args).await?;
    
    Ok(())
}

async fn list_audio_devices(manager: &AudioDeviceManager) -> Result<()> {
    println!("\n=== AUDIO DEVICES ===\n");
    
    // List input devices
    println!("INPUT DEVICES (Microphones):");
    let inputs = manager.list_devices(AudioDirection::Input).await?;
    for (i, device) in inputs.iter().enumerate() {
        println!("  {}. {} {}", 
            i + 1, 
            device.name,
            if device.is_default { "(DEFAULT)" } else { "" }
        );
        println!("     ID: {}", device.id);
        println!("     Sample rates: {:?}", device.supported_sample_rates);
        println!("     Channels: {:?}", device.supported_channels);
    }
    
    // List output devices
    println!("\nOUTPUT DEVICES (Speakers):");
    let outputs = manager.list_devices(AudioDirection::Output).await?;
    for (i, device) in outputs.iter().enumerate() {
        println!("  {}. {} {}", 
            i + 1, 
            device.name,
            if device.is_default { "(DEFAULT)" } else { "" }
        );
        println!("     ID: {}", device.id);
        println!("     Sample rates: {:?}", device.supported_sample_rates);
        println!("     Channels: {:?}", device.supported_channels);
    }
    
    Ok(())
}

async fn run_record_playback_test(manager: &AudioDeviceManager, args: &Args) -> Result<()> {
    // Get audio devices
    let input_device = if let Some(name) = &args.input_device {
        // Find device by name
        let devices = manager.list_devices(AudioDirection::Input).await?;
        devices.into_iter()
            .find(|d| d.name == *name || d.id == *name)
            .ok_or_else(|| anyhow::anyhow!("Input device '{}' not found", name))?
    } else {
        // Use default input
        let device = manager.get_default_device(AudioDirection::Input).await?;
        device.info().clone()
    };
    
    let output_device = if let Some(name) = &args.output_device {
        // Find device by name
        let devices = manager.list_devices(AudioDirection::Output).await?;
        devices.into_iter()
            .find(|d| d.name == *name || d.id == *name)
            .ok_or_else(|| anyhow::anyhow!("Output device '{}' not found", name))?
    } else {
        // Use default output
        let device = manager.get_default_device(AudioDirection::Output).await?;
        device.info().clone()
    };
    
    println!("\n=== RECORD-PLAYBACK TEST ===");
    println!("Recording from: {}", input_device.name);
    println!("Playing to: {}", output_device.name);
    println!("Duration: {} seconds", args.duration);
    println!();
    
    // Use native hardware format for testing
    // In production, capture would be at native rate and codec would handle conversion
    let format = if input_device.supported_sample_rates.contains(&44100) {
        AudioFormat::new(44100, 1, 16, 20)  // 44.1kHz mono
    } else if input_device.supported_sample_rates.contains(&48000) {
        AudioFormat::new(48000, 1, 16, 20)  // 48kHz mono
    } else {
        // Fallback to first supported rate
        let rate = input_device.supported_sample_rates.first()
            .copied()
            .unwrap_or(44100);
        AudioFormat::new(rate, 1, 16, 20)
    };
    
    println!("Using audio format: {}", format.description());
    
    // Storage for recorded frames
    let recorded_frames = Arc::new(Mutex::new(Vec::new()));
    let recorded_frames_capture = recorded_frames.clone();
    
    // Phase 1: Recording
    println!("🎤 RECORDING... Speak now!");
    
    // Create capture pipeline
    let input_device_obj = manager.get_device_by_id(&input_device.id, AudioDirection::Input).await?;
    let mut capture_pipeline = AudioPipelineBuilder::new()
        .input_device(input_device_obj)
        .input_format(format.clone())
        .output_format(format.clone())
        .device_manager(manager.clone())
        .build()
        .await?;
    
    // Start capture
    capture_pipeline.start().await?;
    
    // Capture frames for the specified duration
    let start_time = Instant::now();
    let mut frame_count = 0;
    
    while start_time.elapsed() < Duration::from_secs(args.duration) {
        match capture_pipeline.capture_frame().await {
            Ok(frame) => {
                frame_count += 1;
                
                // Show audio level
                if frame_count % 50 == 0 { // Every second at 20ms frames
                    let rms = frame.rms_level() / i16::MAX as f32;
                    let bar_width = (rms * 50.0) as usize;
                    let bar = "█".repeat(bar_width);
                    print!("\r🎤 Level: [{:<50}] {:.1}s", bar, start_time.elapsed().as_secs_f32());
                    use std::io::Write;
                    std::io::stdout().flush()?;
                }
                
                // Store frame
                recorded_frames_capture.lock().await.push(frame);
            }
            Err(e) => {
                error!("Failed to capture frame: {}", e);
            }
        }
    }
    
    // Stop capture
    capture_pipeline.stop().await?;
    
    let frames = recorded_frames.lock().await.clone();  // Clone to avoid holding the lock
    println!("\n✅ Recording complete! Captured {} frames ({:.1}s)", 
        frames.len(), 
        frames.len() as f32 * format.frame_size_ms as f32 / 1000.0
    );
    
    // Phase 2: Playback (if requested)
    if args.playback && !frames.is_empty() {
        println!("\n🔊 PLAYING BACK...");
        
        // Create playback pipeline
        let output_device_obj = manager.get_device_by_id(&output_device.id, AudioDirection::Output).await?;
        let mut playback_pipeline = AudioPipelineBuilder::new()
            .output_device(output_device_obj)
            .input_format(format.clone())
            .output_format(format.clone())
            .device_manager(manager.clone())
            .build()
            .await?;
        
        // Start playback
        playback_pipeline.start().await?;
        
        // Send all frames to the pipeline with progress
        let total_frames = frames.len();
        for (i, frame) in frames.iter().enumerate() {
            if let Err(e) = playback_pipeline.play_frame(frame.clone()).await {
                error!("Failed to play frame {}: {}", i, e);
            }
            
            // Show progress
            if i % 50 == 0 || i == total_frames - 1 {
                let progress = ((i + 1) as f32 / total_frames as f32) * 100.0;
                print!("\r🔊 Sending frames: {:.0}%", progress);
                use std::io::Write;
                std::io::stdout().flush()?;
            }
        }
        
        println!("\r🔊 All {} frames sent. Waiting for playback to complete...", total_frames);
        
        // Wait for all frames to finish playing
        playback_pipeline.wait_for_playback_complete().await?;
        
        // Now stop playback
        playback_pipeline.stop().await?;
        
        println!("✅ Playback complete!");
    }
    
    // Phase 3: Save to file (if requested)
    if let Some(filename) = &args.save {
        println!("\n💾 Saving to {}...", filename);
        save_recording_to_file(&frames, &format, filename).await?;
        println!("✅ Saved!");
    }
    
    println!("\n🎉 Test complete!");
    
    // Give audio threads time to clean up properly
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    Ok(())
}

async fn save_recording_to_file(frames: &[AudioFrame], format: &AudioFormat, filename: &str) -> Result<()> {
    use std::fs::File;
    use std::io::Write;
    
    // Calculate total samples
    let total_samples: usize = frames.iter().map(|f| f.samples.len()).sum();
    
    // Create WAV header
    let mut wav_data = Vec::new();
    
    // RIFF header
    wav_data.extend_from_slice(b"RIFF");
    wav_data.extend_from_slice(&((36 + total_samples * 2) as u32).to_le_bytes());
    wav_data.extend_from_slice(b"WAVE");
    
    // Format chunk
    wav_data.extend_from_slice(b"fmt ");
    wav_data.extend_from_slice(&16u32.to_le_bytes()); // Chunk size
    wav_data.extend_from_slice(&1u16.to_le_bytes()); // PCM format
    wav_data.extend_from_slice(&(format.channels as u16).to_le_bytes());
    wav_data.extend_from_slice(&format.sample_rate.to_le_bytes());
    wav_data.extend_from_slice(&(format.sample_rate * format.channels as u32 * 2).to_le_bytes()); // Byte rate
    wav_data.extend_from_slice(&(format.channels * 2).to_le_bytes()); // Block align
    wav_data.extend_from_slice(&16u16.to_le_bytes()); // Bits per sample
    
    // Data chunk
    wav_data.extend_from_slice(b"data");
    wav_data.extend_from_slice(&(total_samples * 2).to_le_bytes());
    
    // Write samples
    for frame in frames {
        for &sample in &frame.samples {
            wav_data.extend_from_slice(&sample.to_le_bytes());
        }
    }
    
    // Write to file
    let mut file = File::create(filename)?;
    file.write_all(&wav_data)?;
    
    Ok(())
}