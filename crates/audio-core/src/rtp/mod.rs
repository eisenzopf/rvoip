//! RTP integration
//!
//! This module provides RTP payload encoding and decoding.

use crate::types::{AudioFrame, AudioFormat};
use crate::codec::{CodecType, CodecConfig};
use crate::error::AudioError;

/// RTP payload handling for audio codecs
pub struct RtpPayloadHandler {
    codec: CodecType,
    payload_type: u8,
    ssrc: u32,
    sequence_number: u16,
    timestamp: u32,
    sample_rate: u32,
}

/// RTP packet structure
#[derive(Debug, Clone)]
pub struct RtpPacket {
    pub version: u8,
    pub padding: bool,
    pub extension: bool,
    pub marker: bool,
    pub payload_type: u8,
    pub sequence_number: u16,
    pub timestamp: u32,
    pub ssrc: u32,
    pub payload: Vec<u8>,
}

/// RTP payload format for different codecs
pub trait RtpPayloadFormat {
    /// Pack encoded audio data into RTP payload
    fn pack_payload(&self, data: &[u8]) -> Vec<u8>;
    
    /// Unpack RTP payload to encoded audio data
    fn unpack_payload(&self, payload: &[u8]) -> Result<Vec<u8>, AudioError>;
    
    /// Get payload type
    fn payload_type(&self) -> u8;
    
    /// Get samples per packet
    fn samples_per_packet(&self) -> usize;
}

impl RtpPayloadHandler {
    /// Create a new RTP payload handler
    pub fn new(codec: CodecType, ssrc: u32) -> Self {
        Self {
            codec,
            payload_type: codec.payload_type(),
            ssrc,
            sequence_number: 0,
            timestamp: 0,
            sample_rate: codec.default_sample_rate(),
        }
    }

    /// Create RTP packet from encoded audio data
    pub fn create_packet(&mut self, encoded_data: &[u8], marker: bool) -> RtpPacket {
        let packet = RtpPacket {
            version: 2,
            padding: false,
            extension: false,
            marker,
            payload_type: self.payload_type,
            sequence_number: self.sequence_number,
            timestamp: self.timestamp,
            ssrc: self.ssrc,
            payload: encoded_data.to_vec(),
        };

        // Update sequence number and timestamp
        self.sequence_number = self.sequence_number.wrapping_add(1);
        self.timestamp = self.timestamp.wrapping_add(self.samples_per_packet() as u32);

        packet
    }

    /// Parse RTP packet from raw data
    pub fn parse_packet(&self, data: &[u8]) -> Result<RtpPacket, AudioError> {
        if data.len() < 12 {
            return Err(AudioError::invalid_format("RTP packet too short".to_string()));
        }

        let version = (data[0] >> 6) & 0x03;
        let padding = (data[0] & 0x20) != 0;
        let extension = (data[0] & 0x10) != 0;
        let marker = (data[1] & 0x80) != 0;
        let payload_type = data[1] & 0x7F;
        let sequence_number = u16::from_be_bytes([data[2], data[3]]);
        let timestamp = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
        let ssrc = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);

        let payload = data[12..].to_vec();

