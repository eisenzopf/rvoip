//! Utility functions for server transport
//!
//! This module contains helper functions used across the server transport implementation.

use crate::api::common::frame::MediaFrameType;

/// Get the frame type based on payload type
///
/// This uses a simple heuristic based on common payload type ranges:
/// - 0-34, 96-98: Audio
/// - 35-50, 99-112: Video
/// - Others: Data
pub fn get_frame_type_from_payload_type(payload_type: u8) -> MediaFrameType {
    match payload_type {
        // Common audio payload types
        0..=34 | 96..=98 => MediaFrameType::Audio,
        
        // Common video payload types
        35..=50 | 99..=112 => MediaFrameType::Video,
        
        // Everything else we'll assume is data
        _ => MediaFrameType::Data,
    }
} 