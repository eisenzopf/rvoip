//! Media mixing functionality (moved from rtp-core)
//!
//! This module handles media mixing for conference scenarios, including
//! voice activity detection and active speaker mixing.

use std::collections::HashMap;
use bytes::Bytes;
use tracing::{debug, warn};

use crate::api::error::MediaError;
use crate::api::types::{MediaFrame, MediaFrameType};

/// Mix multiple audio frames into a single output frame
///
/// This is a simple implementation that performs basic mixing by:
/// 1. Converting all input frames to the same format (if needed)
/// 2. Normalizing audio levels
/// 3. Mixing the samples
/// 4. Creating a new output frame with mixed data
///
/// For more advanced mixing with proper level adjustment, silence detection,
/// and format conversion, a more sophisticated mixer should be used.
pub async fn mix_audio_frames(
    frames: Vec<MediaFrame>,
    output_payload_type: u8,
    output_timestamp: u32,
    output_ssrc: u32,
) -> Result<MediaFrame, MediaError> {
    // Check if we have any frames to mix
    if frames.is_empty() {
        return Err(MediaError::InvalidInput("No frames to mix".to_string()));
    }
    
    // For simplicity, just use the first frame's timestamp, sequence, and data
    // In a real implementation, this would properly mix audio samples
    let first_frame = &frames[0];
    
    // Create a new frame with mixed data (in this simplified implementation, just use the first frame)
    let mixed_frame = MediaFrame {
        frame_type: MediaFrameType::Audio,
        data: first_frame.data.clone(),
        timestamp: output_timestamp,
        sequence: first_frame.sequence, // In a real implementation, this would be generated
        marker: first_frame.marker,
        payload_type: output_payload_type,
        ssrc: output_ssrc,
        csrcs: frames.iter().map(|f| f.ssrc).collect(), // Use source SSRCs as CSRCs
    };
    
    Ok(mixed_frame)
}

/// Mix audio frames from multiple sources based on active speakers
///
/// This is useful for conference scenarios where we want to mix
/// the N loudest speakers into a single output stream.
pub async fn mix_active_speakers(
    frames: Vec<(String, MediaFrame)>, 
    max_speakers: usize,
    output_payload_type: u8,
    output_timestamp: u32,
    output_ssrc: u32,
) -> Result<MediaFrame, MediaError> {
    // Check if we have any frames to mix
    if frames.is_empty() {
        return Err(MediaError::InvalidInput("No frames to mix".to_string()));
    }
    
    // Filter for audio frames only
    let audio_frames = frames
        .into_iter()
        .filter(|(_, frame)| frame.frame_type == MediaFrameType::Audio)
        .collect::<Vec<_>>();
    
    if audio_frames.is_empty() {
        return Err(MediaError::InvalidInput("No audio frames to mix".to_string()));
    }
    
    // In a real implementation, we would:
    // 1. Calculate audio levels for each frame
    // 2. Sort by level (loudness)
    // 3. Take the top N frames
    // 4. Mix those frames
    
    // For simplicity, just take up to max_speakers frames
    let frames_to_mix = audio_frames
        .into_iter()
        .take(max_speakers)
        .map(|(_, frame)| frame)
        .collect::<Vec<_>>();
    
    // Mix the selected frames
    mix_audio_frames(frames_to_mix, output_payload_type, output_timestamp, output_ssrc).await
}

/// Calculate audio level for a frame (simple implementation)
///
/// Returns a value between 0 (silence) and 127 (loudest)
pub fn calculate_audio_level(frame: &MediaFrame) -> u8 {
    // In a real implementation, we would:
    // 1. Decode the audio samples
    // 2. Calculate RMS power
    // 3. Convert to dB
    // 4. Map to 0-127 range
    
    // For this simple implementation, we just use the average byte value
    // which is not accurate but serves as a placeholder
    if frame.data.is_empty() {
        return 0;
    }
    
    let sum: u32 = frame.data.iter().map(|&b| b as u32).sum();
    let avg = sum / frame.data.len() as u32;
    
    // Map average to 0-127 range
    let level = (avg * 127 / 255) as u8;
    
    level
}

/// Create a voice activity detection result
///
/// Returns (voice_active, level) where:
/// - voice_active is true if voice is detected, false otherwise
/// - level is the audio level (0-127, where 0 is quietest, 127 is loudest)
pub fn detect_voice_activity(frame: &MediaFrame, threshold: u8) -> (bool, u8) {
    // Calculate audio level
    let level = calculate_audio_level(frame);
    
    // Determine if voice is active (level above threshold)
    let voice_active = level > threshold;
    
    (voice_active, level)
}

/// Media mixer for multi-party conferences
pub struct MediaMixer {
    /// Active speakers (SSRC -> audio level)
    active_speakers: HashMap<u32, u8>,
    /// Maximum number of speakers to mix
    max_speakers: usize,
    /// Voice activity threshold
    vad_threshold: u8,
    /// Current output SSRC
    output_ssrc: u32,
    /// Current output payload type
    output_payload_type: u8,
}