        Ok(RtpPacket {
            version,
            padding,
            extension,
            marker,
            payload_type,
            sequence_number,
            timestamp,
            ssrc,
            payload,
        })
    }

    /// Serialize RTP packet to raw data
    pub fn serialize_packet(&self, packet: &RtpPacket) -> Vec<u8> {
        let mut data = Vec::with_capacity(12 + packet.payload.len());

        // First byte: V(2) + P(1) + X(1) + CC(4)
        let byte0 = (packet.version << 6) | 
                   (if packet.padding { 0x20 } else { 0 }) |
                   (if packet.extension { 0x10 } else { 0 });
        data.push(byte0);

        // Second byte: M(1) + PT(7)
        let byte1 = (if packet.marker { 0x80 } else { 0 }) | packet.payload_type;
        data.push(byte1);

        // Sequence number
        data.extend_from_slice(&packet.sequence_number.to_be_bytes());

        // Timestamp
        data.extend_from_slice(&packet.timestamp.to_be_bytes());

        // SSRC
        data.extend_from_slice(&packet.ssrc.to_be_bytes());

        // Payload
        data.extend_from_slice(&packet.payload);

        data
    }

    /// Get samples per packet for current codec
    pub fn samples_per_packet(&self) -> usize {
        match self.codec {
            CodecType::G711Pcmu | CodecType::G711Pcma => 160, // 20ms at 8kHz
            CodecType::G722 => 160, // 20ms at 16kHz (but transmitted as 8kHz)
            CodecType::G729 => 80, // 10ms at 8kHz
            CodecType::Opus => {
                // Opus frame size depends on sample rate
                match self.sample_rate {
                    8000 => 160,
                    16000 => 320,
                    48000 => 960,
                    _ => 320,
                }
            }
        }
    }

    /// Set sample rate (for dynamic codecs like Opus)
    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate;
    }

    /// Get current timestamp
    pub fn current_timestamp(&self) -> u32 {
        self.timestamp
    }

    /// Get current sequence number
    pub fn current_sequence_number(&self) -> u16 {
        self.sequence_number
    }

    /// Reset sequence number and timestamp
    pub fn reset(&mut self) {
        self.sequence_number = 0;
        self.timestamp = 0;
    }
}

/// G.711 RTP payload format
pub struct G711PayloadFormat {
    payload_type: u8,
}

impl G711PayloadFormat {
    pub fn new(is_pcmu: bool) -> Self {
        Self {
            payload_type: if is_pcmu { 0 } else { 8 },
        }
    }
}

impl RtpPayloadFormat for G711PayloadFormat {
    fn pack_payload(&self, data: &[u8]) -> Vec<u8> {
        // G.711 payload is just the raw encoded bytes
        data.to_vec()
    }

    fn unpack_payload(&self, payload: &[u8]) -> Result<Vec<u8>, AudioError> {
        // G.711 payload is just the raw encoded bytes
        Ok(payload.to_vec())
    }

    fn payload_type(&self) -> u8 {
        self.payload_type
    }

    fn samples_per_packet(&self) -> usize {
        160 // 20ms at 8kHz
    }
}

/// G.722 RTP payload format
pub struct G722PayloadFormat;

impl RtpPayloadFormat for G722PayloadFormat {
    fn pack_payload(&self, data: &[u8]) -> Vec<u8> {
        // G.722 payload is just the raw encoded bytes
        data.to_vec()
    }

    fn unpack_payload(&self, payload: &[u8]) -> Result<Vec<u8>, AudioError> {
        // G.722 payload is just the raw encoded bytes
        Ok(payload.to_vec())
    }

    fn payload_type(&self) -> u8 {
        9
    }

    fn samples_per_packet(&self) -> usize {
        160 // 20ms at 16kHz, but RTP timestamp increments at 8kHz rate
    }
}

/// G.729 RTP payload format
pub struct G729PayloadFormat;

impl RtpPayloadFormat for G729PayloadFormat {
    fn pack_payload(&self, data: &[u8]) -> Vec<u8> {
        // G.729 payload is just the raw encoded bytes (10 bytes per frame)
        data.to_vec()
    }

    fn unpack_payload(&self, payload: &[u8]) -> Result<Vec<u8>, AudioError> {
        // G.729 payload is just the raw encoded bytes
        Ok(payload.to_vec())
    }

    fn payload_type(&self) -> u8 {
        18
    }

    fn samples_per_packet(&self) -> usize {
        80 // 10ms at 8kHz
    }
}

/// Opus RTP payload format
pub struct OpusPayloadFormat {
    sample_rate: u32,
}

impl OpusPayloadFormat {
    pub fn new(sample_rate: u32) -> Self {
        Self { sample_rate }
    }
}

