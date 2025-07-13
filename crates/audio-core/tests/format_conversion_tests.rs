//! Format conversion integration tests
//!
//! This module contains integration tests for audio format conversion functionality
//! including sample rate conversion, channel mapping, and frame buffering.

use rvoip_audio_core::{
    types::{AudioFormat, AudioFrame},
    format::{FormatConverter, AudioFrameBuffer, utils},
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
    fn test_sample_rate_upsampling() {
        let input_format = AudioFormat::pcm_8khz_mono();
        let output_format = AudioFormat::pcm_16khz_mono();
        let mut converter = FormatConverter::new(input_format.clone(), output_format).unwrap();
        
        let input_samples = vec![100, 200, 300, 400];
        let input_frame = AudioFrame::new(input_samples, input_format, 1000);
        
        let result = converter.convert_frame(&input_frame).unwrap();
        assert_eq!(result.format.sample_rate, 16000);
        // Upsampling should increase sample count
        assert!(result.samples.len() > 4);
        assert!(result.samples.len() <= 8); // Should be approximately double
    }

    #[test]
    fn test_sample_rate_downsampling() {
        let input_format = AudioFormat::pcm_16khz_mono();
        let output_format = AudioFormat::pcm_8khz_mono();
        let mut converter = FormatConverter::new(input_format.clone(), output_format).unwrap();
        
        let input_samples = vec![100, 150, 200, 250, 300, 350, 400, 450];
        let input_frame = AudioFrame::new(input_samples, input_format, 1000);
        
        let result = converter.convert_frame(&input_frame).unwrap();
        assert_eq!(result.format.sample_rate, 8000);
        // Downsampling should decrease sample count
        assert!(result.samples.len() < 8);
        assert!(result.samples.len() >= 3); // Should be approximately half
    }

    #[test]
    fn test_mono_to_stereo_conversion() {
        let input_format = AudioFormat::pcm_8khz_mono();
        let output_format = AudioFormat::new(8000, 2, 16, 20);
        let mut converter = FormatConverter::new(input_format.clone(), output_format).unwrap();
        
        let input_samples = vec![100, 200, 300];
        let input_frame = AudioFrame::new(input_samples, input_format, 1000);
        
        let result = converter.convert_frame(&input_frame).unwrap();
        assert_eq!(result.format.channels, 2);
        assert_eq!(result.samples, vec![100, 100, 200, 200, 300, 300]);
    }

    #[test]
    fn test_stereo_to_mono_conversion() {
        let input_format = AudioFormat::new(8000, 2, 16, 20);
        let output_format = AudioFormat::pcm_8khz_mono();
        let mut converter = FormatConverter::new(input_format.clone(), output_format).unwrap();
        
        let input_samples = vec![100, 200, 300, 400]; // Two stereo samples
        let input_frame = AudioFrame::new(input_samples, input_format, 1000);
        
        let result = converter.convert_frame(&input_frame).unwrap();
        assert_eq!(result.format.channels, 1);
        assert_eq!(result.samples, vec![150, 350]); // (100+200)/2, (300+400)/2
    }

    #[test]
    fn test_complex_conversion() {
        let input_format = AudioFormat::new(16000, 2, 16, 20); // 16kHz stereo
        let output_format = AudioFormat::pcm_8khz_mono(); // 8kHz mono
        let mut converter = FormatConverter::new(input_format.clone(), output_format).unwrap();
        
        let input_samples = vec![100, 200, 150, 250, 200, 300, 250, 350]; // 4 stereo samples
        let input_frame = AudioFrame::new(input_samples, input_format, 1000);
        
        let result = converter.convert_frame(&input_frame).unwrap();
        assert_eq!(result.format.sample_rate, 8000);
        assert_eq!(result.format.channels, 1);
        // Should have fewer samples due to both channel and sample rate conversion
        assert!(result.samples.len() >= 1 && result.samples.len() <= 4);
    }

    #[test]
    fn test_unsupported_bit_depth() {
        let input_format = AudioFormat::new(8000, 1, 24, 20); // 24-bit
        let output_format = AudioFormat::pcm_8khz_mono(); // 16-bit
        
        let result = FormatConverter::new(input_format, output_format);
        assert!(result.is_err());
        
        if let Err(AudioError::FormatConversionFailed { reason, .. }) = result {
            assert!(reason.contains("16-bit"));
        } else {
            panic!("Expected FormatConversionFailed error");
        }
    }

    #[test]
    fn test_converter_statistics() {
        let input_format = AudioFormat::pcm_8khz_mono();
        let output_format = AudioFormat::pcm_16khz_mono();
        let converter = FormatConverter::new(input_format.clone(), output_format.clone()).unwrap();
        
        let stats = converter.get_stats();
        assert_eq!(stats.input_format, input_format);
        assert_eq!(stats.output_format, output_format);
        assert_eq!(stats.sample_rate_ratio, 2.0);
    }

    #[test]
    fn test_converter_reset() {
        let input_format = AudioFormat::pcm_8khz_mono();
        let output_format = AudioFormat::pcm_16khz_mono();
        let mut converter = FormatConverter::new(input_format.clone(), output_format).unwrap();
        
        // Process a frame
        let input_samples = vec![100, 200];
        let input_frame = AudioFrame::new(input_samples, input_format, 1000);
        let _result = converter.convert_frame(&input_frame).unwrap();
        
        // Reset and verify
        converter.reset();
        // After reset, converter should work normally (we can't directly check internal state)
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
        for i in 0..4 {
            let samples = vec![i as i16; 160];
            let frame = AudioFrame::new(samples, format.clone(), i as u32);
            buffer.push_frame(frame).unwrap();
        }
        
        // Should only keep max_frames
        assert_eq!(buffer.len(), 3);
        
        // Pop frames
        let frame1 = buffer.pop_frame().unwrap();
        assert_eq!(frame1.timestamp, 1); // First frame was dropped
        
        let frame2 = buffer.pop_frame().unwrap();
        assert_eq!(frame2.timestamp, 2);
        
        assert_eq!(buffer.len(), 1);
    }

    #[test]
    fn test_buffer_format_conversion() {
        let target_format = AudioFormat::pcm_8khz_mono();
        let mut buffer = AudioFrameBuffer::new(2, target_format.clone());
        
        // Add frame with different format
        let input_format = AudioFormat::pcm_16khz_mono();
        let input_samples = vec![100, 200, 300, 400];
        let input_frame = AudioFrame::new(input_samples, input_format, 1000);
        
        buffer.push_frame(input_frame).unwrap();
        assert_eq!(buffer.len(), 1);
        
        // Pop frame should be in target format
        let converted_frame = buffer.pop_frame().unwrap();
        assert_eq!(converted_frame.format.sample_rate, 8000);
        assert!(converted_frame.samples.len() >= 1 && converted_frame.samples.len() <= 4);
    }

    #[test]
    fn test_buffer_statistics() {
        let format = AudioFormat::pcm_8khz_mono();
        let buffer = AudioFrameBuffer::new(5, format.clone());
        
        let stats = buffer.get_stats();
        assert_eq!(stats.current_frames, 0);
        assert_eq!(stats.max_frames, 5);
        assert_eq!(stats.target_format, format);
        assert!(!stats.converter_active);
    }

    #[test]
    fn test_buffer_clear() {
        let format = AudioFormat::pcm_8khz_mono();
        let mut buffer = AudioFrameBuffer::new(3, format.clone());
        
        // Add frames
        for i in 0..2 {
            let samples = vec![i as i16; 160];
            let frame = AudioFrame::new(samples, format.clone(), i as u32);
            buffer.push_frame(frame).unwrap();
        }
        
        assert_eq!(buffer.len(), 2);
        
        buffer.clear();
        assert_eq!(buffer.len(), 0);
        assert!(buffer.is_empty());
    }
}