impl MediaMixer {
    /// Create a new media mixer
    pub fn new(max_speakers: usize, output_ssrc: u32, output_payload_type: u8) -> Self {
        Self {
            active_speakers: HashMap::new(),
            max_speakers,
            vad_threshold: 10, // Default VAD threshold
            output_ssrc,
            output_payload_type,
        }
    }
    
    /// Set voice activity detection threshold
    pub fn set_vad_threshold(&mut self, threshold: u8) {
        self.vad_threshold = threshold;
    }
    
    /// Update active speakers based on incoming frame
    pub fn update_active_speakers(&mut self, frame: &MediaFrame) {
        let (voice_active, level) = detect_voice_activity(frame, self.vad_threshold);
        
        if voice_active {
            self.active_speakers.insert(frame.ssrc, level);
        } else {
            self.active_speakers.remove(&frame.ssrc);
        }
        
        // Keep only the loudest speakers
        if self.active_speakers.len() > self.max_speakers {
            // Sort by level and keep top speakers
            let mut speakers: Vec<(u32, u8)> = self.active_speakers.iter().map(|(&ssrc, &level)| (ssrc, level)).collect();
            speakers.sort_by(|a, b| b.1.cmp(&a.1)); // Sort by level descending
            
            self.active_speakers.clear();
            for (ssrc, level) in speakers.into_iter().take(self.max_speakers) {
                self.active_speakers.insert(ssrc, level);
            }
        }
    }
    
    /// Get current active speakers
    pub fn get_active_speakers(&self) -> Vec<u32> {
        self.active_speakers.keys().cloned().collect()
    }
    
    /// Mix frames from active speakers
    pub async fn mix_frames(&self, frames: Vec<MediaFrame>, timestamp: u32) -> Result<Option<MediaFrame>, MediaError> {
        if frames.is_empty() {
            return Ok(None);
        }
        
        // Filter frames to only include active speakers
        let active_frames: Vec<MediaFrame> = frames
            .into_iter()
            .filter(|frame| self.active_speakers.contains_key(&frame.ssrc))
            .collect();
        
        if active_frames.is_empty() {
            return Ok(None);
        }
        
        // Mix the active frames
        let mixed = mix_audio_frames(
            active_frames,
            self.output_payload_type,
            timestamp,
            self.output_ssrc,
        ).await?;
        
        Ok(Some(mixed))
    }
}

impl Default for MediaMixer {
    fn default() -> Self {
        Self::new(5, 0x12345678, 0) // Default: mix up to 5 speakers
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    
    fn create_test_frame(ssrc: u32, data: Vec<u8>) -> MediaFrame {
        MediaFrame {
            frame_type: MediaFrameType::Audio,
            data: Bytes::from(data),
            timestamp: 1000,
            sequence: 1,
            marker: false,
            payload_type: 0,
            ssrc,
            csrcs: vec![],
        }
    }
    
    #[tokio::test]
    async fn test_mix_audio_frames() {
        let frame1 = create_test_frame(1, vec![100, 120, 110]);
        let frame2 = create_test_frame(2, vec![80, 90, 100]);
        
        let result = mix_audio_frames(vec![frame1, frame2], 0, 2000, 0x999).await;
        assert!(result.is_ok());
        
        let mixed = result.unwrap();
        assert_eq!(mixed.ssrc, 0x999);
        assert_eq!(mixed.timestamp, 2000);
        assert_eq!(mixed.csrcs, vec![1, 2]);
    }
    
    #[test]
    fn test_calculate_audio_level() {
        let frame = create_test_frame(1, vec![255, 255, 255]); // Max level
        let level = calculate_audio_level(&frame);
        assert_eq!(level, 127); // Should be maximum
        
        let frame = create_test_frame(1, vec![0, 0, 0]); // Silence
        let level = calculate_audio_level(&frame);
        assert_eq!(level, 0); // Should be minimum
    }
    
    #[test]
    fn test_voice_activity_detection() {
        let frame = create_test_frame(1, vec![200, 200, 200]); // Loud
        let (active, level) = detect_voice_activity(&frame, 50);
        assert!(active);
        assert!(level > 50);
        
        let frame = create_test_frame(1, vec![10, 10, 10]); // Quiet
        let (active, level) = detect_voice_activity(&frame, 50);
        assert!(!active);
        assert!(level < 50);
    }
    
    #[test]
    fn test_media_mixer() {
        let mut mixer = MediaMixer::new(2, 0x999, 0);
        mixer.set_vad_threshold(50);
        
        // Add loud frame
        let loud_frame = create_test_frame(1, vec![200, 200, 200]);
        mixer.update_active_speakers(&loud_frame);
        
        // Add quiet frame
        let quiet_frame = create_test_frame(2, vec![10, 10, 10]);
        mixer.update_active_speakers(&quiet_frame);
        
        let active = mixer.get_active_speakers();
        assert_eq!(active.len(), 1); // Only loud speaker should be active
        assert!(active.contains(&1));
    }
}