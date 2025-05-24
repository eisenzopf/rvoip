//! Codec Transcoding
//!
//! This module provides real-time transcoding between different audio codecs,
//! enabling mixed-codec calls and codec negotiation fallbacks.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn, trace};

use crate::error::{Result, CodecError};
use crate::types::{AudioFrame, SampleRate, PayloadType};
use crate::codec::audio::common::AudioCodec;
use crate::codec::audio::{G711Codec, G711Variant, OpusCodec, OpusApplication, G729Codec};
use crate::processing::format::{FormatConverter, ConversionParams};

/// Transcoding path between two codecs
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TranscodingPath {
    /// Source codec payload type
    pub from_codec: PayloadType,
    /// Target codec payload type  
    pub to_codec: PayloadType,
}

/// Transcoding session for converting between two specific codecs
pub struct TranscodingSession {
    /// Source codec for decoding
    source_codec: Box<dyn AudioCodec>,
    /// Target codec for encoding
    target_codec: Box<dyn AudioCodec>,
    /// Format converter for sample rate/channel conversion
    format_converter: Arc<RwLock<FormatConverter>>,
    /// Transcoding statistics
    stats: TranscodingStats,
}

/// Transcoding statistics
#[derive(Debug, Clone, Default)]
pub struct TranscodingStats {
    /// Frames transcoded
    pub frames_transcoded: u64,
    /// Total processing time (microseconds)
    pub total_processing_time_us: u64,
    /// Average processing time per frame (microseconds)
    pub avg_processing_time_us: f32,
    /// Transcoding errors
    pub errors: u64,
}

/// Main transcoding engine
pub struct Transcoder {
    /// Active transcoding sessions
    sessions: HashMap<TranscodingPath, TranscodingSession>,
    /// Format converter for intermediate processing
    format_converter: Arc<RwLock<FormatConverter>>,
    /// Enable performance statistics
    enable_stats: bool,
}

impl Transcoder {
    /// Create a new transcoder
    pub fn new(format_converter: Arc<RwLock<FormatConverter>>) -> Self {
        debug!("Creating Transcoder");
        
        Self {
            sessions: HashMap::new(),
            format_converter,
            enable_stats: true,
        }
    }
    
    /// Get or create a transcoding session for the given path
    pub fn get_or_create_session(
        &mut self,
        from_codec: PayloadType,
        to_codec: PayloadType,
    ) -> Result<&mut TranscodingSession> {
        let path = TranscodingPath { from_codec, to_codec };
        
        // Return existing session if available
        if self.sessions.contains_key(&path) {
            return Ok(self.sessions.get_mut(&path).unwrap());
        }
        
        // Create new transcoding session
        let session = self.create_transcoding_session(from_codec, to_codec)?;
        self.sessions.insert(path.clone(), session);
        
        debug!("Created transcoding session: {} -> {}", from_codec, to_codec);
        Ok(self.sessions.get_mut(&path).unwrap())
    }
    
    /// Transcode audio data from one codec to another
    pub async fn transcode(
        &mut self,
        encoded_data: &[u8],
        from_codec: PayloadType,
        to_codec: PayloadType,
    ) -> Result<Vec<u8>> {
        // Short-circuit if no transcoding needed
        if from_codec == to_codec {
            return Ok(encoded_data.to_vec());
        }
        
        let start_time = std::time::Instant::now();
        let enable_stats = self.enable_stats; // Copy the flag to avoid borrowing issues
        
        // Get transcoding session
        let session = self.get_or_create_session(from_codec, to_codec)?;
        
        // Perform transcoding
        let result = session.transcode(encoded_data).await;
        
        // Update statistics
        if enable_stats {
            let processing_time = start_time.elapsed().as_micros() as u64;
            session.stats.total_processing_time_us += processing_time;
            
            match &result {
                Ok(_) => {
                    session.stats.frames_transcoded += 1;
                    session.stats.avg_processing_time_us = 
                        session.stats.total_processing_time_us as f32 / session.stats.frames_transcoded as f32;
                }
                Err(_) => session.stats.errors += 1,
            }
        }
        
        result
    }
    
