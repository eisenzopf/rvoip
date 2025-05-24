//! Processing Pipeline Demo
//!
//! This example demonstrates the new processing pipeline capabilities including
//! voice activity detection, format conversion, and audio processing.

use rvoip_media_core::prelude::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    println!("🎛️ Media Processing Pipeline Demo");
    println!("=================================");
    
    // Create processing configuration
    let mut processing_config = ProcessingConfig::default();
    processing_config.audio_config.enable_vad = true;
    processing_config.enable_format_conversion = true;
    processing_config.target_format.sample_rate = SampleRate::Rate16000; // Upsample to 16kHz
    processing_config.target_format.channels = 1; // Keep mono
    
    // Create processing pipeline
    println!("🏗️ Creating processing pipeline...");
    let pipeline = ProcessingPipeline::new(processing_config).await?;
    
    // Demo 1: Process synthetic audio with voice activity
    println!("\n📊 Demo 1: Voice Activity Detection");
    
    // Create synthetic audio frames (simulating speech and silence)
    let speech_frame = create_synthetic_speech_frame(8000, 1, 160); // 20ms at 8kHz
    let silence_frame = create_synthetic_silence_frame(8000, 1, 160);
    
    // Process speech frame
    println!("🗣️ Processing speech frame...");
    let speech_result = pipeline.process_capture(&speech_frame).await?;
    
    if let Some(audio_result) = &speech_result.audio_result {
        if let Some(vad_result) = &audio_result.vad_result {
            println!("   VAD Result: {} (confidence: {:.2}, energy: {:.4})", 
                     if vad_result.is_voice { "VOICE" } else { "SILENCE" },
                     vad_result.confidence, vad_result.energy_level);
        }
    }
    
    // Process silence frame
    println!("🤫 Processing silence frame...");
    let silence_result = pipeline.process_capture(&silence_frame).await?;
    
    if let Some(audio_result) = &silence_result.audio_result {
        if let Some(vad_result) = &audio_result.vad_result {
            println!("   VAD Result: {} (confidence: {:.2}, energy: {:.4})", 
                     if vad_result.is_voice { "VOICE" } else { "SILENCE" },
                     vad_result.confidence, vad_result.energy_level);
        }
    }
    
    // Demo 2: Format conversion
    println!("\n🔄 Demo 2: Format Conversion");
    
    println!("📈 Original: {}Hz, {}ch, {} samples", 
             speech_frame.sample_rate, speech_frame.channels, speech_frame.samples.len());
    println!("📈 Converted: {}Hz, {}ch, {} samples", 
             speech_result.frame.sample_rate, speech_result.frame.channels, speech_result.frame.samples.len());
    
    if speech_result.format_converted {
        println!("✅ Format conversion was applied");
    } else {
        println!("➡️ No format conversion needed");
    }
    
    // Demo 3: Performance metrics
    println!("\n⚡ Demo 3: Performance Metrics");
    
    let stats = pipeline.get_stats().await;
    println!("📊 Pipeline Statistics:");
    println!("   Frames processed: {}", stats.frames_processed);
    println!("   Audio processing ops: {}", stats.audio_processing_operations);
    println!("   Format conversions: {}", stats.format_conversions);
    println!("   Avg processing time: {:.2} μs", 
             stats.total_processing_time_us as f64 / stats.frames_processed as f64);
    
    // Demo 4: Batch processing
    println!("\n📦 Demo 4: Batch Processing");
    
    let batch_size = 10;
    println!("🔄 Processing {} frames...", batch_size);
    
    let start_time = std::time::Instant::now();
    
    for i in 0..batch_size {
        let frame = if i % 3 == 0 {
            create_synthetic_silence_frame(8000, 1, 160)
        } else {
            create_synthetic_speech_frame(8000, 1, 160)
        };
        
        let _result = pipeline.process_capture(&frame).await?;
    }
    
    let batch_time = start_time.elapsed();
    println!("✅ Batch completed in {:.2} ms", batch_time.as_millis());
    
    // Final stats
    let final_stats = pipeline.get_stats().await;
    println!("\n📈 Final Statistics:");
    println!("   Total frames processed: {}", final_stats.frames_processed);
    println!("   Voice frames detected: {}%", 
             (final_stats.audio_processing_operations as f64 / final_stats.frames_processed as f64) * 100.0);
    println!("   Format conversions: {}", final_stats.format_conversions);
    
    println!("\n✨ Processing pipeline demo completed successfully!");
    Ok(())
}

/// Create synthetic speech frame with moderate energy and varied zero-crossing rate
fn create_synthetic_speech_frame(sample_rate: u32, channels: u8, samples_per_channel: usize) -> AudioFrame {
    let total_samples = samples_per_channel * channels as usize;
    let mut samples = Vec::with_capacity(total_samples);
    
    // Generate synthetic speech-like signal
    for i in 0..samples_per_channel {
        let t = i as f32 / sample_rate as f32;
        
        // Mix of frequencies to simulate speech
        let signal = 
            (t * 2.0 * std::f32::consts::PI * 200.0).sin() * 0.3 +  // 200 Hz fundamental
            (t * 2.0 * std::f32::consts::PI * 600.0).sin() * 0.2 +  // 600 Hz harmonic
            (t * 2.0 * std::f32::consts::PI * 1000.0).sin() * 0.1;  // 1000 Hz harmonic
        
        // Add some noise for realism
        let noise = (rand::random::<f32>() - 0.5) * 0.05;
        let sample = ((signal + noise) * 8000.0) as i16;
        
        for _ in 0..channels {
            samples.push(sample);
        }
    }
    
    AudioFrame::new(samples, sample_rate, channels, 0)
}

/// Create synthetic silence frame with very low energy
fn create_synthetic_silence_frame(sample_rate: u32, channels: u8, samples_per_channel: usize) -> AudioFrame {
    let total_samples = samples_per_channel * channels as usize;
    let mut samples = Vec::with_capacity(total_samples);
    
    // Generate very low amplitude noise to simulate silence
    for _i in 0..samples_per_channel {
        let noise = (rand::random::<f32>() - 0.5) * 0.001; // Very quiet noise
        let sample = (noise * 100.0) as i16; // Very low amplitude
        
        for _ in 0..channels {
            samples.push(sample);
        }
    }
    
    AudioFrame::new(samples, sample_rate, channels, 0)
} 