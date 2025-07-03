//! RFC 3551 (RTP A/V Profile) Compatibility Example
//!
//! This example demonstrates and validates compatibility with RFC 3551,
//! which defines the RTP Audio/Video Profile including standard payload types
//! and their mappings, clock rates, and format parameters.

use std::collections::HashMap;
use bytes::Bytes;
use tracing::{info, debug};

use rvoip_rtp_core::{
    payload::{PayloadType, PayloadFormat, PayloadFormatFactory, create_payload_format},
    RtpPacket, RtpHeader, RtpTimestamp,
};

/// Payload type information per RFC 3551
struct PayloadTypeInfo {
    /// Payload type value
    pt: u8,
    /// Name of the encoding
    encoding_name: &'static str,
    /// Clock rate in Hertz
    clock_rate: u32,
    /// Number of channels (audio only)
    channels: Option<u8>,
    /// Audio or Video
    media_type: &'static str,
}

/// Create a table of standard payload types from RFC 3551
fn create_rfc3551_table() -> Vec<PayloadTypeInfo> {
    vec![
        // Audio payload types
        PayloadTypeInfo { pt: 0, encoding_name: "PCMU", clock_rate: 8000, channels: Some(1), media_type: "audio" },
        PayloadTypeInfo { pt: 3, encoding_name: "GSM", clock_rate: 8000, channels: Some(1), media_type: "audio" },
        PayloadTypeInfo { pt: 4, encoding_name: "G723", clock_rate: 8000, channels: Some(1), media_type: "audio" },
        PayloadTypeInfo { pt: 5, encoding_name: "DVI4", clock_rate: 8000, channels: Some(1), media_type: "audio" },
        PayloadTypeInfo { pt: 6, encoding_name: "DVI4", clock_rate: 16000, channels: Some(1), media_type: "audio" },
        PayloadTypeInfo { pt: 7, encoding_name: "LPC", clock_rate: 8000, channels: Some(1), media_type: "audio" },
        PayloadTypeInfo { pt: 8, encoding_name: "PCMA", clock_rate: 8000, channels: Some(1), media_type: "audio" },
        PayloadTypeInfo { pt: 9, encoding_name: "G722", clock_rate: 8000, channels: Some(1), media_type: "audio" },
        PayloadTypeInfo { pt: 10, encoding_name: "L16", clock_rate: 44100, channels: Some(2), media_type: "audio" },
        PayloadTypeInfo { pt: 11, encoding_name: "L16", clock_rate: 44100, channels: Some(1), media_type: "audio" },
        PayloadTypeInfo { pt: 12, encoding_name: "QCELP", clock_rate: 8000, channels: Some(1), media_type: "audio" },
        PayloadTypeInfo { pt: 13, encoding_name: "CN", clock_rate: 8000, channels: Some(1), media_type: "audio" },
        PayloadTypeInfo { pt: 14, encoding_name: "MPA", clock_rate: 90000, channels: None, media_type: "audio" },
        PayloadTypeInfo { pt: 15, encoding_name: "G728", clock_rate: 8000, channels: Some(1), media_type: "audio" },
        PayloadTypeInfo { pt: 16, encoding_name: "DVI4", clock_rate: 11025, channels: Some(1), media_type: "audio" },
        PayloadTypeInfo { pt: 17, encoding_name: "DVI4", clock_rate: 22050, channels: Some(1), media_type: "audio" },
        PayloadTypeInfo { pt: 18, encoding_name: "G729", clock_rate: 8000, channels: Some(1), media_type: "audio" },
        
        // Video payload types
        PayloadTypeInfo { pt: 25, encoding_name: "CelB", clock_rate: 90000, channels: None, media_type: "video" },
        PayloadTypeInfo { pt: 26, encoding_name: "JPEG", clock_rate: 90000, channels: None, media_type: "video" },
        PayloadTypeInfo { pt: 28, encoding_name: "nv", clock_rate: 90000, channels: None, media_type: "video" },
        PayloadTypeInfo { pt: 31, encoding_name: "H261", clock_rate: 90000, channels: None, media_type: "video" },
        PayloadTypeInfo { pt: 32, encoding_name: "MPV", clock_rate: 90000, channels: None, media_type: "video" },
        PayloadTypeInfo { pt: 33, encoding_name: "MP2T", clock_rate: 90000, channels: None, media_type: "video" },
        PayloadTypeInfo { pt: 34, encoding_name: "H263", clock_rate: 90000, channels: None, media_type: "video" },
        
        // Dynamic payload types (examples)
        PayloadTypeInfo { pt: 96, encoding_name: "H264", clock_rate: 90000, channels: None, media_type: "video" },
        PayloadTypeInfo { pt: 97, encoding_name: "VP8", clock_rate: 90000, channels: None, media_type: "video" },
        PayloadTypeInfo { pt: 98, encoding_name: "VP9", clock_rate: 90000, channels: None, media_type: "video" },
        PayloadTypeInfo { pt: 99, encoding_name: "opus", clock_rate: 48000, channels: Some(2), media_type: "audio" },
    ]
}