impl RtpPayloadFormat for OpusPayloadFormat {
    fn pack_payload(&self, data: &[u8]) -> Vec<u8> {
        // Opus payload is the raw encoded frame
        data.to_vec()
    }

    fn unpack_payload(&self, payload: &[u8]) -> Result<Vec<u8>, AudioError> {
        // Opus payload is the raw encoded frame
        Ok(payload.to_vec())
    }

    fn payload_type(&self) -> u8 {
        111 // Dynamic payload type for Opus
    }

    fn samples_per_packet(&self) -> usize {
        // 20ms frame at current sample rate
        (self.sample_rate * 20 / 1000) as usize
    }
}

/// Jitter buffer for handling packet reordering and timing
pub struct JitterBuffer {
    buffer: std::collections::BTreeMap<u16, RtpPacket>,
    max_size: usize,
    expected_seq: u16,
    last_timestamp: u32,
}

impl JitterBuffer {
    /// Create a new jitter buffer
    pub fn new(max_size: usize) -> Self {
        Self {
            buffer: std::collections::BTreeMap::new(),
            max_size,
            expected_seq: 0,
            last_timestamp: 0,
        }
    }

    /// Add packet to buffer
    pub fn add_packet(&mut self, packet: RtpPacket) {
        // Remove old packets if buffer is full
        if self.buffer.len() >= self.max_size {
            if let Some((&oldest_seq, _)) = self.buffer.iter().next() {
                self.buffer.remove(&oldest_seq);
            }
        }

        self.buffer.insert(packet.sequence_number, packet);
    }

    /// Get next packet in sequence
    pub fn get_next_packet(&mut self) -> Option<RtpPacket> {
        if let Some(packet) = self.buffer.remove(&self.expected_seq) {
            self.expected_seq = self.expected_seq.wrapping_add(1);
            self.last_timestamp = packet.timestamp;
            Some(packet)
        } else {
            None
        }
    }

    /// Check if buffer has packets ready
    pub fn has_packets(&self) -> bool {
        self.buffer.contains_key(&self.expected_seq)
    }

    /// Get buffer statistics
    pub fn stats(&self) -> JitterBufferStats {
        JitterBufferStats {
            buffer_size: self.buffer.len(),
            max_size: self.max_size,
            expected_seq: self.expected_seq,
            last_timestamp: self.last_timestamp,
        }
    }

    /// Reset buffer
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.expected_seq = 0;
        self.last_timestamp = 0;
    }
}

/// Jitter buffer statistics
#[derive(Debug, Clone)]
pub struct JitterBufferStats {
    pub buffer_size: usize,
    pub max_size: usize,
    pub expected_seq: u16,
    pub last_timestamp: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rtp_payload_handler_creation() {
        let handler = RtpPayloadHandler::new(CodecType::G711Pcmu, 0x12345678);
        assert_eq!(handler.codec, CodecType::G711Pcmu);
        assert_eq!(handler.payload_type, 0);
        assert_eq!(handler.ssrc, 0x12345678);
    }

    #[test]
    fn test_rtp_packet_creation() {
        let mut handler = RtpPayloadHandler::new(CodecType::G711Pcmu, 0x12345678);
        let data = vec![0x80, 0x01, 0x02, 0x03];
        
        let packet = handler.create_packet(&data, false);
        assert_eq!(packet.version, 2);
        assert_eq!(packet.payload_type, 0);
        assert_eq!(packet.sequence_number, 0);
        assert_eq!(packet.timestamp, 0);
        assert_eq!(packet.ssrc, 0x12345678);
        assert_eq!(packet.payload, data);
    }

    #[test]
    fn test_rtp_packet_serialization() {
        let packet = RtpPacket {
            version: 2,
            padding: false,
            extension: false,
            marker: false,
            payload_type: 0,
            sequence_number: 0x1234,
            timestamp: 0x56789ABC,
            ssrc: 0xDEADBEEF,
            payload: vec![0x80, 0x01, 0x02, 0x03],
        };

        let handler = RtpPayloadHandler::new(CodecType::G711Pcmu, 0xDEADBEEF);
        let serialized = handler.serialize_packet(&packet);
        
        assert_eq!(serialized.len(), 16); // 12 byte header + 4 byte payload
        assert_eq!(serialized[0], 0x80); // Version 2
        assert_eq!(serialized[1], 0x00); // Payload type 0
        assert_eq!(serialized[2], 0x12); // Sequence number high byte
        assert_eq!(serialized[3], 0x34); // Sequence number low byte
    }

