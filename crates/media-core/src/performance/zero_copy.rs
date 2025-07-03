//! Zero-copy audio frame implementations
//!
//! This module provides zero-copy alternatives to the standard AudioFrame
//! that eliminate buffer copies during media processing.

use std::sync::Arc;
use std::time::Duration;
use crate::types::AudioFrame;

/// Shared audio buffer using Arc for zero-copy sharing
#[derive(Debug, Clone)]
pub struct SharedAudioBuffer {
    /// Shared PCM audio data (interleaved samples)
    data: Arc<[i16]>,
    /// Offset into the shared buffer
    offset: usize,
    /// Length of this view
    length: usize,
}

impl SharedAudioBuffer {
    /// Create a new shared buffer from existing data
    pub fn new(data: Vec<i16>) -> Self {
        let length = data.len();
        Self {
            data: Arc::from(data.into_boxed_slice()),
            offset: 0,
            length,
        }
    }
    
    /// Create from Arc<[i16]> directly
    pub fn from_arc(data: Arc<[i16]>) -> Self {
        let length = data.len();
        Self {
            data,
            offset: 0,
            length,
        }
    }
    
    /// Create a view into a subset of the buffer (zero-copy slice)
    pub fn slice(&self, start: usize, len: usize) -> Option<Self> {
        if start + len <= self.length {
            Some(Self {
                data: self.data.clone(),
                offset: self.offset + start,
                length: len,
            })
        } else {
            None
        }
    }
    
    /// Get samples as a slice
    pub fn samples(&self) -> &[i16] {
        &self.data[self.offset..self.offset + self.length]
    }
    
    /// Get the length of this buffer view
    pub fn len(&self) -> usize {
        self.length
    }
    
    /// Check if buffer is empty
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }
    
    /// Clone the underlying data into a Vec (when copy is explicitly needed)
    pub fn to_vec(&self) -> Vec<i16> {
        self.samples().to_vec()
    }
    
    /// Get reference count for debugging
    pub fn ref_count(&self) -> usize {
        Arc::strong_count(&self.data)
    }
}

/// Zero-copy audio frame using shared buffers
#[derive(Debug, Clone)]
pub struct ZeroCopyAudioFrame {
    /// Shared audio buffer
    pub buffer: SharedAudioBuffer,
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Number of channels
    pub channels: u8,
    /// Frame duration
    pub duration: Duration,
    /// Timestamp
    pub timestamp: u32,
}

impl ZeroCopyAudioFrame {
    /// Create a new zero-copy audio frame
    pub fn new(
        samples: Vec<i16>,
        sample_rate: u32,
        channels: u8,
        timestamp: u32,
    ) -> Self {
        let buffer = SharedAudioBuffer::new(samples);
        let sample_count = buffer.len() / channels as usize;
        let duration = Duration::from_secs_f64(sample_count as f64 / sample_rate as f64);
        
        Self {
            buffer,
            sample_rate,
            channels,
            duration,
            timestamp,
        }
    }
    
    /// Create from SharedAudioBuffer
    pub fn from_buffer(
        buffer: SharedAudioBuffer,
        sample_rate: u32,
        channels: u8,
        timestamp: u32,
    ) -> Self {
        let sample_count = buffer.len() / channels as usize;
        let duration = Duration::from_secs_f64(sample_count as f64 / sample_rate as f64);
        
        Self {
            buffer,
            sample_rate,
            channels,
            duration,
            timestamp,
        }
    }
    
    /// Get samples as a slice (zero-copy access)
    pub fn samples(&self) -> &[i16] {
        self.buffer.samples()
    }
    
    /// Get the number of samples per channel
    pub fn samples_per_channel(&self) -> usize {
        self.buffer.len() / self.channels as usize
    }
    
    /// Check if frame is mono
    pub fn is_mono(&self) -> bool {
        self.channels == 1
    }
    
    /// Check if frame is stereo
    pub fn is_stereo(&self) -> bool {
        self.channels == 2
    }
    
    /// Create a zero-copy slice of this frame
    pub fn slice(&self, start_sample: usize, sample_count: usize) -> Option<Self> {
        let start_idx = start_sample * self.channels as usize;
        let length = sample_count * self.channels as usize;
        
        if let Some(buffer_slice) = self.buffer.slice(start_idx, length) {
            Some(Self::from_buffer(
                buffer_slice,
                self.sample_rate,
                self.channels,
                self.timestamp + start_sample as u32,
            ))
        } else {
            None
        }
    }
    