#[cfg(test)]
mod conversion_utils_tests {
    use super::*;

    #[test]
    fn test_requires_conversion() {
        let format1 = AudioFormat::pcm_8khz_mono();
        let format2 = AudioFormat::pcm_8khz_mono();
        let format3 = AudioFormat::pcm_16khz_mono();
        
        assert!(!utils::requires_conversion(&format1, &format2));
        assert!(utils::requires_conversion(&format1, &format3));
    }

    #[test]
    fn test_conversion_complexity() {
        let simple = utils::conversion_complexity(
            &AudioFormat::pcm_8khz_mono(),
            &AudioFormat::pcm_8khz_mono()
        );
        assert_eq!(simple, 0);
        
        let sample_rate_only = utils::conversion_complexity(
            &AudioFormat::pcm_8khz_mono(),
            &AudioFormat::pcm_16khz_mono()
        );
        assert_eq!(sample_rate_only, 1);
        
        let channel_only = utils::conversion_complexity(
            &AudioFormat::pcm_8khz_mono(),
            &AudioFormat::new(8000, 2, 16, 20)
        );
        assert_eq!(channel_only, 1);
        
        let both = utils::conversion_complexity(
            &AudioFormat::pcm_8khz_mono(),
            &AudioFormat::new(16000, 2, 16, 20)
        );
        assert_eq!(both, 2);
        
        let extreme = utils::conversion_complexity(
            &AudioFormat::pcm_8khz_mono(),
            &AudioFormat::pcm_48khz_stereo()
        );
        assert!(extreme > 2);
    }

    #[test]
    fn test_create_optimal_converter() {
        let input = AudioFormat::pcm_8khz_mono();
        let output = AudioFormat::pcm_16khz_mono();
        
        let converter = utils::create_optimal_converter(input, output);
        assert!(converter.is_ok());
    }