    /// Create a new transcoding session between two codecs
    fn create_transcoding_session(
        &self,
        from_codec: PayloadType,
        to_codec: PayloadType,
    ) -> Result<TranscodingSession> {
        let source_codec = self.create_codec(from_codec)?;
        let target_codec = self.create_codec(to_codec)?;
        
        Ok(TranscodingSession {
            source_codec,
            target_codec,
            format_converter: self.format_converter.clone(),
            stats: TranscodingStats::default(),
        })
    }
    
    /// Create a codec instance for the given payload type
    fn create_codec(&self, payload_type: PayloadType) -> Result<Box<dyn AudioCodec>> {
        match payload_type {
            0 => { // PCMU
                let codec = G711Codec::new(
                    SampleRate::Rate8000,
                    1,
                    crate::codec::audio::G711Config {
                        variant: G711Variant::MuLaw,
                        sample_rate: 8000,
                        channels: 1,
                        frame_size_ms: 10.0,
                    }
                )?;
                Ok(Box::new(codec))
            }
            8 => { // PCMA
                let codec = G711Codec::new(
                    SampleRate::Rate8000,
                    1,
                    crate::codec::audio::G711Config {
                        variant: G711Variant::ALaw,
                        sample_rate: 8000,
                        channels: 1,
                        frame_size_ms: 10.0,
                    }
                )?;
                Ok(Box::new(codec))
            }
            18 => { // G.729
                let codec = G729Codec::new(
                    SampleRate::Rate8000,
                    1,
                    crate::codec::audio::G729Config::default(),
                )?;
                Ok(Box::new(codec))
            }
            111 => { // Opus (dynamic)
                let codec = OpusCodec::new(
                    SampleRate::Rate48000,
                    2,
                    crate::codec::audio::OpusConfig {
                        application: OpusApplication::Voip,
                        bitrate: 64000,
                        frame_size_ms: 20.0,
                        vbr: true,
                        complexity: 5,
                    }
                )?;
                Ok(Box::new(codec))
            }
            _ => Err(CodecError::UnsupportedPayloadType { payload_type }.into()),
        }
    }
    
    /// Get transcoding statistics for a specific path
    pub fn get_stats(&self, from_codec: PayloadType, to_codec: PayloadType) -> Option<&TranscodingStats> {
        let path = TranscodingPath { from_codec, to_codec };
        self.sessions.get(&path).map(|session| &session.stats)
    }
    
    /// Get all supported transcoding paths
    pub fn get_supported_paths(&self) -> Vec<TranscodingPath> {
        let supported_codecs = vec![0u8, 8u8, 18u8, 111u8]; // PCMU, PCMA, G.729, Opus
        let mut paths = Vec::new();
        
        for &from in &supported_codecs {
            for &to in &supported_codecs {
                if from != to {
                    paths.push(TranscodingPath { from_codec: from, to_codec: to });
                }
            }
        }
        
        paths
    }
    
    /// Clear all transcoding sessions (useful for memory management)
    pub fn clear_sessions(&mut self) {
        self.sessions.clear();
        debug!("Cleared all transcoding sessions");
    }
}

impl TranscodingSession {
    /// Transcode audio data
    pub async fn transcode(&mut self, encoded_data: &[u8]) -> Result<Vec<u8>> {
        // Step 1: Decode source audio
        let source_frame = self.source_codec.decode(encoded_data)?;
        
        // Step 2: Format conversion if needed
        let target_info = self.target_codec.get_info();
        let converted_frame = if source_frame.sample_rate != target_info.sample_rate 
            || source_frame.channels != target_info.channels {
            
            trace!("Converting format: {}Hz/{}ch -> {}Hz/{}ch", 
                   source_frame.sample_rate, source_frame.channels,
                   target_info.sample_rate, target_info.channels);
            
            // Use FormatConverter's public API
            let conversion_params = ConversionParams::new(
                SampleRate::from_hz(target_info.sample_rate)
                    .unwrap_or(SampleRate::Rate8000),
                target_info.channels,
            );
            
            let conversion_result = self.format_converter.write().await.convert_frame(&source_frame, &conversion_params)?;
            conversion_result.frame
        } else {
            source_frame
        };
        
        // Step 3: Encode to target format
        let encoded = self.target_codec.encode(&converted_frame)?;
        
        trace!("Transcoded {} bytes -> {} bytes", encoded_data.len(), encoded.len());
        Ok(encoded)
    }
}

