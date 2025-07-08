//! Audio Performance Benchmark
//!
//! This example benchmarks the audio device system to measure:
//! - Audio capture/playback latency
//! - Throughput (frames per second)
//! - Memory usage patterns
//! - Device switching performance
//!
//! Run with: cargo run --example audio_benchmark

use std::sync::Arc;
use std::time::{Duration, Instant};
use std::collections::VecDeque;
use tokio::time::timeout;
use tracing::{info, warn};

use rvoip_client_core::audio::{
    AudioDeviceManager, AudioDirection, AudioFormat, AudioDevice,
    device::AudioFrame,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for better output
    tracing_subscriber::fmt::init();
    
    println!("📊 Audio Performance Benchmark");
    println!("==============================\n");
    
    // Create audio device manager
    let manager = AudioDeviceManager::new().await?;
    
    // Get default devices
    let input_device = manager.get_default_device(AudioDirection::Input).await?;
    let output_device = manager.get_default_device(AudioDirection::Output).await?;
    
    println!("🎤 Input Device: {} ({})", input_device.info().name, input_device.info().id);
    println!("🔊 Output Device: {} ({})", output_device.info().name, output_device.info().id);
    
    // Run benchmarks
    run_format_compatibility_benchmark(&manager).await?;
    run_capture_benchmark(&input_device).await?;
    run_playback_benchmark(&output_device).await?;
    run_latency_benchmark(&input_device, &output_device).await?;
    run_concurrent_sessions_benchmark(&manager).await?;
    
    println!("\n✨ Benchmark complete!");
    Ok(())
}

/// Test format compatibility performance across different formats
async fn run_format_compatibility_benchmark(manager: &AudioDeviceManager) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n🧪 Format Compatibility Benchmark");
    println!("----------------------------------");
    
    let test_formats = vec![
        ("VoIP 8kHz", AudioFormat::default_voip()),
        ("VoIP 16kHz", AudioFormat::wideband_voip()),
        ("CD 44.1kHz", AudioFormat::new(44100, 2, 16, 20)),
        ("Studio 48kHz", AudioFormat::new(48000, 2, 16, 20)),
        ("High-res 96kHz", AudioFormat::new(96000, 2, 24, 20)),
    ];
    
    let input_devices = manager.list_devices(AudioDirection::Input).await?;
    let output_devices = manager.list_devices(AudioDirection::Output).await?;
    
    for (name, format) in test_formats {
        let start = Instant::now();
        
        let mut input_support = 0;
        let mut output_support = 0;
        
        for device_info in &input_devices {
            let device = manager.create_device(&device_info.id).await?;
            if device.supports_format(&format) {
                input_support += 1;
            }
        }
        
        for device_info in &output_devices {
            let device = manager.create_device(&device_info.id).await?;
            if device.supports_format(&format) {
                output_support += 1;
            }
        }
        
        let elapsed = start.elapsed();
        println!("  {}: {}/{} input, {}/{} output devices ({:.1}ms)", 
                 name, input_support, input_devices.len(), 
                 output_support, output_devices.len(),
                 elapsed.as_millis());
    }
    
    Ok(())
}

/// Benchmark audio capture performance
async fn run_capture_benchmark(device: &Arc<dyn AudioDevice>) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n🎤 Audio Capture Benchmark");
    println!("---------------------------");
    
    // Find supported format
    let format = find_supported_format(device).await?;
    println!("Using format: {}Hz, {} channels", format.sample_rate, format.channels);
    
    // Start capture
    let start_time = Instant::now();
    let mut receiver = device.start_capture(format.clone()).await?;
    let setup_time = start_time.elapsed();
    
    println!("Setup time: {:.1}ms", setup_time.as_millis());
    
    // Collect frames for analysis
    let mut frames_received = 0;
    let mut total_samples = 0;
    let mut frame_times = VecDeque::new();
    let benchmark_duration = Duration::from_secs(5);
    let benchmark_start = Instant::now();
    
    while benchmark_start.elapsed() < benchmark_duration {
        match timeout(Duration::from_millis(100), receiver.recv()).await {
            Ok(Some(frame)) => {
                frames_received += 1;
                total_samples += frame.samples.len();
                frame_times.push_back(Instant::now());
                
                // Keep only recent frame times for jitter calculation
                while frame_times.len() > 100 {
                    frame_times.pop_front();
                }
            }
            Ok(None) => break,
            Err(_) => continue, // Timeout
        }
    }
    
    // Stop capture
    let stop_start = Instant::now();
    device.stop_capture().await?;
    let stop_time = stop_start.elapsed();
    
    // Calculate statistics
    let total_time = benchmark_start.elapsed();
    let fps = frames_received as f64 / total_time.as_secs_f64();
    let sample_rate = total_samples as f64 / total_time.as_secs_f64();
    
    // Calculate frame jitter
    let mut intervals = Vec::new();
    for i in 1..frame_times.len() {
        let interval = frame_times[i].duration_since(frame_times[i-1]);
        intervals.push(interval.as_millis() as f64);
    }
    
    let avg_interval = intervals.iter().sum::<f64>() / intervals.len() as f64;
    let jitter = intervals.iter()
        .map(|&x| (x - avg_interval).abs())
        .sum::<f64>() / intervals.len() as f64;
    
    println!("Results:");
    println!("  Frames received: {}", frames_received);
    println!("  Frame rate: {:.1} fps", fps);
    println!("  Sample rate: {:.0} samples/sec", sample_rate);
    println!("  Average frame interval: {:.1}ms", avg_interval);
    println!("  Frame jitter: {:.1}ms", jitter);
    println!("  Stop time: {:.1}ms", stop_time.as_millis());
    
    Ok(())
}

