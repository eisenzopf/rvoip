//! RTP Header Extensions API Layer
//!
//! This module provides the API layer integration for RTP header extensions
//! defined in RFC 8285. It allows applications to add, configure, and manage
//! header extensions without directly interfacing with the lower-level packet
//! structures.

use std::collections::HashMap;
use bytes::Bytes;

use crate::packet::extension::{RtpHeaderExtensions, ExtensionElement};
use crate::error::Error;
use crate::Result;

// Re-export the ExtensionFormat enum directly from the packet layer
pub use crate::packet::extension::ExtensionFormat;

// URI constants for common extensions (the actual URIs, not the ID numbers)
pub const AUDIO_LEVEL_URI: &str = "urn:ietf:params:rtp-hdrext:ssrc-audio-level";
pub const VIDEO_ORIENTATION_URI: &str = "urn:3gpp:video-orientation";
pub const TRANSPORT_CC_URI: &str = "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01";
pub const ABS_SEND_TIME_URI: &str = "http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time";
pub const SDES_MID_URI: &str = "urn:ietf:params:rtp-hdrext:sdes:mid";
pub const SDES_RTP_STREAM_ID_URI: &str = "urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id";

/// Create a new RtpHeaderExtensions instance with the specified format
pub fn create_header_extensions(format: ExtensionFormat) -> RtpHeaderExtensions {
    match format {
        ExtensionFormat::OneByte => RtpHeaderExtensions::new_one_byte(),
        ExtensionFormat::TwoByte => RtpHeaderExtensions::new_two_byte(),
        ExtensionFormat::Legacy => {
            // Legacy format needs a profile ID which isn't standard
            // Default to using 0x1000 for legacy, but this isn't ideal
            RtpHeaderExtensions::new_legacy(0x1000) 
        }
    }
}

/// Create an audio level extension data
/// 
/// * `voice_activity` - true if voice activity is detected, false otherwise
/// * `level` - audio level in dB below full scale (0-127)
pub fn create_audio_level_data(voice_activity: bool, level: u8) -> Vec<u8> {
    // Clamp level to 0-127
    let level = level.min(127);
    
    // For audio level, the format is:
    // - Bit 7 (MSB): Voice activity flag (0 = active, 1 = inactive)
    // - Bits 0-6: Level (0-127 dB)
    let data = if voice_activity {
        // For active voice, the MSB is 0
        vec![level]
    } else {
        // For inactive voice, the MSB is 1
        vec![level | 0x80]
    };
    
    data
}

/// Parse audio level extension data
/// 
/// Returns a tuple of (voice_activity, level) if successful
pub fn parse_audio_level_data(data: &[u8]) -> Option<(bool, u8)> {
    if data.is_empty() {
        return None;
    }
    
    let byte = data[0];
    
    // Extract voice activity flag (0 = active, 1 = inactive)
    let voice_activity = (byte & 0x80) == 0;
    
    // Extract level (0-127 dB)
    let level = byte & 0x7F;
    
    Some((voice_activity, level))
}

/// Create a video orientation extension data
///
/// * `camera_front_facing` - true if camera is front-facing, false otherwise
/// * `camera_flipped` - true if camera is flipped, false otherwise
/// * `rotation` - rotation in degrees (0, 90, 180, or 270)
pub fn create_video_orientation_data(
    camera_front_facing: bool, 
    camera_flipped: bool,
    rotation: u16
) -> Vec<u8> {
    // Normalize degrees to 0, 90, 180, or 270
    let normalized_degrees = match rotation {
        0..=44 => 0,
        45..=134 => 90,
        135..=224 => 180,
        _ => 270,
    };
    
    // Create orientation byte
    let mut orientation_byte = 0;
    
    // Bit 0: Camera (0=back/1=front)
    if camera_front_facing {
        orientation_byte |= 0x01;
    }
    
    // Bit 1: Flip (0=normal/1=flipped)
    if camera_flipped {
        orientation_byte |= 0x02;
    }
    
    // Bits 2-3: Rotation
    match normalized_degrees {
        90 => orientation_byte |= 0x04,   // 01 << 2
        180 => orientation_byte |= 0x08,  // 10 << 2
        270 => orientation_byte |= 0x0C,  // 11 << 2
        _ => {}                           // 00 << 2 (0 degrees)
    }
    
    vec![orientation_byte]
}

/// Parse video orientation extension data
///
/// Returns a tuple of (camera_front_facing, camera_flipped, rotation) if successful
pub fn parse_video_orientation_data(data: &[u8]) -> Option<(bool, bool, u16)> {
    if data.is_empty() {
        return None;
    }
    
    let byte = data[0];
    
    // Bit 0: Camera (0=back/1=front)
    let camera_front_facing = (byte & 0x01) != 0;
    
    // Bit 1: Flip (0=normal/1=flipped)
    let camera_flipped = (byte & 0x02) != 0;
    
    // Bits 2-3: Rotation
    let rotation_bits = (byte >> 2) & 0x03;
    let rotation_degrees = match rotation_bits {
        0 => 0,
        1 => 90,
        2 => 180,
        3 => 270,
        _ => unreachable!(),
    };
    
    Some((camera_front_facing, camera_flipped, rotation_degrees))
}

