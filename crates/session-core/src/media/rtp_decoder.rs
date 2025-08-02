//! RTP Payload Decoder
//!
//! This module handles decoding RTP payloads to AudioFrames for playback.
//! It bridges rtp-core events with codec-core decoding capabilities.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, warn};

use crate::api::types::{SessionId, AudioFrame};
use crate::errors::{Result, SessionError};

/// RTP event types from rtp-core
#[derive(Debug, Clone)]
pub enum RtpEvent {
    MediaReceived {
        payload_type: u8,
        payload: Vec<u8>,
        timestamp: u32,
        sequence_number: u16,
        ssrc: u32,
    },
    PacketLost {
        sequence_number: u16,
    },
    JitterBufferOverflow,
}

/// RTP Payload Decoder
/// 
/// Handles conversion of RTP payloads to AudioFrames using codec-core.
/// Manages audio frame distribution to multiple subscribers per session.
pub struct RtpPayloadDecoder {
    /// Audio frame senders for each session
    audio_frame_senders: HashMap<SessionId, mpsc::Sender<AudioFrame>>,
    /// Statistics for monitoring
    packets_processed: u64,
    decode_errors: u64,
}

impl RtpPayloadDecoder {
    /// Create a new RTP payload decoder
    pub fn new() -> Self {
        Self {
            audio_frame_senders: HashMap::new(),
            packets_processed: 0,
            decode_errors: 0,
        }
    }

    /// Process an RTP event and decode to AudioFrame if applicable
    pub async fn process_rtp_event(&mut self, event: RtpEvent, session_id: &SessionId) -> Result<()> {
        match event {
            RtpEvent::MediaReceived { payload_type, payload, timestamp, .. } => {
                self.packets_processed += 1;
                
                // Decode payload based on type
                let decoded_samples = match self.decode_payload(payload_type, &payload) {
                    Ok(samples) => samples,
                    Err(e) => {
                        self.decode_errors += 1;
                        error!("Failed to decode RTP payload type {}: {}", payload_type, e);
                        return Err(e);
                    }
                };
                
                // Create AudioFrame
                let audio_frame = AudioFrame {
                    samples: decoded_samples,
                    sample_rate: self.get_sample_rate_for_payload_type(payload_type),
                    channels: 1, // G.711 is always mono
                    timestamp,
                };
                
                // Forward to subscribers
                if let Some(sender) = self.audio_frame_senders.get(session_id) {
                    let sample_count = audio_frame.samples.len();
                    if let Err(e) = sender.try_send(audio_frame) {
                        warn!("Failed to send audio frame to subscriber for session {}: {}", session_id, e);
                    } else {
                        debug!("Sent audio frame with {} samples to session {}", sample_count, session_id);
                    }
                } else {
                    debug!("No subscriber for session {}, dropping audio frame", session_id);
                }
            }
            RtpEvent::PacketLost { sequence_number } => {
                debug!("RTP packet lost for session {}: seq {}", session_id, sequence_number);
                // Could implement packet loss concealment here
            }
            RtpEvent::JitterBufferOverflow => {
                warn!("Jitter buffer overflow for session {}", session_id);
            }
        }
        
        Ok(())
    }
    
    /// Add a subscriber for audio frames from a specific session
    pub fn add_subscriber(&mut self, session_id: SessionId, sender: mpsc::Sender<AudioFrame>) {
        debug!("Added audio frame subscriber for session: {}", session_id);
        self.audio_frame_senders.insert(session_id, sender);
    }
    
    /// Remove a subscriber for a session
    pub fn remove_subscriber(&mut self, session_id: &SessionId) {
        if self.audio_frame_senders.remove(session_id).is_some() {
            debug!("Removed audio frame subscriber for session: {}", session_id);
        }
    }
    