/// Benchmark audio playback performance
async fn run_playback_benchmark(device: &Arc<dyn AudioDevice>) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n🔊 Audio Playback Benchmark");
    println!("----------------------------");
    
    // Find supported format
    let format = find_supported_format(device).await?;
    println!("Using format: {}Hz, {} channels", format.sample_rate, format.channels);
    
    // Start playback
    let start_time = Instant::now();
    let sender = device.start_playback(format.clone()).await?;
    let setup_time = start_time.elapsed();
    
    println!("Setup time: {:.1}ms", setup_time.as_millis());
    
    // Generate and send test audio frames
    let benchmark_duration = Duration::from_secs(5);
    let benchmark_start = Instant::now();
    let mut frames_sent = 0;
    let mut send_times = Vec::new();
    
    while benchmark_start.elapsed() < benchmark_duration {
        // Generate a test frame (sine wave)
        let samples_per_frame = format.samples_per_frame();
        let mut samples = Vec::with_capacity(samples_per_frame);
        
        for i in 0..samples_per_frame {
            let t = (frames_sent * samples_per_frame + i) as f64 / format.sample_rate as f64;
            let frequency = 440.0; // A4 note
            let amplitude = 0.1; // Quiet to avoid disturbing
            let sample = (amplitude * (2.0 * std::f64::consts::PI * frequency * t).sin() * i16::MAX as f64) as i16;
            samples.push(sample);
        }
        
        let frame = AudioFrame::new(samples, format.clone(), 
                                   benchmark_start.elapsed().as_millis() as u64);
        
        let send_start = Instant::now();
        match sender.send(frame).await {
            Ok(()) => {
                let send_time = send_start.elapsed();
                send_times.push(send_time.as_micros() as f64);
                frames_sent += 1;
            }
            Err(_) => break,
        }
        
        // Maintain proper frame timing
        tokio::time::sleep(Duration::from_millis(format.frame_size_ms as u64)).await;
    }
    
    // Stop playback
    let stop_start = Instant::now();
    device.stop_playback().await?;
    let stop_time = stop_start.elapsed();
    
    // Calculate statistics
    let total_time = benchmark_start.elapsed();
    let fps = frames_sent as f64 / total_time.as_secs_f64();
    
    let avg_send_time = send_times.iter().sum::<f64>() / send_times.len() as f64;
    let max_send_time = send_times.iter().fold(0.0f64, |acc, &x| acc.max(x));
    
    println!("Results:");
    println!("  Frames sent: {}", frames_sent);
    println!("  Frame rate: {:.1} fps", fps);
    println!("  Average send time: {:.0}μs", avg_send_time);
    println!("  Max send time: {:.0}μs", max_send_time);
    println!("  Stop time: {:.1}ms", stop_time.as_millis());
    
    Ok(())
}

