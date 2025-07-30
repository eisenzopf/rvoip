//! Integration tests for rvoip-audio-core
//!
//! This module contains integration tests that verify the core functionality
//! of the audio-core library including types, device management, format conversion,
//! and pipeline operations.

use rvoip_audio_core::{
    types::*,
    device::AudioDeviceManager,
    pipeline::AudioPipeline,
    error::{AudioError, AudioResult},
};

#[cfg(test)]
mod types_tests {
    use super::*;

    #[test]
    fn test_audio_format_basic() {
        let format = AudioFormat::pcm_8khz_mono();
        assert_eq!(format.sample_rate, 8000);
        assert_eq!(format.channels, 1);
        assert_eq!(format.bits_per_sample, 16);
        assert_eq!(format.frame_size_ms, 20);
        assert_eq!(format.samples_per_frame(), 160);
        assert_eq!(format.bytes_per_frame(), 320);
    }

    #[test]
    fn test_audio_format_voip_suitable() {
        let voip_format = AudioFormat::pcm_8khz_mono();
        assert!(voip_format.is_voip_suitable());

        let wideband_format = AudioFormat::pcm_16khz_mono();
        assert!(wideband_format.is_voip_suitable());

        let hifi_format = AudioFormat::pcm_48khz_stereo();
        assert!(!hifi_format.is_voip_suitable());
    }

    #[test]
    fn test_audio_format_compatibility() {
        let format1 = AudioFormat::pcm_8khz_mono();
        let format2 = AudioFormat::pcm_8khz_mono();
        let format3 = AudioFormat::pcm_16khz_mono();

        assert!(format1.is_compatible_with(&format2));
        assert!(!format1.is_compatible_with(&format3));
    }

    #[test]
    fn test_audio_frame_creation() {
        let format = AudioFormat::pcm_8khz_mono();
        let samples = vec![0i16; format.samples_per_frame()];
        let frame = AudioFrame::new(samples, format, 1000);

        assert_eq!(frame.samples.len(), 160);
        assert_eq!(frame.timestamp, 1000);
        assert!(frame.is_silent());
    }

    #[test]
    fn test_audio_frame_silence_detection() {
        let format = AudioFormat::pcm_8khz_mono();
        
        // Test silent frame
        let silent_frame = AudioFrame::silent(format.clone(), 1000);
        assert!(silent_frame.is_silent());

        // Test non-silent frame
        let loud_samples = vec![1000i16; format.samples_per_frame()];
        let loud_frame = AudioFrame::new(loud_samples, format, 1000);
        assert!(!loud_frame.is_silent());
    }

    #[test]
    fn test_audio_frame_rms_level() {
        let format = AudioFormat::pcm_8khz_mono();
        
        // Test silent frame
        let silent_frame = AudioFrame::silent(format.clone(), 1000);
        assert_eq!(silent_frame.rms_level(), 0.0);

        // Test frame with known values
        let samples = vec![100i16; format.samples_per_frame()];
        let frame = AudioFrame::new(samples, format, 1000);
        assert!(frame.rms_level() > 0.0);
    }

    #[test]
    fn test_audio_device_info() {
        let device = AudioDeviceInfo::new("test-id", "Test Device", AudioDirection::Input);
        assert_eq!(device.id, "test-id");
        assert_eq!(device.name, "Test Device");
        assert_eq!(device.direction, AudioDirection::Input);
        assert!(!device.is_default);

        let format = AudioFormat::pcm_8khz_mono();
        assert!(device.supports_format(&format));

        let best_format = device.best_voip_format();
        assert!(best_format.is_voip_suitable());
    }


    #[test]
    fn test_audio_stream_config() {
        let config = AudioStreamConfig::voip_basic();
        assert_eq!(config.input_format.sample_rate, 8000);
        assert_eq!(config.codec_name, "PCMU");
        assert!(config.enable_aec);
        assert!(config.enable_agc);

        let hq_config = AudioStreamConfig::voip_high_quality();
        assert_eq!(hq_config.input_format.sample_rate, 48000);
        assert_eq!(hq_config.codec_name, "opus");
        assert!(hq_config.enable_noise_suppression);
        assert!(hq_config.enable_vad);
    }

