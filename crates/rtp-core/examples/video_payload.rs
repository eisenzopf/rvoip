//! Example demonstrating VP8 and VP9 payload formats
//!
//! This example shows how to use the VP8 and VP9 payload format handlers
//! for RTP video transmission.

use bytes::Bytes;
use rvoip_rtp_core::{
    PayloadFormat, PayloadType, create_payload_format,
    Vp8PayloadFormat, Vp9PayloadFormat,
};
use tracing::{info, debug};

fn main() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();
    
    info!("RTP Video Payload Format Example");
    
    // Create VP8 and VP9 payload format handlers with different configurations
    let vp8_format = Vp8PayloadFormat::new(98)
        .with_bitrate(2_000_000) // 2 Mbps
        .with_frame_rate(30);    // 30 fps
    
    let vp9_format = Vp9PayloadFormat::new(99)
        .with_bitrate(3_000_000) // 3 Mbps
        .with_frame_rate(60)     // 60 fps
        .with_picture_id(true)
        .with_layer_indices(true);
    
    // Display information about the formats
    info!("VP8: PT={}, Clock Rate={} Hz, Frame Rate={} fps, Bitrate={} bps", 
          vp8_format.payload_type(),
          vp8_format.clock_rate(),
          1000 / vp8_format.preferred_packet_duration(),
          2_000_000);
          
    info!("VP9: PT={}, Clock Rate={} Hz, Frame Rate={} fps, Bitrate={} bps", 
          vp9_format.payload_type(),
          vp9_format.clock_rate(),
          1000 / vp9_format.preferred_packet_duration(),
          3_000_000);
    
    // Create some mock video data (could be a video frame or part of one)
    let video_frame = create_mock_video_frame(640, 480);
    info!("Created {} bytes of mock video data", video_frame.len());
    
    // In a real implementation, we would encode the video to VP8/VP9 here
    // For this example, we'll just use the mock data directly
    
    // Pack the data into RTP payloads
    let timestamp_vp8 = 90000; // Arbitrary timestamp
    let vp8_payload = vp8_format.pack(&video_frame, timestamp_vp8);
    info!("Packed into {} bytes of VP8 RTP payload", vp8_payload.len());
    
    let timestamp_vp9 = 180000; // Arbitrary timestamp
    let vp9_payload = vp9_format.pack(&video_frame, timestamp_vp9);
    info!("Packed into {} bytes of VP9 RTP payload", vp9_payload.len());
    
    // Calculate frame size information for VP8
    let timestamp_increment_vp8 = vp8_format.samples_from_duration(vp8_format.preferred_packet_duration());
    info!("VP8: {} ms at 90kHz = {} samples for RTP timestamp increment", 
          vp8_format.preferred_packet_duration(),
          timestamp_increment_vp8);
    
    // Calculate frame size information for VP9
    let timestamp_increment_vp9 = vp9_format.samples_from_duration(vp9_format.preferred_packet_duration());
    info!("VP9: {} ms at 90kHz = {} samples for RTP timestamp increment", 
          vp9_format.preferred_packet_duration(),
          timestamp_increment_vp9);
    
    // Demonstrate typical packet sizes for various resolutions and frame rates
    info!("");
    info!("Estimated VP8 packet sizes for a full frame:");
    info!("+-----------------+--------+--------+--------+--------+");
    info!("| Resolution      | 15 fps | 30 fps | 60 fps | 90 fps |");
    info!("+-----------------+--------+--------+--------+--------+");
    
    let resolutions = [
        (320, 240, "320x240"),
        (640, 480, "640x480"),
        (1280, 720, "1280x720"),
        (1920, 1080, "1920x1080"),
    ];
    
    let frame_rates = [15, 30, 60, 90];
    
    for (width, height, name) in &resolutions {
        let mut line = format!("| {:<15} |", name);
        
        for &fps in &frame_rates {
            let format = Vp8PayloadFormat::new(98)
                .with_bitrate(calculate_bitrate(*width, *height, fps))
                .with_frame_rate(fps);
                
            let duration_ms = 1000 / fps;
            let size_bytes = format.packet_size_from_duration(duration_ms);
            let size_kb = size_bytes as f64 / 1024.0;
            
            line.push_str(&format!(" {:<6.1} |", format!("{}KB", size_kb)));
        }
        
        info!("{}", line);
    }
    
    info!("+-----------------+--------+--------+--------+--------+");
    info!("");
    
    // Demonstrate VP9 with multiple layers
    info!("VP9 can support multiple spatial and temporal layers:");
    let vp9_advanced = Vp9PayloadFormat::new(99)
        .with_layer_indices(true)
        .with_flexible_mode(true)
        .with_bitrate(5_000_000); // 5 Mbps
    
    let advanced_descriptor_size = vp9_advanced.descriptor_size();
    info!("VP9 with advanced features has {} byte descriptor overhead", advanced_descriptor_size);
    
    // Using the factory function with dynamic payload types
    info!("");
    info!("Creating formats using the payload factory:");
    
    if let Some(format) = create_payload_format(PayloadType::Dynamic(98), Some(90000)) {
        info!("Created format for PT=98 with clock rate 90kHz: {}", 
              format.payload_type());
        
        // Calculate packet sizes for a typical MTU
        let mtu = 1400; // Typical MTU minus IP/UDP headers
        let duration = format.duration_from_packet_size(mtu);
        
        info!("A packet size of {} bytes represents ~{} ms of video at default bitrate", 
              mtu, duration);
    }
    
    if let Some(format) = create_payload_format(PayloadType::Dynamic(99), Some(90000)) {
        info!("Created format for PT=99 with clock rate 90kHz: {}", 
              format.payload_type());
    }
}

/// Create a mock video frame for testing
fn create_mock_video_frame(width: u32, height: u32) -> Vec<u8> {
    // In real application, this would be actual encoded video data
    // Here we'll just create a buffer with a pattern based on position
    
    // Rough estimate - actual compressed size would depend on the codec and content
    // Here we're just creating a simple test pattern, not real VP8/VP9 data
    let size = (width * height) / 4; // Very rough approximation of compressed size
    let mut data = Vec::with_capacity(size as usize);
    
    // Create a simple pattern
    for i in 0..size {
        let value = ((i % 255) + 1) as u8;
        data.push(value);
    }
    
    data
}

/// Calculate an estimated bitrate for a given resolution and frame rate
fn calculate_bitrate(width: u32, height: u32, fps: u32) -> u32 {
    // This is a very rough heuristic - real bitrates depend on content complexity,
    // codec efficiency, quality target, etc.
    let pixels = width * height;
    let bits_per_pixel = 0.1; // Very rough approximation
    
    let bits_per_frame = (pixels as f64 * bits_per_pixel) as u32;
    let bits_per_second = bits_per_frame * fps;
    
    bits_per_second
} 