/// Create transport-cc extension data
///
/// * `sequence_number` - transport-wide sequence number
pub fn create_transport_cc_data(sequence_number: u16) -> Vec<u8> {
    // Transport-CC sequence number is sent in network byte order (big-endian)
    vec![(sequence_number >> 8) as u8, (sequence_number & 0xFF) as u8]
}

/// Parse transport-cc extension data
///
/// Returns the transport-wide sequence number if successful
pub fn parse_transport_cc_data(data: &[u8]) -> Option<u16> {
    if data.len() < 2 {
        return None;
    }
    
    // Transport-CC sequence number is in network byte order (big-endian)
    let sequence_number = ((data[0] as u16) << 8) | (data[1] as u16);
    
    Some(sequence_number)
}

/// Create WebRTC standard extension URIs mapping
///
/// This creates a mapping of standard WebRTC extension IDs to their URIs
pub fn create_webrtc_extension_mappings() -> HashMap<u8, String> {
    let mut mappings = HashMap::new();
    
    // Common WebRTC extension mappings
    mappings.insert(1, AUDIO_LEVEL_URI.to_string());
    mappings.insert(3, VIDEO_ORIENTATION_URI.to_string());
    mappings.insert(5, TRANSPORT_CC_URI.to_string());
    mappings.insert(10, ABS_SEND_TIME_URI.to_string());
    mappings.insert(13, SDES_MID_URI.to_string());
    mappings.insert(14, SDES_RTP_STREAM_ID_URI.to_string());
    
    mappings
}

/// Find the ID for a specific extension URI in the mappings
pub fn find_extension_id(mappings: &HashMap<u8, String>, uri: &str) -> Option<u8> {
    for (id, mapped_uri) in mappings {
        if mapped_uri == uri {
            return Some(*id);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_audio_level_data() {
        // Test active voice
        let active_data = create_audio_level_data(true, 30);
        assert_eq!(active_data.len(), 1);
        assert_eq!(active_data[0], 30);
        
        // Test inactive voice
        let inactive_data = create_audio_level_data(false, 60);
        assert_eq!(inactive_data.len(), 1);
        assert_eq!(inactive_data[0], 0x80 | 60);
        
        // Test parsing
        let (active, level) = parse_audio_level_data(&active_data).unwrap();
        assert_eq!(active, true);
        assert_eq!(level, 30);
        
        let (active, level) = parse_audio_level_data(&inactive_data).unwrap();
        assert_eq!(active, false);
        assert_eq!(level, 60);
    }
    
    #[test]
    fn test_video_orientation_data() {
        // Test front facing camera, flipped, 90 degrees
        let data = create_video_orientation_data(true, true, 90);
        assert_eq!(data.len(), 1);
        assert_eq!(data[0], 0x07); // 0000 0111
        
        // Test back facing camera, not flipped, 180 degrees
        let data = create_video_orientation_data(false, false, 180);
        assert_eq!(data.len(), 1);
        assert_eq!(data[0], 0x08); // 0000 1000
        
        // Test parsing
        let (front, flipped, rotation) = parse_video_orientation_data(&data).unwrap();
        assert_eq!(front, false);
        assert_eq!(flipped, false);
        assert_eq!(rotation, 180);
    }
    
    #[test]
    fn test_transport_cc_data() {
        // Test sequence number 1000
        let data = create_transport_cc_data(1000);
        assert_eq!(data.len(), 2);
        assert_eq!(data[0], 3); // 1000 >> 8 = 3
        assert_eq!(data[1], 232); // 1000 & 0xFF = 232
        
        // Test parsing
        let sequence_number = parse_transport_cc_data(&data).unwrap();
        assert_eq!(sequence_number, 1000);
    }
    
    #[test]
    fn test_webrtc_extension_mappings() {
        let mappings = create_webrtc_extension_mappings();
        
        // Verify standard mappings
        assert_eq!(mappings.get(&1).unwrap(), AUDIO_LEVEL_URI);
        assert_eq!(mappings.get(&3).unwrap(), VIDEO_ORIENTATION_URI);
        assert_eq!(mappings.get(&5).unwrap(), TRANSPORT_CC_URI);
        
        // Test finding an ID by URI
        assert_eq!(find_extension_id(&mappings, AUDIO_LEVEL_URI), Some(1));
        assert_eq!(find_extension_id(&mappings, VIDEO_ORIENTATION_URI), Some(3));
        assert_eq!(find_extension_id(&mappings, "non-existent-uri"), None);
    }
} 