/// Map RFC 3551 encoding names to our PayloadType enum
fn get_payload_type_enum(name: &str, pt: u8) -> Option<PayloadType> {
    match name {
        "PCMU" => Some(PayloadType::PCMU),
        "PCMA" => Some(PayloadType::PCMA),
        "G722" => Some(PayloadType::G722),
        "opus" => Some(PayloadType::Opus),
        "VP8" => Some(PayloadType::VP8),
        "VP9" => Some(PayloadType::VP9),
        // Add other mappings as we implement them
        _ => {
            // For types we don't have specific enum values for
            if pt < 96 {
                // Static payload type
                Some(PayloadType::from_u8(pt))
            } else {
                // Dynamic payload type
                Some(PayloadType::Dynamic(pt))
            }
        }
    }
}

/// Verify our implementation supports the correct clock rates for standard payload types
fn verify_clock_rates() -> Result<(), Box<dyn std::error::Error>> {
    info!("Verifying clock rates for standard payload types...");
    
    // Create payload formats for each standard type we support
    // and verify they have the correct clock rate
    let rfc3551_table = create_rfc3551_table();
    
    for pt_info in rfc3551_table {
        // Only test the payload types we have specific implementations for
        let enum_type = match pt_info.encoding_name {
            "PCMU" | "PCMA" | "G722" | "opus" | "VP8" | "VP9" => {
                get_payload_type_enum(pt_info.encoding_name, pt_info.pt)
            },
            _ => None
        };
        
        if let Some(payload_enum) = enum_type {
            // Convert channels from u8 to u32 for create_payload_format
            let channels_u32 = pt_info.channels.map(|c| c as u32);
            
            // Create payload format handler
            let format_result = create_payload_format(payload_enum, channels_u32);
            
            if let Some(format) = format_result {
                // Verify clock rate matches RFC 3551
                let actual_clock_rate = format.clock_rate();
                
                if actual_clock_rate == pt_info.clock_rate {
                    info!("✅ {} (PT={}) has correct clock rate: {} Hz", 
                          pt_info.encoding_name, pt_info.pt, pt_info.clock_rate);
                } else {
                    info!("❌ {} (PT={}) has incorrect clock rate: {} Hz, expected: {} Hz", 
                          pt_info.encoding_name, pt_info.pt, actual_clock_rate, pt_info.clock_rate);
                }
                
                // For audio formats, also verify channel count
                if pt_info.media_type == "audio" && pt_info.channels.is_some() {
                    let actual_channels = format.channels();
                    let expected_channels = pt_info.channels.unwrap_or(1);
                    
                    if actual_channels == expected_channels {
                        info!("✅ {} (PT={}) has correct channel count: {}", 
                              pt_info.encoding_name, pt_info.pt, expected_channels);
                    } else {
                        info!("❌ {} (PT={}) has incorrect channel count: {}, expected: {}", 
                              pt_info.encoding_name, pt_info.pt, actual_channels, expected_channels);
                    }
                }
            } else {
                info!("⚠️ {} (PT={}) format handler not implemented", 
                      pt_info.encoding_name, pt_info.pt);
            }
        }
    }
    
    Ok(())
}

