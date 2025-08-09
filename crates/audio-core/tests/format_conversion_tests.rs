//! Format conversion integration tests
//!
//! This module contains integration tests for audio format conversion functionality
//! including sample rate conversion, channel mapping, and frame buffering.

use rvoip_audio_core::{
    types::{AudioFormat, AudioFrame},
    format::{FormatConverter, AudioFrameBuffer},
    error::AudioError,
};

#[cfg(test)]
mod format_converter_tests {
    use super::*;

    #[test]
    fn test_format_converter_same_format() {
        let format = AudioFormat::pcm_8khz_mono();
        let mut converter = FormatConverter::new(format.clone(), format.clone()).unwrap();
        
        let input_samples = vec![100, 200, 300, 400];
        let input_frame = AudioFrame::new(input_samples.clone(), format, 1000);
        
        let result = converter.convert_frame(&input_frame).unwrap();
        assert_eq!(result.samples, input_samples);
        assert_eq!(result.format.sample_rate, 8000);
    }

    #[test]
    fn test_converter_creation() {
        let input_format = AudioFormat::pcm_8khz_mono();
        let output_format = AudioFormat::pcm_16khz_mono();
        let converter = FormatConverter::new(input_format, output_format);
        assert!(converter.is_ok());
    }

    #[test]
    fn test_converter_unsupported_format() {
        let input_format = AudioFormat::new(8000, 1, 8, 20); // 8-bit (unsupported)
        let output_format = AudioFormat::pcm_16khz_mono();
        let converter = FormatConverter::new(input_format, output_format);
        assert!(converter.is_err());
    }

    #[test]
    fn test_formats_compatible() {
        let format1 = AudioFormat::pcm_8khz_mono();
        let format2 = AudioFormat::pcm_8khz_mono();
        let format3 = AudioFormat::pcm_16khz_mono();
        
        assert!(FormatConverter::formats_compatible(&format1, &format2));
        assert!(!FormatConverter::formats_compatible(&format1, &format3));
    }

    #[test]
    fn test_converter_properties() {
        let input_format = AudioFormat::pcm_8khz_mono();
        let output_format = AudioFormat::pcm_16khz_mono();
        let mut converter = FormatConverter::new(input_format.clone(), output_format.clone()).unwrap();
        
        // Test that converter was created successfully and can process frames
        let input_samples = vec![100, 200];
        let input_frame = AudioFrame::new(input_samples, input_format, 1000);
        let result = converter.convert_frame(&input_frame);
        assert!(result.is_ok());
    }
}

#[cfg(test)]
mod audio_frame_buffer_tests {
    use super::*;

    #[test]
    fn test_buffer_same_format() {
        let format = AudioFormat::pcm_8khz_mono();
        let mut buffer = AudioFrameBuffer::new(3, format.clone());
        
        // Add frames
        let mut successful_pushes = 0;
        for i in 0..4 {
            let samples = vec![i as i16; 160];
            let frame = AudioFrame::new(samples, format.clone(), i as u32);
            if buffer.push(frame) {
                successful_pushes += 1;
            }
        }
        
        // Should only accept max_frames
        assert_eq!(successful_pushes, 3);
        assert_eq!(buffer.len(), 3);
        
        // Pop frames in order they were added
        let frame1 = buffer.pop().unwrap();
        assert_eq!(frame1.timestamp, 0); // First frame added
        
        let frame2 = buffer.pop().unwrap();
        assert_eq!(frame2.timestamp, 1);
        
        assert_eq!(buffer.len(), 1);
    }

    #[test]
    fn test_buffer_frame_storage() {
        let format = AudioFormat::pcm_8khz_mono();
        let mut buffer = AudioFrameBuffer::new(2, format.clone());
        
        // Add frame with same format
        let input_samples = vec![100, 200, 300, 400];
        let input_frame = AudioFrame::new(input_samples.clone(), format, 1000);
        
        assert!(buffer.push(input_frame));
        assert_eq!(buffer.len(), 1);
        
        // Pop frame should be unchanged
        let retrieved_frame = buffer.pop().unwrap();
        assert_eq!(retrieved_frame.samples, input_samples);
        assert_eq!(retrieved_frame.timestamp, 1000);
    }

    #[test]
    fn test_buffer_statistics() {
        let format = AudioFormat::pcm_8khz_mono();
        let buffer = AudioFrameBuffer::new(5, format.clone());
        
        let stats = buffer.get_stats();
        assert_eq!(stats.current_frames, 0);
        assert_eq!(stats.max_frames, 5);
        assert_eq!(stats.format, format);
    }

    #[test]
    fn test_buffer_clear() {
        let format = AudioFormat::pcm_8khz_mono();
        let mut buffer = AudioFrameBuffer::new(5, format.clone());
        
        // Add frames
        for i in 0..2 {
            let samples = vec![i as i16; 160];
            let frame = AudioFrame::new(samples, format.clone(), i as u32);
            buffer.push(frame);
        }
        
        assert_eq!(buffer.len(), 2);
        
        buffer.clear();
        assert_eq!(buffer.len(), 0);
        assert!(buffer.is_empty());
    }

    #[test]
    fn test_buffer_full_empty() {
        let format = AudioFormat::pcm_8khz_mono();
        let mut buffer = AudioFrameBuffer::new(2, format.clone());
        
        assert!(buffer.is_empty());
        assert!(!buffer.is_full());
        
        // Add one frame
        let frame1 = AudioFrame::new(vec![100], format.clone(), 1);
        assert!(buffer.push(frame1));
        assert!(!buffer.is_empty());
        assert!(!buffer.is_full());
        
        // Add second frame
        let frame2 = AudioFrame::new(vec![200], format.clone(), 2);
        assert!(buffer.push(frame2));
        assert!(!buffer.is_empty());
        assert!(buffer.is_full());
        
        // Try to add third frame
        let frame3 = AudioFrame::new(vec![300], format, 3);
        assert!(!buffer.push(frame3)); // Should fail because buffer is full
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[test]
    fn test_mono_to_stereo_basic() {
        let input_format = AudioFormat::pcm_8khz_mono();
        let output_format = AudioFormat::new(8000, 2, 16, 20);
        let mut converter = FormatConverter::new(input_format.clone(), output_format).unwrap();
        
        let input_samples = vec![100, 200, 300];
        let input_frame = AudioFrame::new(input_samples, input_format, 1000);
        
        let result = converter.convert_frame(&input_frame);
        assert!(result.is_ok());
        
        let output_frame = result.unwrap();
        assert_eq!(output_frame.format.channels, 2);
        assert_eq!(output_frame.samples.len(), 6); // 3 mono samples -> 6 stereo samples
    }

    #[test]
    fn test_converter_error_handling() {
        let input_format = AudioFormat::pcm_8khz_mono();
        let output_format = AudioFormat::pcm_16khz_mono();
        let mut converter = FormatConverter::new(input_format.clone(), output_format).unwrap();
        
        // Try to convert frame with wrong format
        let wrong_format = AudioFormat::new(44100, 1, 16, 20);
        let wrong_samples = vec![100, 200, 300, 400];
        let wrong_frame = AudioFrame::new(wrong_samples, wrong_format, 1000);
        
        let result = converter.convert_frame(&wrong_frame);
        assert!(result.is_err());
        
        if let Err(AudioError::FormatConversionFailed { source_format, target_format, .. }) = result {
            assert!(source_format.contains("44100"));
            assert!(target_format.contains("16000"));
        } else {
            panic!("Expected FormatConversionFailed error");
        }
    }
}