/// Utility functions for common transcoding scenarios
impl Transcoder {
    /// Transcode G.711 PCMU to Opus
    pub async fn pcmu_to_opus(&mut self, pcmu_data: &[u8]) -> Result<Vec<u8>> {
        self.transcode(pcmu_data, 0, 111).await
    }
    
    /// Transcode Opus to G.711 PCMU
    pub async fn opus_to_pcmu(&mut self, opus_data: &[u8]) -> Result<Vec<u8>> {
        self.transcode(opus_data, 111, 0).await
    }
    
    /// Transcode G.711 PCMU to PCMA
    pub async fn pcmu_to_pcma(&mut self, pcmu_data: &[u8]) -> Result<Vec<u8>> {
        self.transcode(pcmu_data, 0, 8).await
    }
    
    /// Transcode G.711 PCMA to PCMU
    pub async fn pcma_to_pcmu(&mut self, pcma_data: &[u8]) -> Result<Vec<u8>> {
        self.transcode(pcma_data, 8, 0).await
    }
    
    /// Transcode G.711 PCMU to G.729
    pub async fn pcmu_to_g729(&mut self, pcmu_data: &[u8]) -> Result<Vec<u8>> {
        self.transcode(pcmu_data, 0, 18).await
    }
    
    /// Transcode G.729 to G.711 PCMU
    pub async fn g729_to_pcmu(&mut self, g729_data: &[u8]) -> Result<Vec<u8>> {
        self.transcode(g729_data, 18, 0).await
    }
    
    /// Transcode G.729 to Opus
    pub async fn g729_to_opus(&mut self, g729_data: &[u8]) -> Result<Vec<u8>> {
        self.transcode(g729_data, 18, 111).await
    }
    