    /// Get decoder statistics
    pub fn get_stats(&self) -> RtpDecoderStats {
        RtpDecoderStats {
            packets_processed: self.packets_processed,
            decode_errors: self.decode_errors,
            active_subscribers: self.audio_frame_senders.len(),
        }
    }
    
    /// Decode RTP payload based on payload type
    fn decode_payload(&self, payload_type: u8, payload: &[u8]) -> Result<Vec<i16>> {
        match payload_type {
            0 => self.decode_g711_ulaw(payload),  // PCMU
            8 => self.decode_g711_alaw(payload),  // PCMA
            _ => {
                debug!("Unsupported payload type for decode: {}", payload_type);
                Err(SessionError::MediaIntegration { 
                    message: format!("Unsupported payload type: {}", payload_type) 
                })
            }
        }
    }
    
    /// Get sample rate for a payload type
    fn get_sample_rate_for_payload_type(&self, payload_type: u8) -> u32 {
        match payload_type {
            0 | 8 => 8000,  // G.711 is always 8kHz
            _ => 8000,      // Default fallback
        }
    }
    
    /// Decode G.711 μ-law payload to PCM samples
    fn decode_g711_ulaw(&self, payload: &[u8]) -> Result<Vec<i16>> {
        let mut samples = Vec::with_capacity(payload.len());
        
        // G.711 μ-law decode using standard table
        for &byte in payload {
            let sample = Self::ulaw_to_linear(byte);
            samples.push(sample);
        }
        
        Ok(samples)
    }
    
    /// Decode G.711 A-law payload to PCM samples
    fn decode_g711_alaw(&self, payload: &[u8]) -> Result<Vec<i16>> {
        let mut samples = Vec::with_capacity(payload.len());
        
        // G.711 A-law decode using standard table
        for &byte in payload {
            let sample = Self::alaw_to_linear(byte);
            samples.push(sample);
        }
        
        Ok(samples)
    }
    
    /// Convert μ-law byte to linear PCM sample
    fn ulaw_to_linear(ulaw_byte: u8) -> i16 {
        // Standard ITU-T G.711 μ-law to linear conversion
        let mut ulaw_byte = !ulaw_byte;
        let sign = if (ulaw_byte & 0x80) != 0 { -1 } else { 1 };
        let exponent = (ulaw_byte >> 4) & 0x07;
        let mantissa = ulaw_byte & 0x0F;
        
        let mut sample = ((mantissa << 4) | 0x08) as u16;
        if exponent > 0 {
            sample = (sample | 0x100) << (exponent - 1);
        }
        
        (sign * sample as i16) << 2
    }
    
    /// Convert A-law byte to linear PCM sample  
    fn alaw_to_linear(alaw_byte: u8) -> i16 {
        // Standard ITU-T G.711 A-law to linear conversion
        let mut alaw_byte = alaw_byte ^ 0x55;
        let sign = if (alaw_byte & 0x80) != 0 { -1 } else { 1 };
        let exponent = (alaw_byte >> 4) & 0x07;
        let mantissa = alaw_byte & 0x0F;
        
        let mut sample = (mantissa << 4) as u16;
        if exponent > 0 {
            sample = (sample | 0x100) << (exponent - 1);
        } else {
            sample |= 0x08;
        }
        
        (sign * sample as i16) << 3
    }
}

impl Default for RtpPayloadDecoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics for RTP payload decoder
#[derive(Debug, Clone)]
pub struct RtpDecoderStats {
    pub packets_processed: u64,
    pub decode_errors: u64,
    pub active_subscribers: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;
    
    #[tokio::test]
    async fn test_rtp_decoder_creation() {
        let decoder = RtpPayloadDecoder::new();
        let stats = decoder.get_stats();
        
        assert_eq!(stats.packets_processed, 0);
        assert_eq!(stats.decode_errors, 0);
        assert_eq!(stats.active_subscribers, 0);
    }
    
