//! Common video codec interfaces and types
//!
//! This module defines the base interfaces and types used by all video codecs.

use std::sync::Arc;
use bytes::Bytes;

use crate::error::Result;

/// Video frame types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoFrameType {
    /// Key frame (I-frame)
    Key,
    /// Delta frame (P-frame)
    Delta,
    /// Bi-directionally predicted frame (B-frame)
    Bidirectional,
}

/// Video resolution
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolution {
    /// Width in pixels
    pub width: u32,
    /// Height in pixels
    pub height: u32,
}

impl Resolution {
    /// Create a new resolution
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
    
    /// Standard video resolution constants
    pub mod standard {
        use super::Resolution;
        
        /// QCIF (176x144)
        pub const QCIF: Resolution = Resolution { width: 176, height: 144 };
        /// CIF (352x288)
        pub const CIF: Resolution = Resolution { width: 352, height: 288 };
        /// 4CIF (704x576)
        pub const FOUR_CIF: Resolution = Resolution { width: 704, height: 576 };
        /// 720p HD (1280x720)
        pub const HD_720P: Resolution = Resolution { width: 1280, height: 720 };
        /// 1080p Full HD (1920x1080)
        pub const FULL_HD_1080P: Resolution = Resolution { width: 1920, height: 1080 };
    }
}

/// Video frame rate
#[derive(Debug, Clone, Copy)]
pub struct FrameRate {
    /// Frames per second
    pub fps: f32,
}

impl FrameRate {
    /// Create a new frame rate
    pub fn new(fps: f32) -> Self {
        Self { fps }
    }
    
    /// Standard video frame rates
    pub mod standard {
        use super::FrameRate;
        
        /// 15 fps
        pub const FPS_15: FrameRate = FrameRate { fps: 15.0 };
        /// 24 fps
        pub const FPS_24: FrameRate = FrameRate { fps: 24.0 };
        /// 25 fps (PAL)
        pub const FPS_25: FrameRate = FrameRate { fps: 25.0 };
        /// 30 fps
        pub const FPS_30: FrameRate = FrameRate { fps: 30.0 };
        /// 60 fps
        pub const FPS_60: FrameRate = FrameRate { fps: 60.0 };
    }
}

/// Raw video frame
#[derive(Debug, Clone)]
pub struct VideoFrame {
    /// Raw frame data
    pub data: Bytes,
    /// Frame resolution
    pub resolution: Resolution,
    /// Frame type (key, delta, etc)
    pub frame_type: VideoFrameType,
    /// Timestamp in milliseconds
    pub timestamp_ms: u64,
}

/// Video codec interface
pub trait VideoCodec: Send + Sync {
    /// Encode a raw video frame
    fn encode_frame(&self, frame: &VideoFrame) -> Result<Bytes>;
    
    /// Decode a compressed video frame
    fn decode_frame(&self, data: &[u8], timestamp_ms: u64) -> Result<VideoFrame>;
    
    /// Get the codec's maximum supported resolution
    fn max_resolution(&self) -> Resolution;
    
    /// Get the codec's default bitrate
    fn default_bitrate(&self) -> u32;
    
    /// Set the target bitrate
    fn set_bitrate(&mut self, bitrate: u32) -> Result<()>;
    
    /// Request a key frame
    fn request_key_frame(&mut self) -> Result<()>;
    
    /// Clone the codec as a boxed trait object
    fn box_clone(&self) -> Box<dyn VideoCodec>;
} 