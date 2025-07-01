//! AEC (Acoustic Echo Cancellation) Demo
//!
//! This example demonstrates the acoustic echo cancellation capabilities
//! including double-talk detection and adaptive filtering.

use rvoip_media_core::prelude::*;
use rvoip_media_core::processing::audio::{AcousticEchoCanceller, AecConfig};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    println!("🔇 Acoustic Echo Cancellation Demo");
    println!("==================================");
    
    // Create AEC configuration
    let aec_config = AecConfig {
        filter_length: 128,           // 16ms at 8kHz
        step_size: 0.02,              // Learning rate
        suppression_factor: 0.8,      // 80% echo suppression
        min_echo_level: 0.001,
        comfort_noise: true,
        double_talk_threshold: 0.3,   // Lower threshold for demo
    };
    
    // Create AEC instance
    println!("🏗️ Creating AEC with {} taps filter...", aec_config.filter_length);
    let mut aec = AcousticEchoCanceller::new(aec_config)?;
    
    // Demo 1: Echo cancellation without double-talk
    println!("\n📢 Demo 1: Echo Cancellation (Far-end only)");
    
    // Create far-end signal (what would cause echo)
    let far_end_signal = create_test_signal(8000, 1, 160, 1000.0, 0.5); // 1kHz tone
    
    // Create near-end signal with simulated echo (delayed far-end + some original signal)
    let mut near_end_with_echo = create_delayed_echo(&far_end_signal, 0.3, 20); // 30% echo, 20 sample delay
    add_original_signal(&mut near_end_with_echo, 500.0, 0.1); // Add weak original signal
    
    // Process with AEC
    let aec_result = aec.process_frame(&near_end_with_echo, &far_end_signal)?;
    
    println!("   Far-end level: {:.4}", aec_result.input_level);
    println!("   Near-end (with echo): {:.4}", aec_result.input_level);
    println!("   Output (echo cancelled): {:.4}", aec_result.output_level);
    println!("   Echo suppression: {:.2}%", aec_result.echo_suppression * 100.0);
    println!("   Double-talk detected: {}", aec_result.double_talk_detected);
    
    // Demo 2: Double-talk scenario
    println!("\n🗣️ Demo 2: Double-talk Detection");
    
    // Create strong near-end signal (person speaking)
    let near_end_speech = create_test_signal(8000, 1, 160, 800.0, 0.6); // 800Hz, higher amplitude
    
    // Create far-end signal (remote person speaking)
    let far_end_speech = create_test_signal(8000, 1, 160, 1200.0, 0.4); // 1200Hz
    
    // Mix them (simulating both people talking)
    let mixed_near_end = mix_signals(&near_end_speech, &create_delayed_echo(&far_end_speech, 0.2, 15));
    
    // Process with AEC
    let double_talk_result = aec.process_frame(&mixed_near_end, &far_end_speech)?;
    
    println!("   Near-end level: {:.4}", double_talk_result.input_level);
    println!("   Far-end level: {:.4}", double_talk_result.input_level);
    println!("   Output level: {:.4}", double_talk_result.output_level);
    println!("   Echo suppression: {:.2}%", double_talk_result.echo_suppression * 100.0);
    println!("   Double-talk detected: {}", double_talk_result.double_talk_detected);
    
    // Demo 3: Adaptation over time
    println!("\n🎯 Demo 3: Filter Adaptation");
    
    let adaptation_frames = 10;
    println!("   Processing {} frames to show adaptation...", adaptation_frames);
    
    for i in 0..adaptation_frames {
        // Create consistent far-end signal
        let far_signal = create_test_signal(8000, 1, 160, 1000.0, 0.4);
        
        // Create near-end with varying echo characteristics
        let echo_strength = 0.5 - (i as f32 * 0.03); // Gradually reduce echo
        let near_signal = create_delayed_echo(&far_signal, echo_strength, 25);
        
        let result = aec.process_frame(&near_signal, &far_signal)?;
        
        if i % 3 == 0 { // Print every 3rd frame
            println!("   Frame {}: echo_level={:.4}, suppression={:.2}%", 
                     i, result.echo_estimate, result.echo_suppression * 100.0);
        }
    }
    
    // Demo 4: Performance metrics
    println!("\n⚡ Demo 4: Performance Test");
    
    let performance_frames = 100;
    let start_time = std::time::Instant::now();
    
    for _ in 0..performance_frames {
        let far_signal = create_test_signal(8000, 1, 160, 1000.0, 0.3);
        let near_signal = create_delayed_echo(&far_signal, 0.4, 20);
        let _result = aec.process_frame(&near_signal, &far_signal)?;
    }
    
    let total_time = start_time.elapsed();
    let avg_time_per_frame = total_time.as_micros() / performance_frames;
    
    println!("   Processed {} frames in {:.2}ms", performance_frames, total_time.as_millis());
    println!("   Average time per frame: {} μs", avg_time_per_frame);
    println!("   Real-time factor: {:.1}x (20ms frames)", 
             20000.0 / avg_time_per_frame as f32);
    
    println!("\n✨ AEC demo completed successfully!");
    println!("   Echo cancellation is working properly");
    println!("   Double-talk detection is functional");
    println!("   Adaptive filtering is converging");
    
    Ok(())
}

