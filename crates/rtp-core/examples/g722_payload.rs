//! Example demonstrating the G.722 payload format
//!
//! This example shows how to use the G.722 payload format handler
//! for RTP transmission.

use bytes::Bytes;
use rvoip_rtp_core::{
    PayloadFormat, PayloadType, create_payload_format,
    G722PayloadFormat,
};
use tracing::{info, debug};

fn main() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();
    
    info!("RTP G.722 Payload Format Example");
    
    // Create G.722 payload format handler directly
    let g722_format = G722PayloadFormat::new(8000);
    
    // Create some mock audio data (320 bytes = 20ms of 16kHz audio)
    let audio_data = create_sine_wave(16000, 440.0, 0.020);
    info!("Created {} bytes of 16kHz PCM audio data", audio_data.len());
    
    // In a real implementation, we would encode the PCM to G.722 here
    // For this example, we'll just simulate encoding by using half the size
    // (since G.722 compresses 16-bit PCM to 4 bits per sample)
    let encoded_data = simulate_g722_encoding(&audio_data);
    info!("Encoded to {} bytes of G.722 data", encoded_data.len());
    
    // Pack the G.722 data into an RTP payload
    let payload = g722_format.pack(&encoded_data, 0);
    info!("Packed into {} bytes of RTP payload", payload.len());
    
    // Demonstrate the special timestamp handling of G.722
    let ms_duration = 20;
    let rtp_timestamp_inc = g722_format.samples_from_duration(ms_duration);
    let actual_samples = g722_format.actual_samples_from_duration(ms_duration);
    
    info!("G.722 special timestamp handling:");
    info!("  - {} ms of audio = {} samples at 16kHz", ms_duration, actual_samples);
    info!("  - RTP timestamp increases by only {} (8kHz rate)", rtp_timestamp_inc);
    
    // Using the factory function
    match create_payload_format(PayloadType::G722, None) {
        Some(format) => {
            info!("Created {} format with PT={}", PayloadType::G722.name(), format.payload_type());
            
            // Calculate packet sizes
            let packet_size = format.packet_size_from_duration(20);
            info!("20ms of G.722 = {} bytes", packet_size);
            
            // Verify that the packet size matches our encoded data
            assert_eq!(packet_size, encoded_data.len());
        },
        None => {
            info!("Failed to create G.722 format handler");
        }
    }
}

/// Create a sine wave audio sample (16-bit PCM)
fn create_sine_wave(sample_rate: u32, frequency: f32, duration: f32) -> Vec<u8> {
    let num_samples = (sample_rate as f32 * duration) as usize;
    let mut result = Vec::with_capacity(num_samples);
    
    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        let amplitude = (2.0 * std::f32::consts::PI * frequency * t).sin();
        
        // Scale to 8-bit unsigned PCM (0-255) for simplicity
        // In a real implementation, this would be 16-bit PCM
        let sample_value = ((amplitude + 1.0) * 127.5) as u8;
        result.push(sample_value);
    }
    
    result
}

/// Simulate G.722 encoding (compression from 16-bit PCM to 4 bits per sample)
/// This is just for demonstration - a real implementation would use a proper G.722 codec
fn simulate_g722_encoding(pcm_data: &[u8]) -> Vec<u8> {
    // G.722 compresses 16kHz 16-bit PCM to 4 bits per sample
    // For this simulation, we'll just take every other sample 
    // to simulate the compression (not real G.722 encoding!)
    let num_output_bytes = pcm_data.len() / 2;
    let mut result = Vec::with_capacity(num_output_bytes);
    
    for i in 0..num_output_bytes {
        result.push(pcm_data[i * 2]);
    }
    
    result
} 