    /// Transcode Opus to G.729
    pub async fn opus_to_g729(&mut self, opus_data: &[u8]) -> Result<Vec<u8>> {
        self.transcode(opus_data, 111, 18).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processing::format::ConversionParams;
    
    fn create_test_transcoder() -> Transcoder {
        let format_converter = Arc::new(RwLock::new(FormatConverter::new()));
        Transcoder::new(format_converter)
    }
    
    #[test]
    fn test_transcoder_creation() {
        let transcoder = create_test_transcoder();
        assert_eq!(transcoder.sessions.len(), 0);
        
        let paths = transcoder.get_supported_paths();
        assert!(!paths.is_empty());
        assert!(paths.contains(&TranscodingPath { from_codec: 0, to_codec: 8 }));
        assert!(paths.contains(&TranscodingPath { from_codec: 0, to_codec: 111 }));
    }
    
    #[tokio::test]
    async fn test_same_codec_transcoding() {
        let mut transcoder = create_test_transcoder();
        let test_data = vec![0x80, 0x90, 0xA0, 0xB0];
        
        // Same codec should return identical data
        let result = transcoder.transcode(&test_data, 0, 0).await.unwrap();
        assert_eq!(result, test_data);
    }
    
    #[tokio::test]
    async fn test_pcmu_to_pcma_transcoding() {
        let mut transcoder = create_test_transcoder();
        
        // Create test PCMU data (80 bytes for 10ms frame)
        let pcmu_data = vec![0xFF; 80]; // PCMU 10ms frame
        
        let result = transcoder.pcmu_to_pcma(&pcmu_data).await;
        assert!(result.is_ok());
        
        let pcma_data = result.unwrap();
        assert_eq!(pcma_data.len(), 80); // Same length for G.711 variants (10ms frame)
        assert_ne!(pcma_data, pcmu_data); // Should be different encoding
    }
    
    #[tokio::test]
    async fn test_transcoding_statistics() {
        let mut transcoder = create_test_transcoder();
        let test_data = vec![0xFF; 80]; // 10ms frame
        
        // Perform several transcodings
        for _ in 0..5 {
            transcoder.pcmu_to_pcma(&test_data).await.unwrap();
        }
        
        let stats = transcoder.get_stats(0, 8).unwrap();
        assert_eq!(stats.frames_transcoded, 5);
        assert_eq!(stats.errors, 0);
        assert!(stats.avg_processing_time_us > 0.0);
    }
    
    #[tokio::test]
    async fn test_unsupported_codec() {
        let mut transcoder = create_test_transcoder();
        let test_data = vec![0x80, 0x90];
        
        // Try to transcode to unsupported codec
        let result = transcoder.transcode(&test_data, 0, 99).await;
        assert!(result.is_err());
        
        if let Err(e) = result {
            assert!(matches!(e, crate::error::Error::Codec(CodecError::UnsupportedPayloadType { payload_type: 99 })));
        }
    }
    
    #[test]
    fn test_session_management() {
        let mut transcoder = create_test_transcoder();
        
        // Create a session
        transcoder.get_or_create_session(0, 8).unwrap();
        assert_eq!(transcoder.sessions.len(), 1);
        
        // Reuse existing session
        transcoder.get_or_create_session(0, 8).unwrap();
        assert_eq!(transcoder.sessions.len(), 1);
        
        // Create different session
        transcoder.get_or_create_session(8, 0).unwrap();
        assert_eq!(transcoder.sessions.len(), 2);
        
        // Clear sessions
        transcoder.clear_sessions();
        assert_eq!(transcoder.sessions.len(), 0);
    }
    
    #[tokio::test]
    async fn test_g729_transcoding() {
        let mut transcoder = create_test_transcoder();
        
        // Create test G.729 data (10 bytes for one frame)
        let g729_data = vec![0x80, 0x90, 0xA0, 0xB0, 0xC0, 0xD0, 0xE0, 0xF0, 0x10, 0x20];
        
        let result = transcoder.g729_to_pcmu(&g729_data).await;
        if let Err(e) = &result {
            println!("G.729 to PCMU error: {:?}", e);
        }
        assert!(result.is_ok());
        
        let pcmu_data = result.unwrap();
        println!("G.729 -> PCMU: {} bytes -> {} bytes", g729_data.len(), pcmu_data.len());
        assert_eq!(pcmu_data.len(), 80); // G.729 10ms -> G.711 10ms (both 80 samples)
        
        // Test reverse transcoding with properly sized PCMU data (80 bytes for 10ms)
        let pcmu_test_data = vec![0xFF; 80]; // PCMU 10ms frame
        let result = transcoder.pcmu_to_g729(&pcmu_test_data).await;
        if let Err(e) = &result {
            println!("PCMU to G.729 error: {:?}", e);
        }
        assert!(result.is_ok());
        
        let transcoded_g729 = result.unwrap();
        println!("PCMU -> G.729: {} bytes -> {} bytes", pcmu_test_data.len(), transcoded_g729.len());
        assert_eq!(transcoded_g729.len(), 10); // Back to G.729 frame size (10 bytes)
    }
    
    #[test]
    fn test_g729_transcoding_paths() {
        let transcoder = create_test_transcoder();
        let paths = transcoder.get_supported_paths();
        
        // Check that G.729 transcoding paths are included
        assert!(paths.contains(&TranscodingPath { from_codec: 18, to_codec: 0 })); // G.729 -> PCMU
        assert!(paths.contains(&TranscodingPath { from_codec: 0, to_codec: 18 })); // PCMU -> G.729
        assert!(paths.contains(&TranscodingPath { from_codec: 18, to_codec: 111 })); // G.729 -> Opus
        assert!(paths.contains(&TranscodingPath { from_codec: 111, to_codec: 18 })); // Opus -> G.729
    }
} 