    #[test]
    fn test_audio_quality_metrics() {
        let mut metrics = AudioQualityMetrics::new();
        assert_eq!(metrics.mos_score, 0.0);
        assert_eq!(metrics.packet_loss_percent, 0.0);
        assert!(!metrics.is_acceptable());

        metrics.mos_score = 4.2;
        metrics.packet_loss_percent = 1.0;
        metrics.jitter_ms = 15.0;
        
        assert!(metrics.is_acceptable());
        assert_eq!(metrics.quality_rating(), "Good");
    }

    #[test]
    fn test_session_core_integration() {
        let format = AudioFormat::pcm_8khz_mono();
        let samples = vec![100i16; format.samples_per_frame()];
        let frame = AudioFrame::new(samples.clone(), format.clone(), 1000);

        // Test conversion to session-core format
        let session_frame = frame.to_session_core();
        assert_eq!(session_frame.samples, samples);
        assert_eq!(session_frame.sample_rate, 8000);
        assert_eq!(session_frame.channels, 1);
        assert_eq!(session_frame.timestamp, 1000);

        // Test conversion from session-core format
        let converted_frame = AudioFrame::from_session_core(&session_frame, 20);
        assert_eq!(converted_frame.samples, samples);
        assert_eq!(converted_frame.format.sample_rate, 8000);
        assert_eq!(converted_frame.format.channels, 1);
        assert_eq!(converted_frame.timestamp, 1000);
    }
}

#[cfg(test)]
mod device_tests {
    use super::*;

    #[tokio::test]
    async fn test_device_manager_creation() {
        let manager = AudioDeviceManager::new().await;
        assert!(manager.is_ok());
    }

    #[tokio::test]
    async fn test_device_enumeration() {
        let manager = AudioDeviceManager::new().await.unwrap();
        
        // Test listing input devices
        let input_devices = manager.list_devices(AudioDirection::Input).await;
        assert!(input_devices.is_ok());
        
        // Test listing output devices
        let output_devices = manager.list_devices(AudioDirection::Output).await;
        assert!(output_devices.is_ok());
    }

    #[tokio::test]
    async fn test_default_device_access() {
        let manager = AudioDeviceManager::new().await.unwrap();
        
        // Test getting default input device
        let input_device = manager.get_default_device(AudioDirection::Input).await;
        assert!(input_device.is_ok());
        
        // Test getting default output device
        let output_device = manager.get_default_device(AudioDirection::Output).await;
        assert!(output_device.is_ok());
    }

    #[tokio::test]
    async fn test_device_format_support() {
        let manager = AudioDeviceManager::new().await.unwrap();
        let device = manager.get_default_device(AudioDirection::Input).await.unwrap();
        
        let format = AudioFormat::pcm_8khz_mono();
        assert!(device.supports_format(&format));
        
        let info = device.info();
        assert!(!info.id.is_empty());
        assert!(!info.name.is_empty());
        assert_eq!(info.direction, AudioDirection::Input);
    }
}

#[cfg(test)]
mod pipeline_tests {
    use super::*;

    #[tokio::test]
    async fn test_pipeline_builder() {
        let input_format = AudioFormat::pcm_8khz_mono();
        let output_format = AudioFormat::pcm_16khz_mono();
        let device_manager = AudioDeviceManager::new().await.unwrap();

        let pipeline = AudioPipeline::builder()
            .input_format(input_format)
            .output_format(output_format)
            .device_manager(device_manager)
            .build()
            .await;

        assert!(pipeline.is_ok());
    }

    #[tokio::test]
    async fn test_pipeline_operations() {
        let device_manager = AudioDeviceManager::new().await.unwrap();
        let pipeline = AudioPipeline::builder()
            .input_format(AudioFormat::pcm_8khz_mono())
            .output_format(AudioFormat::pcm_16khz_mono())
            .device_manager(device_manager)
            .build()
            .await;

        assert!(pipeline.is_ok());
        
        let mut pipeline = pipeline.unwrap();
        let start_result = pipeline.start().await;
        assert!(start_result.is_ok());
    }
}

#[cfg(test)]
mod error_tests {
    use super::*;