/// Benchmark end-to-end latency
async fn run_latency_benchmark(
    input_device: &Arc<dyn AudioDevice>,
    output_device: &Arc<dyn AudioDevice>
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n⚡ End-to-End Latency Benchmark");
    println!("-------------------------------");
    println!("⚠️  This test may produce audio feedback - use headphones!");
    
    // Find compatible format
    let format = find_compatible_format(input_device, output_device).await?;
    println!("Using format: {}Hz, {} channels", format.sample_rate, format.channels);
    
    // Start both devices
    let mut receiver = input_device.start_capture(format.clone()).await?;
    let sender = output_device.start_playback(format.clone()).await?;
    
    println!("Measuring latency for 10 seconds...");
    
    let mut latencies = Vec::new();
    let benchmark_start = Instant::now();
    let benchmark_duration = Duration::from_secs(10);
    
    while benchmark_start.elapsed() < benchmark_duration {
        match timeout(Duration::from_millis(50), receiver.recv()).await {
            Ok(Some(mut frame)) => {
                // Record when we received the frame
                let receive_time = Instant::now();
                
                // Add a timestamp to the frame and send it back
                frame.timestamp_ms = receive_time.elapsed().as_millis() as u64;
                
                let send_start = Instant::now();
                if sender.send(frame).await.is_ok() {
                    let round_trip_time = send_start.duration_since(receive_time);
                    latencies.push(round_trip_time.as_micros() as f64);
                }
            }
            Ok(None) => break,
            Err(_) => continue, // Timeout
        }
    }
    
    // Stop devices
    input_device.stop_capture().await?;
    output_device.stop_playback().await?;
    
    // Calculate latency statistics
    if !latencies.is_empty() {
        latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
        
        let avg_latency = latencies.iter().sum::<f64>() / latencies.len() as f64;
        let min_latency = latencies[0];
        let max_latency = latencies[latencies.len() - 1];
        let p95_latency = latencies[(latencies.len() as f64 * 0.95) as usize];
        
        println!("Results:");
        println!("  Samples: {}", latencies.len());
        println!("  Average latency: {:.0}μs ({:.1}ms)", avg_latency, avg_latency / 1000.0);
        println!("  Min latency: {:.0}μs ({:.1}ms)", min_latency, min_latency / 1000.0);
        println!("  Max latency: {:.0}μs ({:.1}ms)", max_latency, max_latency / 1000.0);
        println!("  95th percentile: {:.0}μs ({:.1}ms)", p95_latency, p95_latency / 1000.0);
    } else {
        println!("  No latency samples collected");
    }
    
    Ok(())
}

/// Benchmark concurrent sessions performance
async fn run_concurrent_sessions_benchmark(manager: &AudioDeviceManager) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n🔄 Concurrent Sessions Benchmark");
    println!("--------------------------------");
    
    // Test with increasing number of concurrent sessions
    let max_sessions = 5;
    
    for session_count in 1..=max_sessions {
        println!("\nTesting {} concurrent sessions:", session_count);
        
        let start_time = Instant::now();
        let mut input_receivers = Vec::new();
        let mut output_senders = Vec::new();
        
        // Start sessions
        for i in 0..session_count {
            let input_device = manager.get_default_device(AudioDirection::Input).await?;
            let output_device = manager.get_default_device(AudioDirection::Output).await?;
            
            let format = find_compatible_format(&input_device, &output_device).await?;
            
            let receiver = input_device.start_capture(format.clone()).await?;
            let sender = output_device.start_playback(format.clone()).await?;
            
            input_receivers.push((input_device, receiver));
            output_senders.push((output_device, sender));
        }
        
        let setup_time = start_time.elapsed();
        println!("  Setup time: {:.1}ms", setup_time.as_millis());
        
        // Run for a short time
        tokio::time::sleep(Duration::from_secs(2)).await;
        
        // Stop all sessions
        let stop_start = Instant::now();
        
        for (device, _) in input_receivers {
            device.stop_capture().await?;
        }
        
        for (device, _) in output_senders {
            device.stop_playback().await?;
        }
        
        let stop_time = stop_start.elapsed();
        println!("  Stop time: {:.1}ms", stop_time.as_millis());
        println!("  Total time: {:.1}ms", start_time.elapsed().as_millis());
    }
    
    Ok(())
}

/// Find a supported format for a device
async fn find_supported_format(device: &Arc<dyn AudioDevice>) -> Result<AudioFormat, Box<dyn std::error::Error>> {
    let test_formats = vec![
        AudioFormat::new(48000, 1, 16, 20),
        AudioFormat::new(44100, 1, 16, 20),
        AudioFormat::wideband_voip(),
        AudioFormat::default_voip(),
    ];
    
    for format in test_formats {
        if device.supports_format(&format) {
            return Ok(format);
        }
    }
    
    Err("No supported format found".into())
}

/// Find a format compatible with both devices
async fn find_compatible_format(
    input_device: &Arc<dyn AudioDevice>,
    output_device: &Arc<dyn AudioDevice>
) -> Result<AudioFormat, Box<dyn std::error::Error>> {
    let test_formats = vec![
        AudioFormat::new(48000, 1, 16, 20),
        AudioFormat::new(44100, 1, 16, 20),
        AudioFormat::wideband_voip(),
        AudioFormat::default_voip(),
    ];
    
    for format in test_formats {
        if input_device.supports_format(&format) && output_device.supports_format(&format) {
            return Ok(format);
        }
    }
    
    Err("No compatible format found".into())
} 