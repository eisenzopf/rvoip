//! Example demonstrating the Opus payload format
//!
//! This example shows how to use the Opus payload format handler
//! for RTP transmission.

use bytes::Bytes;
use rvoip_rtp_core::{
    PayloadFormat, PayloadType, create_payload_format,
    OpusPayloadFormat, OpusBandwidth,
};
use tracing::{info, debug};

fn main() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();
    
    info!("RTP Opus Payload Format Example");
    
    // Create Opus payload format handler with different configurations
    let opus_mono = OpusPayloadFormat::new(101, 1)
        .with_max_bitrate(32000) // 32 kbit/s
        .with_bandwidth(OpusBandwidth::Wideband)
        .with_duration(20); // 20ms
    
    let opus_stereo = OpusPayloadFormat::new(102, 2)
        .with_max_bitrate(64000) // 64 kbit/s
        .with_bandwidth(OpusBandwidth::Fullband)
        .with_duration(20); // 20ms
    
    // Display information about the Opus formats
    info!("Mono Opus: PT={}, Channels={}, Bitrate={} bit/s, Bandwidth={}", 
          opus_mono.payload_type(),
          opus_mono.channels(),
          opus_mono.max_bitrate(),
          opus_mono.bandwidth().name());
          
    info!("Stereo Opus: PT={}, Channels={}, Bitrate={} bit/s, Bandwidth={}", 
          opus_stereo.payload_type(),
          opus_stereo.channels(),
          opus_stereo.max_bitrate(),
          opus_stereo.bandwidth().name());
    
    // Create some mock audio data (960 samples = 20ms of 48kHz audio)
    let pcm_data = create_sine_wave(48000, 440.0, 0.020, opus_mono.channels());
    info!("Created {} bytes of 48kHz PCM audio data", pcm_data.len());
    
    // In a real implementation, we would encode the PCM to Opus here
    // For this example, we'll just simulate encoding with compressed data
    let mono_encoded = simulate_opus_encoding(&pcm_data, opus_mono.max_bitrate(), opus_mono.preferred_packet_duration());
    info!("Encoded to {} bytes of Opus data (mono)", mono_encoded.len());
    
    // Pack the Opus data into an RTP payload
    let payload = opus_mono.pack(&mono_encoded, 0);
    info!("Packed into {} bytes of RTP payload", payload.len());
    
    // Calculate timing information
    let timestamp_increment = opus_mono.samples_from_duration(20);
    info!("20ms at 48kHz = {} samples for RTP timestamp increment", timestamp_increment);
    
    // Demonstrate packet size calculations for different bitrates
    let bandwidths = [
        OpusBandwidth::Narrowband,
        OpusBandwidth::Mediumband,
        OpusBandwidth::Wideband,
        OpusBandwidth::SuperWideband,
        OpusBandwidth::Fullband,
    ];
    
    let bitrates = [8000, 16000, 24000, 32000, 64000, 128000];
    
    info!("Opus packet size estimates for 20ms frames:");
    info!("+-----------------+-------+-------+-------+-------+-------+-------+");
    info!("| Bandwidth       | 8kbps | 16kbps| 24kbps| 32kbps| 64kbps|128kbps|");
    info!("+-----------------+-------+-------+-------+-------+-------+-------+");
    
    for bandwidth in &bandwidths {
        let mut line = format!("| {:<15} |", bandwidth.name().split_once(' ').unwrap().0);
        
        for &bitrate in &bitrates {
            let format = OpusPayloadFormat::new(101, 1)
                .with_max_bitrate(bitrate)
                .with_bandwidth(*bandwidth);
                
            let size = format.packet_size_from_duration(20);
            line.push_str(&format!(" {:<6} |", format!("{}B", size)));
        }
        
        info!("{}", line);
    }
    
    info!("+-----------------+-------+-------+-------+-------+-------+-------+");
    
    // Using the factory function with a dynamic payload type
    if let Some(PayloadType::Dynamic(pt)) = Some(PayloadType::Dynamic(101)) {
        match create_payload_format(PayloadType::Dynamic(pt), None) {
            Some(format) => {
                info!("Created dynamic format with PT={}", format.payload_type());
                
                // The default settings should match what we expect for Opus
                assert_eq!(format.clock_rate(), 48000);
                
                // Calculate packet sizes
                let packet_size = format.packet_size_from_duration(20);
                info!("20ms of Opus at default bitrate = {} bytes (max)", packet_size);
            },
            None => {
                info!("Failed to create format for dynamic payload type {}", pt);
            }
        }
    }
}

/// Create a sine wave audio sample
fn create_sine_wave(sample_rate: u32, frequency: f32, duration: f32, channels: u8) -> Vec<u8> {
    let num_samples = (sample_rate as f32 * duration) as usize;
    let mut result = Vec::with_capacity(num_samples * channels as usize);
    
    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        let amplitude = (2.0 * std::f32::consts::PI * frequency * t).sin();
        
        // Scale to 8-bit unsigned PCM (0-255) for simplicity
        // In real-world use, Opus would operate on 16-bit PCM
        let sample_value = ((amplitude + 1.0) * 127.5) as u8;
        
        // Add the sample for each channel
        for _ in 0..channels {
            result.push(sample_value);
        }
    }
    
    result
}

/// Simulate Opus encoding
fn simulate_opus_encoding(pcm_data: &[u8], bitrate: u32, duration_ms: u32) -> Vec<u8> {
    // Opus is a variable bitrate codec and highly compressed.
    // For this simulation, we'll allocate a buffer based on the maximum bitrate
    // and fill it with some derived data
    
    // Calculate maximum size based on bitrate and duration
    let max_bytes = (bitrate * duration_ms) / (8 * 1000);
    let mut result = Vec::with_capacity(max_bytes as usize);
    
    // In a real implementation, this would be actual Opus encoding
    // For this simulation, we'll just take a subset of the input data
    // and add some Opus-like header bytes
    
    // Add TOC byte (just a placeholder, not real Opus)
    result.push(0x78); // Made-up TOC byte
    
    // Fill with a subset of the PCM data to simulate compression
    let compression_ratio = 4; // Arbitrary compression ratio
    let sample_stride = pcm_data.len() / (max_bytes as usize - 1);
    
    for i in (0..pcm_data.len()).step_by(sample_stride.max(1)) {
        if result.len() >= max_bytes as usize {
            break;
        }
        result.push(pcm_data[i]);
    }
    
    result
} 