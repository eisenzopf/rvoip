//! RTP Packetizer
//!
//! This module provides the RTP packetization functionality for
//! converting media data to RTP packets.

use bytes::{Bytes, BytesMut};
use std::sync::Arc;
use tracing::{debug, trace};

use rvoip_rtp_core::{
    packet::{RtpPacket, RtpPacketBuilder},
    payload::PayloadType,
};

use crate::{AudioBuffer, AudioFormat, codec::Codec, error::{Error, Result}};

/// Configuration for RTP packetization
#[derive(Debug, Clone)]
pub struct PacketizerConfig {
    /// SSRC identifier
    pub ssrc: u32,
    /// Payload type
    pub payload_type: u8,
    /// Initial sequence number
    pub initial_seq: u16,
    /// Initial timestamp
    pub initial_ts: u32,
    /// Whether to set marker bit on first packet
    pub set_marker: bool,
    /// Clock rate in Hz
    pub clock_rate: u32,
}

impl Default for PacketizerConfig {
    fn default() -> Self {
        Self {
            ssrc: rand::random::<u32>(),
            payload_type: 0, // Default to PCMU
            initial_seq: rand::random::<u16>(),
            initial_ts: rand::random::<u32>(),
            set_marker: true,
            clock_rate: 8000, // Default to 8kHz
        }
    }
}

/// RTP packetizer that converts media frames into RTP packets
pub struct Packetizer {
    /// Configuration
    config: PacketizerConfig,
    /// Current sequence number
    sequence: u16,
    /// Current timestamp
    timestamp: u32,
    /// Codec to use for packetization
    codec: Option<Arc<dyn Codec>>,
    /// Whether this is the first packet
    first_packet: bool,
}

impl Packetizer {
    /// Create a new packetizer with the given configuration
    pub fn new(config: PacketizerConfig) -> Self {
        Self {
            sequence: config.initial_seq,
            timestamp: config.initial_ts,
            config,
            codec: None,
            first_packet: true,
        }
    }
    
    /// Create a new packetizer with default configuration
    pub fn new_default() -> Self {
        Self::new(PacketizerConfig::default())
    }
    
    /// Set the codec to use for packetization
    pub fn set_codec(&mut self, codec: Arc<dyn Codec>) {
        self.codec = Some(codec);
    }
    
    /// Set the payload type
    pub fn set_payload_type(&mut self, payload_type: u8) {
        self.config.payload_type = payload_type;
    }
    
    /// Set the SSRC
    pub fn set_ssrc(&mut self, ssrc: u32) {
        self.config.ssrc = ssrc;
    }
    
    /// Set the clock rate
    pub fn set_clock_rate(&mut self, clock_rate: u32) {
        self.config.clock_rate = clock_rate;
    }
    
    /// Get the current sequence number
    pub fn sequence(&self) -> u16 {
        self.sequence
    }
    
    /// Get the current timestamp
    pub fn timestamp(&self) -> u32 {
        self.timestamp
    }
    
    /// Get the SSRC
    pub fn ssrc(&self) -> u32 {
        self.config.ssrc
    }
    
    /// Packetize an audio buffer into RTP packets
    /// 
    /// Returns a vector of RTP packets.
    pub fn packetize_audio(&mut self, buffer: &AudioBuffer) -> Result<Vec<RtpPacket>> {
        // If we have a codec, use it for packetization
        if let Some(codec) = &self.codec {
            return self.packetize_with_codec(buffer, codec.as_ref());
        }
        
        // Otherwise, use simple packetization (raw PCM)
        let mut packets = Vec::new();
        
        // Calculate timestamp increment based on sample rate
        let samples_per_frame = buffer.samples();
        let timestamp_increment = samples_per_frame as u32;
        
        // Create RTP packet
        let mut builder = RtpPacketBuilder::new()
            .with_version(2)
            .with_padding(false)
            .with_extension(false)
            .with_marker(self.first_packet && self.config.set_marker)
            .with_payload_type(PayloadType::new(self.config.payload_type))
            .with_sequence(self.sequence)
            .with_timestamp(self.timestamp)
            .with_ssrc(self.config.ssrc);
        
        // Set payload
        builder = builder.with_payload(Bytes::from(buffer.data.clone()));
        
        // Build packet
        let packet = builder.build();
        packets.push(packet);
        
        // Update state
        self.sequence = self.sequence.wrapping_add(1);
        self.timestamp = self.timestamp.wrapping_add(timestamp_increment);
        self.first_packet = false;
        
        trace!("Packetized audio: seq={}, ts={}, len={}", 
              self.sequence, self.timestamp, buffer.data.len());
        
        Ok(packets)
    }
    
    /// Packetize an audio buffer using a specific codec
    fn packetize_with_codec(&mut self, buffer: &AudioBuffer, codec: &dyn Codec) -> Result<Vec<RtpPacket>> {
        // Encode buffer with codec
        let encoded = codec.encode(buffer)?;
        
        if encoded.is_empty() {
            return Err(Error::EncodingFailed("Codec produced empty output".into()));
        }
        
        // Calculate timestamp increment based on sample rate and codec frame size
        // For most codecs, this is the number of samples in the frame
        let samples_per_frame = buffer.samples();
        let timestamp_increment = samples_per_frame as u32;
        
        let mut packets = Vec::new();
        
        // If the codec produces multiple frames, create multiple packets
        for (i, frame) in encoded.iter().enumerate() {
            // Create RTP packet
            let marker = i == 0 && self.first_packet && self.config.set_marker;
            
            let mut builder = RtpPacketBuilder::new()
                .with_version(2)
                .with_padding(false)
                .with_extension(false)
                .with_marker(marker)
                .with_payload_type(PayloadType::new(self.config.payload_type))
                .with_sequence(self.sequence)
                .with_timestamp(self.timestamp)
                .with_ssrc(self.config.ssrc);
            
            // Set payload
            builder = builder.with_payload(Bytes::from(frame.clone()));
            
            // Build packet
            let packet = builder.build();
            packets.push(packet);
            
            // Update sequence number for each packet
            self.sequence = self.sequence.wrapping_add(1);
        }
        
        // Update timestamp by samples for next buffer
        // (only increment timestamp once per buffer, regardless of packet count)
        self.timestamp = self.timestamp.wrapping_add(timestamp_increment);
        self.first_packet = false;
        
        debug!("Packetized {} frames with codec: seq={}, ts={}", 
              encoded.len(), self.sequence.wrapping_sub(1), self.timestamp);
        
        Ok(packets)
    }
    
    /// Reset the packetizer state
    pub fn reset(&mut self) {
        self.sequence = self.config.initial_seq;
        self.timestamp = self.config.initial_ts;
        self.first_packet = true;
    }
}

/// Convert PCM samples to timestamp units based on clock rate
pub fn pcm_to_timestamp(sample_count: usize, sample_rate: u32, clock_rate: u32) -> u32 {
    (sample_count as u64 * clock_rate as u64 / sample_rate as u64) as u32
}

/// Convert timestamp units to PCM samples based on clock rate
pub fn timestamp_to_pcm(timestamp: u32, clock_rate: u32, sample_rate: u32) -> usize {
    (timestamp as u64 * sample_rate as u64 / clock_rate as u64) as usize
} 