    #[tokio::test]
    async fn test_add_remove_subscriber() {
        let mut decoder = RtpPayloadDecoder::new();
        let (sender, _receiver) = mpsc::channel(10);
        let session_id = SessionId("test-session".to_string());
        
        // Add subscriber
        decoder.add_subscriber(session_id.clone(), sender);
        assert_eq!(decoder.get_stats().active_subscribers, 1);
        
        // Remove subscriber
        decoder.remove_subscriber(&session_id);
        assert_eq!(decoder.get_stats().active_subscribers, 0);
    }
    
    #[test]
    fn test_g711_ulaw_decode() {
        let decoder = RtpPayloadDecoder::new();
        
        // Test silence (μ-law encoded silence is 0xFF)
        let silence_payload = vec![0xFF, 0xFF, 0xFF, 0xFF];
        let decoded = decoder.decode_g711_ulaw(&silence_payload).unwrap();
        
        assert_eq!(decoded.len(), 4);
        // μ-law silence should decode to approximately 0
        for sample in decoded {
            assert!(sample.abs() < 100); // Allow some tolerance
        }
    }
    
    #[test]
    fn test_g711_alaw_decode() {
        let decoder = RtpPayloadDecoder::new();
        
        // Test silence (A-law encoded silence is 0xD5)
        let silence_payload = vec![0xD5, 0xD5, 0xD5, 0xD5];
        let decoded = decoder.decode_g711_alaw(&silence_payload).unwrap();
        
        assert_eq!(decoded.len(), 4);
        // A-law silence should decode to approximately 0
        for sample in decoded {
            assert!(sample.abs() < 100); // Allow some tolerance
        }
    }
    
    #[tokio::test]
    async fn test_process_media_received_event() {
        let mut decoder = RtpPayloadDecoder::new();
        let (sender, mut receiver) = mpsc::channel(10);
        let session_id = SessionId("test-session".to_string());
        
        // Add subscriber
        decoder.add_subscriber(session_id.clone(), sender);
        
        // Create test RTP event with μ-law payload
        let event = RtpEvent::MediaReceived {
            payload_type: 0, // PCMU
            payload: vec![0xFF, 0x7F, 0x00, 0x80], // Test μ-law data
            timestamp: 12345,
            sequence_number: 1,
            ssrc: 0x12345678,
        };
        
        // Process event
        decoder.process_rtp_event(event, &session_id).await.unwrap();
        
        // Verify frame was sent
        let audio_frame = receiver.try_recv().unwrap();
        assert_eq!(audio_frame.samples.len(), 4);
        assert_eq!(audio_frame.sample_rate, 8000);
        assert_eq!(audio_frame.channels, 1);
        assert_eq!(audio_frame.timestamp, 12345);
        
        // Verify stats
        let stats = decoder.get_stats();
        assert_eq!(stats.packets_processed, 1);
        assert_eq!(stats.decode_errors, 0);
    }
    
    #[tokio::test]
    async fn test_unsupported_payload_type() {
        let mut decoder = RtpPayloadDecoder::new();
        let session_id = SessionId("test-session".to_string());
        
        let event = RtpEvent::MediaReceived {
            payload_type: 99, // Unsupported
            payload: vec![0x01, 0x02, 0x03],
            timestamp: 12345,
            sequence_number: 1,
            ssrc: 0x12345678,
        };
        
        // Should return error for unsupported payload type
        let result = decoder.process_rtp_event(event, &session_id).await;
        assert!(result.is_err());
        
        // Should increment error count
        let stats = decoder.get_stats();
        assert_eq!(stats.decode_errors, 1);
    }
    
    #[tokio::test]
    async fn test_packet_loss_event() {
        let mut decoder = RtpPayloadDecoder::new();
        let session_id = SessionId("test-session".to_string());
        
        let event = RtpEvent::PacketLost {
            sequence_number: 42,
        };
        
        // Should handle packet loss gracefully
        let result = decoder.process_rtp_event(event, &session_id).await;
        assert!(result.is_ok());
    }
}