    #[test]
    fn test_error_creation() {
        let device_error = AudioError::device_not_found("test-device");
        assert!(matches!(device_error, AudioError::DeviceNotFound { .. }));


        let buffer_error = AudioError::buffer_error("capture", "overflow", "buffer full");
        assert!(matches!(buffer_error, AudioError::BufferError { .. }));
    }

    #[test]
    fn test_error_recoverability() {
        let device_error = AudioError::device_not_found("test-device");
        assert!(!device_error.is_recoverable());

        let buffer_error = AudioError::buffer_error("capture", "overflow", "buffer full");
        assert!(buffer_error.is_recoverable());

        let format_error = AudioError::FormatConversionFailed {
            source_format: "8kHz".to_string(),
            target_format: "16kHz".to_string(),
            reason: "unsupported".to_string(),
        };
        assert!(format_error.is_recoverable());
    }

    #[test]
    fn test_error_user_messages() {
        let device_error = AudioError::device_not_found("test-device");
        let message = device_error.user_friendly_message();
        assert!(message.contains("device"));
        assert!(message.contains("not found"));

    }
}

#[cfg(test)]
mod defaults_tests {
    use super::*;
    use rvoip_audio_core::defaults;

    #[test]
    fn test_default_formats() {
        let voip_format = defaults::voip_format();
        assert_eq!(voip_format.sample_rate, defaults::SAMPLE_RATE_NARROWBAND);
        assert_eq!(voip_format.channels, defaults::CHANNELS);
        assert_eq!(voip_format.bits_per_sample, defaults::BIT_DEPTH);
        assert_eq!(voip_format.frame_size_ms, defaults::FRAME_SIZE_MS);
        assert!(voip_format.is_voip_suitable());

        let wideband_format = defaults::wideband_format();
        assert_eq!(wideband_format.sample_rate, defaults::SAMPLE_RATE_WIDEBAND);
        assert!(wideband_format.is_voip_suitable());

        let hifi_format = defaults::hifi_format();
        assert_eq!(hifi_format.sample_rate, defaults::SAMPLE_RATE_HIFI);
        assert!(!hifi_format.is_voip_suitable());
    }

    #[test]
    fn test_default_constants() {
        assert_eq!(defaults::SAMPLE_RATE_NARROWBAND, 8000);
        assert_eq!(defaults::SAMPLE_RATE_WIDEBAND, 16000);
        assert_eq!(defaults::SAMPLE_RATE_HIFI, 48000);
        assert_eq!(defaults::FRAME_SIZE_MS, 20);
        assert_eq!(defaults::CHANNELS, 1);
        assert_eq!(defaults::BIT_DEPTH, 16);
    }
}


#[cfg(test)]
mod integration_tests {
    use super::*;

    #[tokio::test]
    async fn test_end_to_end_device_to_pipeline() {
        // Test creating a device manager and connecting it to a pipeline
        let device_manager = AudioDeviceManager::new().await.unwrap();
        let input_device = device_manager.get_default_device(AudioDirection::Input).await.unwrap();
        
        let device_format = input_device.info().best_voip_format();
        
        let pipeline = AudioPipeline::builder()
            .input_format(device_format)
            .output_format(AudioFormat::pcm_8khz_mono())
            .device_manager(device_manager)
            .build()
            .await;
        
        assert!(pipeline.is_ok());
    }

    #[test]
    fn test_format_conversion_chain() {
        // Test a chain of format conversions
        let start_format = AudioFormat::pcm_8khz_mono();
        let middle_format = AudioFormat::pcm_16khz_mono();
        let end_format = AudioFormat::pcm_48khz_stereo();

        // Create frames with different formats
        let frame1 = AudioFrame::silent(start_format, 1000);
        let frame2 = AudioFrame::silent(middle_format, 2000);
        let frame3 = AudioFrame::silent(end_format, 3000);

        // Verify frame properties
        assert_eq!(frame1.format.sample_rate, 8000);
        assert_eq!(frame2.format.sample_rate, 16000);
        assert_eq!(frame3.format.sample_rate, 48000);
        assert_eq!(frame3.format.channels, 2);
    }

} 