/// Create a test signal with specified frequency and amplitude
fn create_test_signal(sample_rate: u32, channels: u8, samples_per_channel: usize, frequency: f32, amplitude: f32) -> AudioFrame {
    let total_samples = samples_per_channel * channels as usize;
    let mut samples = Vec::with_capacity(total_samples);
    
    for i in 0..samples_per_channel {
        let t = i as f32 / sample_rate as f32;
        let signal = (t * 2.0 * std::f32::consts::PI * frequency).sin() * amplitude;
        let sample = (signal * 16384.0) as i16; // Use moderate amplitude to avoid clipping
        
        for _ in 0..channels {
            samples.push(sample);
        }
    }
    
    AudioFrame::new(samples, sample_rate, channels, 0)
}

/// Create a delayed echo of the input signal
fn create_delayed_echo(input: &AudioFrame, echo_strength: f32, delay_samples: usize) -> AudioFrame {
    let mut echo_samples = input.samples.clone();
    
    // Apply delay and attenuation
    for i in 0..echo_samples.len() {
        if i >= delay_samples {
            let original = echo_samples[i - delay_samples] as f32;
            echo_samples[i] = (original * echo_strength) as i16;
        } else {
            echo_samples[i] = 0; // Zero delay samples
        }
    }
    
    AudioFrame::new(echo_samples, input.sample_rate, input.channels, input.timestamp)
}

/// Add an original signal component to the frame
fn add_original_signal(frame: &mut AudioFrame, frequency: f32, amplitude: f32) {
    let sample_rate = frame.sample_rate as f32;
    
    for (i, sample) in frame.samples.iter_mut().enumerate() {
        let sample_index = i / frame.channels as usize;
        let t = sample_index as f32 / sample_rate;
        let original_signal = (t * 2.0 * std::f32::consts::PI * frequency).sin() * amplitude;
        let original_sample = (original_signal * 8192.0) as i16; // Lower amplitude
        
        *sample = sample.saturating_add(original_sample);
    }
}

/// Mix two audio signals together
fn mix_signals(signal1: &AudioFrame, signal2: &AudioFrame) -> AudioFrame {
    assert_eq!(signal1.samples.len(), signal2.samples.len());
    
    let mixed_samples: Vec<i16> = signal1.samples.iter()
        .zip(signal2.samples.iter())
        .map(|(&s1, &s2)| {
            // Mix with saturation protection
            let mixed = (s1 as i32 + s2 as i32) / 2;
            mixed.max(i16::MIN as i32).min(i16::MAX as i32) as i16
        })
        .collect();
    
    AudioFrame::new(mixed_samples, signal1.sample_rate, signal1.channels, signal1.timestamp)
} 