/// Test encoding/decoding with standard payload types
fn test_payload_handling() -> Result<(), Box<dyn std::error::Error>> {
    info!("Testing sample packet encoding/decoding with standard formats...");
    
    // Test G.711 µ-law (PCMU, payload type 0)
    if let Some(pcmu_format) = create_payload_format(PayloadType::PCMU, None) {
        let pcmu_data = vec![0x55, 0xAA, 0x55, 0xAA]; // Sample G.711 µ-law data
        
        // Test packing/unpacking
        let pcmu_ts = 1000;
        let pcmu_packed = pcmu_format.pack(&pcmu_data, pcmu_ts);
        let pcmu_unpacked = pcmu_format.unpack(&pcmu_packed, pcmu_ts).to_vec();
        
        info!("G.711 µ-law: Original size: {}, Packed size: {}, Unpacked size: {}", 
              pcmu_data.len(), pcmu_packed.len(), pcmu_unpacked.len());
        info!("G.711 µ-law: Data matches: {}", pcmu_data == pcmu_unpacked);
    } else {
        info!("G.711 µ-law: Format handler not available");
    }
    
    // Test G.722 (special case with 16kHz sampling but 8kHz RTP timestamp rate)
    if let Some(g722_format) = create_payload_format(PayloadType::G722, None) {
        let g722_data = vec![0x11, 0x22, 0x33, 0x44]; // Sample G.722 data
        
        // G.722 special timestamp handling should be transparent to the user
        let g722_ts = 2000;
        let g722_packed = g722_format.pack(&g722_data, g722_ts);
        let g722_unpacked = g722_format.unpack(&g722_packed, g722_ts).to_vec();
        
        info!("G.722: Original size: {}, Packed size: {}, Unpacked size: {}", 
              g722_data.len(), g722_packed.len(), g722_unpacked.len());
        info!("G.722: Data matches: {}", g722_data == g722_unpacked);
    } else {
        info!("G.722: Format handler not available");
    }
    
    // Test creating an RTP packet with standard payload type
    let rtp_header = RtpHeader::new(0, 1000, 8000, 0x12345678);
    let payload = Bytes::from(vec![0x55, 0xAA, 0x55, 0xAA]); // Sample payload
    let rtp_packet = RtpPacket::new(rtp_header, payload);
    
    // Serialize and deserialize
    let serialized = rtp_packet.serialize()?;
    let deserialized = RtpPacket::parse(&serialized)?;
    
    info!("RTP packet with PCMU: Successfully serialized ({} bytes) and deserialized", 
          serialized.len());
    info!("RTP packet header: PT={}, seq={}, timestamp={}", 
          deserialized.header.payload_type,
          deserialized.header.sequence_number,
          deserialized.header.timestamp);
    
    Ok(())
}

/// Test timestamp calculations for different payload types
fn test_timestamp_calculations() -> Result<(), Box<dyn std::error::Error>> {
    info!("Testing timestamp calculations for various payload types...");
    
    // G.711 (8kHz clock rate)
    if let Some(pcmu_format) = create_payload_format(PayloadType::PCMU, None) {
        let pcmu_samples = 160; // 20ms of audio at 8kHz
        let pcmu_duration = pcmu_format.duration_from_samples(pcmu_samples);
        
        // Calculate timestamp increment manually
        let pcmu_ts_increment = ((pcmu_format.clock_rate() as f64) * (pcmu_duration as f64) / 1000.0) as u32;
        
        info!("G.711 µ-law: {} samples ({}ms) = {} timestamp increment", 
              pcmu_samples, pcmu_duration, pcmu_ts_increment);
    }
    
    // G.722 (16kHz sampling, 8kHz timestamp rate)
    if let Some(g722_format) = create_payload_format(PayloadType::G722, None) {
        let g722_samples = 320; // 20ms of audio at 16kHz
        let g722_duration = g722_format.duration_from_samples(g722_samples);
        
        // Calculate timestamp increment manually
        let g722_ts_increment = ((g722_format.clock_rate() as f64) * (g722_duration as f64) / 1000.0) as u32;
        
        info!("G.722: {} samples ({}ms) = {} timestamp increment", 
              g722_samples, g722_duration, g722_ts_increment);
    }
    
    // Opus (48kHz clock rate)
    if let Some(opus_format) = create_payload_format(PayloadType::Opus, Some(2)) {
        let opus_samples = 960; // 20ms of audio at 48kHz
        let opus_duration = opus_format.duration_from_samples(opus_samples);
        
        // Calculate timestamp increment manually
        let opus_ts_increment = ((opus_format.clock_rate() as f64) * (opus_duration as f64) / 1000.0) as u32;
        
        info!("Opus: {} samples ({}ms) = {} timestamp increment", 
              opus_samples, opus_duration, opus_ts_increment);
    }
    
    // VP8 (90kHz clock rate for video)
    if let Some(vp8_format) = create_payload_format(PayloadType::VP8, None) {
        // 33.33ms for 30fps video
        let vp8_duration = 33.33;
        
        // Calculate timestamp increment manually
        let vp8_ts_increment = ((vp8_format.clock_rate() as f64) * vp8_duration / 1000.0) as u32;
        
        info!("VP8: {:.2}ms = {} timestamp increment", 
              vp8_duration, vp8_ts_increment);
    }
    
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    
    info!("RFC 3551 (RTP A/V Profile) Compatibility Test");
    info!("=============================================");
    
    // 1. Verify clock rates for standard payload types
    verify_clock_rates()?;
    
    info!("\n");
    
    // 2. Test payload handling
    test_payload_handling()?;
    
    info!("\n");
    
    // 3. Test timestamp calculations
    test_timestamp_calculations()?;
    
    info!("\nRFC 3551 compatibility test completed successfully");
    
    Ok(())
} 