//! Media types and frame definitions

use bytes::Bytes;

/// Media frame types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaFrameType {
    /// Audio media
    Audio,
    /// Video media
    Video,
    /// Data/application media
    Data,
}

/// Media frame structure
#[derive(Debug, Clone, PartialEq)]
pub struct MediaFrame {
    /// Frame type (audio, video, data)
    pub frame_type: MediaFrameType,
    /// Frame payload data
    pub data: Bytes,
    /// RTP timestamp
    pub timestamp: u32,
    /// RTP sequence number
    pub sequence: u16,
    /// RTP marker bit
    pub marker: bool,
    /// RTP payload type
    pub payload_type: u8,
    /// RTP SSRC
    pub ssrc: u32,
    /// RTP CSRCs
    pub csrcs: Vec<u32>,
}

impl MediaFrame {
    /// Create a new media frame
    pub fn new(
        frame_type: MediaFrameType,
        data: Bytes,
        timestamp: u32,
        sequence: u16,
        payload_type: u8,
        ssrc: u32,
    ) -> Self {
        Self {
            frame_type,
            data,
            timestamp,
            sequence,
            marker: false,
            payload_type,
            ssrc,
            csrcs: Vec::new(),
        }
    }
    
    /// Set the marker bit
    pub fn with_marker(mut self, marker: bool) -> Self {
        self.marker = marker;
        self
    }
    
    /// Set the CSRCs
    pub fn with_csrcs(mut self, csrcs: Vec<u32>) -> Self {
        self.csrcs = csrcs;
        self
    }
    
    /// Get the frame size in bytes
    pub fn size(&self) -> usize {
        self.data.len()
    }
    
    /// Check if the frame is empty
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

/// Media codec information
#[derive(Debug, Clone, PartialEq)]
pub struct MediaCodec {
    /// Codec name (e.g., "PCMU", "H.264")
    pub name: String,
    /// RTP payload type
    pub payload_type: u8,
    /// Clock rate in Hz
    pub clock_rate: u32,
    /// Number of channels (audio only)
    pub channels: Option<u8>,
    /// Additional codec parameters
    pub parameters: std::collections::HashMap<String, String>,
}

impl MediaCodec {
    /// Create a new media codec
    pub fn new(name: String, payload_type: u8, clock_rate: u32) -> Self {
        Self {
            name,
            payload_type,
            clock_rate,
            channels: None,
            parameters: std::collections::HashMap::new(),
        }
    }
    
    /// Set the number of channels (for audio codecs)
    pub fn with_channels(mut self, channels: u8) -> Self {
        self.channels = Some(channels);
        self
    }
    
    /// Add a codec parameter
    pub fn with_parameter(mut self, key: String, value: String) -> Self {
        self.parameters.insert(key, value);
        self
    }
    
    /// Check if this is an audio codec
    pub fn is_audio(&self) -> bool {
        matches!(
            self.name.as_str(),
            "PCMU" | "PCMA" | "G722" | "G729" | "opus" | "AMR" | "AMR-WB"
        )
    }
    
    /// Check if this is a video codec
    pub fn is_video(&self) -> bool {
        matches!(
            self.name.as_str(),
            "H264" | "H.264" | "H265" | "H.265" | "VP8" | "VP9" | "AV1"
        )
    }
}

/// Media stream direction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaDirection {
    /// Send only
    SendOnly,
    /// Receive only
    ReceiveOnly,
    /// Send and receive
    SendReceive,
    /// Inactive
    Inactive,
}

/// Media stream configuration
#[derive(Debug, Clone, PartialEq)]
pub struct MediaStreamConfig {
    /// Media direction
    pub direction: MediaDirection,
    /// Supported codecs
    pub codecs: Vec<MediaCodec>,
    /// Maximum bitrate in bps
    pub max_bitrate: Option<u32>,
    /// Preferred codec
    pub preferred_codec: Option<String>,
}

impl MediaStreamConfig {
    /// Create a new media stream config
    pub fn new(direction: MediaDirection) -> Self {
        Self {
            direction,
            codecs: Vec::new(),
            max_bitrate: None,
            preferred_codec: None,
        }
    }
    
    /// Add a codec to the stream config
    pub fn with_codec(mut self, codec: MediaCodec) -> Self {
        self.codecs.push(codec);
        self
    }
    
    /// Set the maximum bitrate
    pub fn with_max_bitrate(mut self, bitrate: u32) -> Self {
        self.max_bitrate = Some(bitrate);
        self
    }
    
    /// Set the preferred codec
    pub fn with_preferred_codec(mut self, codec: String) -> Self {
        self.preferred_codec = Some(codec);
        self
    }
}