    #[test]
    fn test_create_converter_unsupported_format() {
        let input = AudioFormat::new(8000, 1, 8, 20); // 8-bit (unsupported)
        let output = AudioFormat::new(48000, 8, 16, 10); // 16-bit but 8 channels (unsupported)
        
        let converter = utils::create_optimal_converter(input, output);
        assert!(converter.is_err());
        
        // The error should be about unsupported bit depth, not complexity
        if let Err(AudioError::FormatConversionFailed { reason, .. }) = converter {
            assert!(reason.contains("16-bit") || reason.contains("too complex"));
        } else {
            panic!("Expected FormatConversionFailed error");
        }
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[test]
    fn test_end_to_end_conversion_chain() {
        // Test a realistic conversion chain: device format → processing format → network format
        
        // Step 1: Device format (44.1kHz stereo) → Processing format (16kHz mono)
        let device_format = AudioFormat::new(44100, 2, 16, 20);
        let processing_format = AudioFormat::pcm_16khz_mono();
        let mut converter1 = FormatConverter::new(device_format.clone(), processing_format.clone()).unwrap();
        
        // Step 2: Processing format (16kHz mono) → Network format (8kHz mono)
        let network_format = AudioFormat::pcm_8khz_mono();
        let mut converter2 = FormatConverter::new(processing_format.clone(), network_format.clone()).unwrap();
        
        // Create test frame
        let device_samples = vec![100i16; device_format.samples_per_frame()];
        let device_frame = AudioFrame::new(device_samples, device_format, 1000);
        
        // Convert through chain
        let processing_frame = converter1.convert_frame(&device_frame).unwrap();
        assert_eq!(processing_frame.format.sample_rate, 16000);
        assert_eq!(processing_frame.format.channels, 1);
        
        let network_frame = converter2.convert_frame(&processing_frame).unwrap();
        assert_eq!(network_frame.format.sample_rate, 8000);
        assert_eq!(network_frame.format.channels, 1);
        
        // Verify final frame has expected properties
        assert!(network_frame.samples.len() > 0);
        assert_eq!(network_frame.timestamp, 1000);
    }

    #[test]
    fn test_buffered_conversion_chain() {
        // Test using AudioFrameBuffer for the conversion chain
        let device_format = AudioFormat::new(44100, 2, 16, 20);
        let network_format = AudioFormat::pcm_8khz_mono();
        
        let mut buffer = AudioFrameBuffer::new(5, network_format.clone());
        
        // Add several frames with device format
        for i in 0..3 {
            let samples = vec![(i * 100) as i16; device_format.samples_per_frame()];
            let frame = AudioFrame::new(samples, device_format.clone(), i as u32 * 1000);
            buffer.push_frame(frame).unwrap();
        }
        
        assert_eq!(buffer.len(), 3);
        
        // Pop frames should be converted to network format
        for i in 0..3 {
            let frame = buffer.pop_frame().unwrap();
            assert_eq!(frame.format.sample_rate, 8000);
            assert_eq!(frame.format.channels, 1);
            assert_eq!(frame.timestamp, i as u32 * 1000);
        }
        
        assert!(buffer.is_empty());
    }

    #[test]
    fn test_quality_preservation() {
        // Test that conversion preserves audio quality reasonably
        let input_format = AudioFormat::pcm_8khz_mono();
        let output_format = AudioFormat::pcm_16khz_mono();
        let mut converter = FormatConverter::new(input_format.clone(), output_format).unwrap();
        
        // Create a sine wave pattern
        let mut input_samples = Vec::new();
        for i in 0..160 { // 20ms at 8kHz
            let sample = (1000.0 * (2.0 * std::f64::consts::PI * i as f64 / 160.0).sin()) as i16;
            input_samples.push(sample);
        }
        
        let input_frame = AudioFrame::new(input_samples.clone(), input_format, 1000);
        let output_frame = converter.convert_frame(&input_frame).unwrap();
        
        // Check that output has roughly double the samples
        assert!(output_frame.samples.len() >= 300 && output_frame.samples.len() <= 340);
        
        // Check that RMS level is preserved approximately
        let input_rms = input_frame.rms_level();
        let output_rms = output_frame.rms_level();
        let rms_difference = (input_rms - output_rms).abs() / input_rms;
        assert!(rms_difference < 0.5); // Within 50% - reasonable for basic interpolation
    }

    #[test]
    fn test_conversion_error_handling() {
        let input_format = AudioFormat::pcm_8khz_mono();
        let output_format = AudioFormat::pcm_16khz_mono();
        let mut converter = FormatConverter::new(input_format.clone(), output_format).unwrap();
        
        // Try to convert frame with wrong format
        let wrong_format = AudioFormat::new(44100, 2, 16, 20);
        let wrong_samples = vec![100; wrong_format.samples_per_frame()];
        let wrong_frame = AudioFrame::new(wrong_samples, wrong_format, 1000);
        
        let result = converter.convert_frame(&wrong_frame);
        assert!(result.is_err());
        
        if let Err(AudioError::FormatConversionFailed { source_format, target_format, .. }) = result {
            assert!(source_format.contains("44100"));
            assert!(target_format.contains("8000"));
        } else {
            panic!("Expected FormatConversionFailed error");
        }
    }
} 