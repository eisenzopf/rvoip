//! RTP payload encoder for audio frames
//! 
//! This module handles encoding PCM audio frames to G.711 formats for RTP transmission.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU16, AtomicU32, Ordering};
use std::time::Instant;

use crate::api::types::{SessionId, AudioFrame};
use tracing::{debug, trace, warn};

/// G.711 encoding results
#[derive(Debug)]
pub struct EncodedPayload {
    /// The encoded G.711 payload
    pub data: Vec<u8>,
    /// The payload type (0 for PCMU, 8 for PCMA)
    pub payload_type: u8,
    /// RTP timestamp for this packet
    pub timestamp: u32,
    /// RTP sequence number for this packet
    pub sequence_number: u16,
}

/// RTP payload encoder for converting PCM audio to G.711
pub struct RtpPayloadEncoder {
    /// Session-specific encoder state
    sessions: HashMap<SessionId, SessionEncoderState>,
    /// Statistics
    packets_encoded: u64,
    encode_errors: u64,
}

/// Per-session encoder state
struct SessionEncoderState {
    /// Current RTP timestamp
    timestamp: AtomicU32,
    /// Current sequence number
    sequence_number: AtomicU16,
    /// Start time for timestamp calculation
    start_time: Instant,
    /// Payload type for this session (0 = PCMU, 8 = PCMA)
    payload_type: u8,
}

impl RtpPayloadEncoder {
    /// Create a new RTP payload encoder
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            packets_encoded: 0,
            encode_errors: 0,
        }
    }
    
    /// Initialize encoder state for a session
    pub fn init_session(&mut self, session_id: SessionId, payload_type: u8) {
        let state = SessionEncoderState {
            timestamp: AtomicU32::new(rand::random::<u32>()),
            sequence_number: AtomicU16::new(rand::random::<u16>()),
            start_time: Instant::now(),
            payload_type,
        };
        
        self.sessions.insert(session_id.clone(), state);
        debug!("Initialized encoder for session {} with PT {}", session_id, payload_type);
    }
    
    /// Encode an audio frame to G.711
    pub fn encode_audio_frame(&mut self, session_id: &SessionId, frame: &AudioFrame) -> Result<EncodedPayload, String> {
        // Get session state
        let state = self.sessions.get(session_id)
            .ok_or_else(|| format!("No encoder state for session {}", session_id))?;
        
        // Check sample rate
        if frame.sample_rate != 8000 {
            self.encode_errors += 1;
            return Err(format!("Unsupported sample rate: {} (only 8000 Hz supported)", frame.sample_rate));
        }
        
        // Encode based on payload type
        let encoded_data = match state.payload_type {
            0 => self.encode_pcmu(&frame.samples),
            8 => self.encode_pcma(&frame.samples),
            _ => {
                self.encode_errors += 1;
                return Err(format!("Unsupported payload type: {}", state.payload_type));
            }
        };
        
        // Calculate RTP timestamp (8000 Hz clock rate)
        let timestamp = state.timestamp.fetch_add(frame.samples.len() as u32, Ordering::Relaxed);
        
        // Get next sequence number
        let sequence_number = state.sequence_number.fetch_add(1, Ordering::Relaxed);
        
        self.packets_encoded += 1;
        
        trace!("Encoded {} PCM samples to {} G.711 bytes for session {}", 
               frame.samples.len(), encoded_data.len(), session_id);
        
        Ok(EncodedPayload {
            data: encoded_data,
            payload_type: state.payload_type,
            timestamp,
            sequence_number,
        })
    }
    
    /// Encode PCM samples to G.711 μ-law
    fn encode_pcmu(&self, samples: &[i16]) -> Vec<u8> {
        samples.iter().map(|&sample| {
            // G.711 μ-law encoding algorithm (ITU-T G.711)
            let mut s = sample;
            let sign = if s < 0 {
                s = -s;
                0x80
            } else {
                0x00
            };
            
            // Add bias
            s += 0x84;
            
            // Find position of MSB
            let mut exponent = 7;
            let mut mask = 0x4000;
            
            while exponent > 0 && (s & mask) == 0 {
                exponent -= 1;
                mask >>= 1;
            }
            
            // Extract mantissa
            let mantissa = if exponent > 0 {
                ((s >> (exponent + 3)) & 0x0F) as u8
            } else {
                ((s >> 4) & 0x0F) as u8
            };
            
            // Combine sign, exponent, and mantissa
            // μ-law is inverted for transmission
            !(sign | (exponent << 4) | mantissa)
        }).collect()
    }
    
    /// Encode PCM samples to G.711 A-law
    fn encode_pcma(&self, samples: &[i16]) -> Vec<u8> {
        samples.iter().map(|&sample| {
            // G.711 A-law encoding algorithm (ITU-T G.711)
            let mut s = sample;
            let sign = if s < 0 {
                s = -s;
                0x80
            } else {
                0x00
            };
            
            // Find position of MSB
            let mut exponent = 7;
            let mut mask = 0x4000;
            
            while exponent > 0 && (s & mask) == 0 {
                exponent -= 1;
                mask >>= 1;
            }
            
            // Extract mantissa
            let mantissa = if exponent > 0 {
                ((s >> (exponent + 3)) & 0x0F) as u8
            } else {
                ((s >> 4) & 0x0F) as u8
            };
            
            // Combine sign, exponent, and mantissa
            // A-law uses XOR with 0x55 for transmission
            (sign | (exponent << 4) | mantissa) ^ 0x55
        }).collect()
    }
    
    /// Remove encoder state for a session
    pub fn remove_session(&mut self, session_id: &SessionId) {
        if self.sessions.remove(session_id).is_some() {
            debug!("Removed encoder state for session {}", session_id);
        }
    }
    
    /// Get encoder statistics
    pub fn get_stats(&self) -> RtpEncoderStats {
        RtpEncoderStats {
            packets_encoded: self.packets_encoded,
            encode_errors: self.encode_errors,
            active_sessions: self.sessions.len(),
        }
    }
}

/// Statistics for RTP encoder
#[derive(Debug, Clone)]
pub struct RtpEncoderStats {
    pub packets_encoded: u64,
    pub encode_errors: u64,
    pub active_sessions: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    
    #[test]
    fn test_session_management() {
        let mut encoder = RtpPayloadEncoder::new();
        let session_id = SessionId::new();
        
        // Should fail without initialization
        let frame = AudioFrame {
            timestamp: 0,
            samples: vec![0; 160],
            sample_rate: 8000,
            channels: 1,
            duration: std::time::Duration::from_millis(20), // 20ms for 160 samples at 8kHz
        };
        
        assert!(encoder.encode_audio_frame(&session_id, &frame).is_err());
        
        // Initialize and try again
        encoder.init_session(session_id.clone(), 0);
        let result = encoder.encode_audio_frame(&session_id, &frame).unwrap();
        
        assert_eq!(result.payload_type, 0);
        assert_eq!(result.data.len(), 160); // One byte per sample
        
        // Check stats
        let stats = encoder.get_stats();
        assert_eq!(stats.packets_encoded, 1);
        assert_eq!(stats.active_sessions, 1);
    }
}