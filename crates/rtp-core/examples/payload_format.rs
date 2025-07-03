//! Example demonstrating the use of payload format handlers
//!
//! This example shows how to use the payload format handlers to
//! pack and unpack media data for RTP transmission.

use bytes::Bytes;
use rvoip_rtp_core::{
    PayloadFormat, PayloadType, create_payload_format,
    G711UPayloadFormat, G711APayloadFormat,
};
use tracing::{info, debug};

fn main() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();
    
    info!("RTP Payload Format Example");
    
    // Create G.711 μ-law payload format handler
    let pcmu_format = G711UPayloadFormat::new(8000);
    
    // Create some mock audio data (160 bytes = 20ms of 8kHz audio)
    let pcm_data = create_sine_wave(8000, 440.0, 0.020);
    info!("Created {} bytes of audio data", pcm_data.len());
    
    // Pack the audio data into an RTP payload
    let payload = pcmu_format.pack(&pcm_data, 0);
    info!("Packed {} bytes of audio data into {} bytes of payload", pcm_data.len(), payload.len());
    
    // Unpack the RTP payload back to audio data
    let unpacked = pcmu_format.unpack(&payload, 0);
    info!("Unpacked payload back to {} bytes of audio data", unpacked.len());
    
    // Verify that the unpacked data matches the original
    assert_eq!(pcm_data.len(), unpacked.len());
    assert_eq!(&pcm_data[..], &unpacked[..]);
    
    // Show timing calculations
    let samples = pcmu_format.samples_from_duration(20);
    let duration = pcmu_format.duration_from_samples(samples);
    info!("20ms at 8kHz = {} samples, which is {}ms", samples, duration);
    
    // Demonstrate using the factory function
    let payload_type = PayloadType::PCMU;
    match create_payload_format(payload_type, None) {
        Some(format) => {
            info!("Created {} format with PT={}", payload_type.name(), format.payload_type());
            
            // Show packet size calculations
            let packet_size = format.packet_size_from_duration(20);
            let calculated_duration = format.duration_from_packet_size(packet_size);
            info!("20ms packet size = {} bytes, which is {}ms", packet_size, calculated_duration);
        },
        None => {
            info!("No format handler available for {}", payload_type.name());
        }
    }
    
    // Test with PCMA (A-law)
    let pcma_format = G711APayloadFormat::new(8000);
    info!("G.711 A-law format: PT={}, rate={}Hz, channels={}", 
          pcma_format.payload_type(), 
          pcma_format.clock_rate(),
          pcma_format.channels());
}

/// Create a sine wave audio sample
fn create_sine_wave(sample_rate: u32, frequency: f32, duration: f32) -> Vec<u8> {
    let num_samples = (sample_rate as f32 * duration) as usize;
    let mut samples = Vec::with_capacity(num_samples);
    
    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        let amplitude = (2.0 * std::f32::consts::PI * frequency * t).sin();
        
        // Scale to 8-bit unsigned PCM (0-255)
        let sample_value = ((amplitude + 1.0) * 127.5) as u8;
        samples.push(sample_value);
    }
    
    samples
} 