    /// Convert to traditional AudioFrame (explicit copy)
    pub fn to_audio_frame(&self) -> AudioFrame {
        AudioFrame::new(
            self.buffer.to_vec(),
            self.sample_rate,
            self.channels,
            self.timestamp,
        )
    }
    
    /// Get reference count for debugging performance
    pub fn ref_count(&self) -> usize {
        self.buffer.ref_count()
    }
}

impl From<AudioFrame> for ZeroCopyAudioFrame {
    fn from(frame: AudioFrame) -> Self {
        Self::new(frame.samples, frame.sample_rate, frame.channels, frame.timestamp)
    }
}

impl From<ZeroCopyAudioFrame> for AudioFrame {
    fn from(frame: ZeroCopyAudioFrame) -> Self {
        frame.to_audio_frame()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shared_audio_buffer_creation() {
        let samples = vec![1, 2, 3, 4, 5, 6];
        let buffer = SharedAudioBuffer::new(samples.clone());
        
        assert_eq!(buffer.len(), 6);
        assert_eq!(buffer.samples(), &samples);
        assert_eq!(buffer.ref_count(), 1);
    }
    
    #[test]
    fn test_shared_audio_buffer_slice() {
        let samples = vec![1, 2, 3, 4, 5, 6];
        let buffer = SharedAudioBuffer::new(samples);
        
        let slice = buffer.slice(2, 3).unwrap();
        assert_eq!(slice.samples(), &[3, 4, 5]);
        assert_eq!(slice.len(), 3);
        
        // Both should share the same underlying data
        assert_eq!(buffer.ref_count(), 2);
        assert_eq!(slice.ref_count(), 2);
    }
    
    #[test]
    fn test_zero_copy_audio_frame_creation() {
        let samples = vec![100, 200, 300, 400]; // 2 channels, 2 samples per channel
        let frame = ZeroCopyAudioFrame::new(samples.clone(), 8000, 2, 1000);
        
        assert_eq!(frame.samples(), &samples);
        assert_eq!(frame.sample_rate, 8000);
        assert_eq!(frame.channels, 2);
        assert_eq!(frame.samples_per_channel(), 2);
        assert!(frame.is_stereo());
        assert!(!frame.is_mono());
    }
    
    #[test]
    fn test_zero_copy_frame_slice() {
        let samples = vec![1, 2, 3, 4, 5, 6, 7, 8]; // 2 channels, 4 samples per channel
        let frame = ZeroCopyAudioFrame::new(samples, 8000, 2, 1000);
        
        // Take middle 2 samples per channel (4 total samples)
        let slice = frame.slice(1, 2).unwrap();
        assert_eq!(slice.samples(), &[3, 4, 5, 6]);
        assert_eq!(slice.samples_per_channel(), 2);
        assert_eq!(slice.timestamp, 1001); // Adjusted timestamp
        
        // Should share the same underlying buffer
        assert_eq!(frame.ref_count(), 2);
        assert_eq!(slice.ref_count(), 2);
    }
    
    #[test]
    fn test_audio_frame_conversion() {
        let samples = vec![100, 200, 300, 400];
        let original_frame = AudioFrame::new(samples.clone(), 8000, 2, 1000);
        
        // Convert to zero-copy
        let zero_copy_frame = ZeroCopyAudioFrame::from(original_frame.clone());
        assert_eq!(zero_copy_frame.samples(), &samples);
        
        // Convert back
        let converted_frame = AudioFrame::from(zero_copy_frame);
        assert_eq!(converted_frame.samples, original_frame.samples);
        assert_eq!(converted_frame.sample_rate, original_frame.sample_rate);
    }
    
    #[test]
    fn test_zero_copy_no_unnecessary_copies() {
        let samples = vec![1, 2, 3, 4, 5, 6];
        let frame = ZeroCopyAudioFrame::new(samples, 8000, 1, 1000);
        
        // Multiple clones should not increase memory usage
        let clone1 = frame.clone();
        let clone2 = frame.clone();
        let clone3 = frame.clone();
        
        // All should share the same underlying data
        assert_eq!(frame.ref_count(), 4);
        assert_eq!(clone1.ref_count(), 4);
        assert_eq!(clone2.ref_count(), 4);
        assert_eq!(clone3.ref_count(), 4);
        
        // All should have the same samples
        assert_eq!(frame.samples(), clone1.samples());
        assert_eq!(frame.samples(), clone2.samples());
        assert_eq!(frame.samples(), clone3.samples());
    }
} 