    #[test]
    fn test_rtp_packet_parsing() {
        let data = vec![
            0x80, 0x00, 0x12, 0x34,  // Version, PT, Sequence
            0x56, 0x78, 0x9A, 0xBC,  // Timestamp
            0xDE, 0xAD, 0xBE, 0xEF,  // SSRC
            0x80, 0x01, 0x02, 0x03,  // Payload
        ];

        let handler = RtpPayloadHandler::new(CodecType::G711Pcmu, 0xDEADBEEF);
        let packet = handler.parse_packet(&data).unwrap();
        
        assert_eq!(packet.version, 2);
        assert_eq!(packet.payload_type, 0);
        assert_eq!(packet.sequence_number, 0x1234);
        assert_eq!(packet.timestamp, 0x56789ABC);
        assert_eq!(packet.ssrc, 0xDEADBEEF);
        assert_eq!(packet.payload, vec![0x80, 0x01, 0x02, 0x03]);
    }

    #[test]
    fn test_g711_payload_format() {
        let format = G711PayloadFormat::new(true); // PCMU
        let data = vec![0x80, 0x01, 0x02, 0x03];
        
        let packed = format.pack_payload(&data);
        assert_eq!(packed, data);
        
        let unpacked = format.unpack_payload(&packed).unwrap();
        assert_eq!(unpacked, data);
        
        assert_eq!(format.payload_type(), 0);
        assert_eq!(format.samples_per_packet(), 160);
    }

    #[test]
    fn test_g729_payload_format() {
        let format = G729PayloadFormat;
        let data = vec![0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01, 0x00, 0xFF]; // 10 bytes for G.729
        
        let packed = format.pack_payload(&data);
        assert_eq!(packed, data);
        
        let unpacked = format.unpack_payload(&packed).unwrap();
        assert_eq!(unpacked, data);
        
        assert_eq!(format.payload_type(), 18);
        assert_eq!(format.samples_per_packet(), 80);
    }

    #[test]
    fn test_jitter_buffer() {
        let mut buffer = JitterBuffer::new(10);
        
        // Add packet with sequence 0
        let packet1 = RtpPacket {
            version: 2,
            padding: false,
            extension: false,
            marker: false,
            payload_type: 0,
            sequence_number: 0,
            timestamp: 0,
            ssrc: 0x12345678,
            payload: vec![0x01],
        };
        buffer.add_packet(packet1);
        
        // Should be able to get packet 0
        assert!(buffer.has_packets());
        let retrieved = buffer.get_next_packet().unwrap();
        assert_eq!(retrieved.sequence_number, 0);
        assert_eq!(retrieved.payload, vec![0x01]);
        
        // Should not have more packets
        assert!(!buffer.has_packets());
    }

    #[test]
    fn test_samples_per_packet() {
        let handler_g711 = RtpPayloadHandler::new(CodecType::G711Pcmu, 0x12345678);
        assert_eq!(handler_g711.samples_per_packet(), 160);
        
        let handler_g722 = RtpPayloadHandler::new(CodecType::G722, 0x12345678);
        assert_eq!(handler_g722.samples_per_packet(), 160);
        
        let handler_g729 = RtpPayloadHandler::new(CodecType::G729, 0x12345678);
        assert_eq!(handler_g729.samples_per_packet(), 80);
        
        let mut handler_opus = RtpPayloadHandler::new(CodecType::Opus, 0x12345678);
        handler_opus.set_sample_rate(48000);
        assert_eq!(handler_opus.samples_per_packet(), 960);
    }
} 