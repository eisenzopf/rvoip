//! Payload Type Registry Demo
//!
//! This example demonstrates the enhanced payload type handling with:
//! - RFC 3551 compliant static payload types
//! - Dynamic payload type support (96-127)
//! - Proper media frame type detection
//! - Codec information lookup

use rvoip_rtp_core::api::common::frame::MediaFrameType;
use rvoip_rtp_core::payload::registry::{get_codec_name, get_media_frame_type, get_payload_info};

fn main() {
    println!("=== RTP Payload Type Registry Demo ===\n");

    // Test RFC 3551 static audio payload types
    println!("📻 RFC 3551 Static Audio Payload Types:");
    let audio_payload_types = [0, 3, 4, 8, 9, 11, 18];
    for pt in audio_payload_types {
        demonstrate_payload_type(pt);
    }

    println!("\n📺 RFC 3551 Static Video Payload Types:");
    let video_payload_types = [25, 26, 31, 32, 34];
    for pt in video_payload_types {
        demonstrate_payload_type(pt);
    }

    println!("\n🔧 Dynamic Payload Types (96-127):");
    let dynamic_payload_types = [96, 97, 98, 111];
    for pt in dynamic_payload_types {
        demonstrate_payload_type(pt);
    }

    println!("\n🚫 Unregistered Payload Types (showing fallback behavior):");
    let unregistered_payload_types = [1, 27, 50, 100, 200];
    for pt in unregistered_payload_types {
        demonstrate_payload_type(pt);
    }

    println!("\n✅ Enhanced Payload Type Handling Benefits:");
    println!("• RFC 3551 compliant payload type mappings");
    println!("• Proper audio/video/data classification");
    println!("• Support for dynamic payload types (96-127)");
    println!("• Centralized payload type management");
    println!("• Codec-specific information lookup");
    println!("• Eliminated hardcoded payload type logic");
    println!("• Consistent behavior across all RTP components");
}

fn demonstrate_payload_type(payload_type: u8) {
    let media_type = get_media_frame_type(payload_type);
    let codec_name = get_codec_name(payload_type);

    let media_icon = match media_type {
        MediaFrameType::Audio => "🎵",
        MediaFrameType::Video => "📹",
        MediaFrameType::Data => "📊",
    };

    if let Some(info) = get_payload_info(payload_type) {
        println!(
            "  {} PT={:3} | {:5} | {} | {} Hz | RFC: {}",
            media_icon,
            payload_type,
            format!("{:?}", media_type),
            codec_name,
            info.clock_rate,
            info.rfc_reference.as_deref().unwrap_or("N/A")
        );
    } else {
        println!(
            "  {} PT={:3} | {:5} | {} | Unknown (fallback)",
            media_icon,
            payload_type,
            format!("{:?}", media_type),
            codec_name
        );
    }
}
