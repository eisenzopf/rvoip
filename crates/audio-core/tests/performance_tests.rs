//! Performance and benchmark tests for rvoip-audio-core
//!
//! This module contains performance tests and benchmarks to verify that
//! the audio-core library performs well under various conditions.

use rvoip_audio_core::{
    types::*,
    device::AudioDeviceManager,
    pipeline::AudioPipeline,
};
use std::time::{Duration, Instant};

#[cfg(test)]
mod audio_frame_performance {
    use super::*;

    #[test]
    fn test_audio_frame_creation_performance() {
        let format = AudioFormat::pcm_8khz_mono();
        let samples = vec![100i16; format.samples_per_frame()];
        
        let start = Instant::now();
        
        // Create many frames to test performance
        for i in 0..1000 {
            let _frame = AudioFrame::new(samples.clone(), format.clone(), i as u32);
        }
        
        let duration = start.elapsed();
        println!("Created 1000 frames in {:?}", duration);
        
        // Should be able to create frames quickly
        assert!(duration < Duration::from_millis(100));
    }

    #[test]
    fn test_audio_frame_rms_calculation_performance() {
        let format = AudioFormat::pcm_8khz_mono();
        let samples = vec![100i16; format.samples_per_frame()];
        let frame = AudioFrame::new(samples, format, 1000);
        
        let start = Instant::now();
        
        // Calculate RMS many times to test performance
        for _ in 0..10000 {
            let _rms = frame.rms_level();
        }
        
        let duration = start.elapsed();
        println!("Calculated RMS 10000 times in {:?}", duration);
        
        // RMS calculation should be fast
        assert!(duration < Duration::from_millis(100));
    }

    #[test]
    fn test_audio_frame_silence_detection_performance() {
        let format = AudioFormat::pcm_8khz_mono();
        let silent_frame = AudioFrame::silent(format.clone(), 1000);
        let loud_samples = vec![1000i16; format.samples_per_frame()];
        let loud_frame = AudioFrame::new(loud_samples, format, 1000);
        
        let start = Instant::now();
        
        // Test silence detection many times
        for _ in 0..10000 {
            let _is_silent = silent_frame.is_silent();
            let _is_loud = loud_frame.is_silent();
        }
        
        let duration = start.elapsed();
        println!("Performed silence detection 20000 times in {:?}", duration);
        
        // Silence detection should be fast
        assert!(duration < Duration::from_millis(50));
    }

    #[test]
    fn test_session_core_conversion_performance() {
        let format = AudioFormat::pcm_8khz_mono();
        let samples = vec![100i16; format.samples_per_frame()];
        let frame = AudioFrame::new(samples, format, 1000);
        
        let start = Instant::now();
        
        // Test conversion performance
        for _ in 0..1000 {
            let session_frame = frame.to_session_core();
            let _converted_back = AudioFrame::from_session_core(&session_frame, 20);
        }
        
        let duration = start.elapsed();
        println!("Performed 1000 round-trip conversions in {:?}", duration);
        
        // Conversions should be reasonably fast
        assert!(duration < Duration::from_millis(200));
    }
}

#[cfg(test)]
mod format_performance {
    use super::*;

    #[test]
    fn test_format_calculations_performance() {
        let format = AudioFormat::pcm_8khz_mono();
        
        let start = Instant::now();
        
        // Test format calculations many times
        for _ in 0..100000 {
            let _samples = format.samples_per_frame();
            let _bytes = format.bytes_per_frame();
            let _desc = format.description();
            let _is_voip = format.is_voip_suitable();
        }
        
        let duration = start.elapsed();
        println!("Performed 400000 format calculations in {:?}", duration);
        
        // Format calculations should be very fast
        assert!(duration < Duration::from_millis(100));
    }

    #[test]
    fn test_format_compatibility_performance() {
        let format1 = AudioFormat::pcm_8khz_mono();
        let format2 = AudioFormat::pcm_16khz_mono();
        let format3 = AudioFormat::pcm_48khz_stereo();
        
        let start = Instant::now();
        
        // Test compatibility checks many times
        for _ in 0..100000 {
            let _comp1 = format1.is_compatible_with(&format2);
            let _comp2 = format2.is_compatible_with(&format3);
            let _comp3 = format3.is_compatible_with(&format1);
        }
        
        let duration = start.elapsed();
        println!("Performed 300000 compatibility checks in {:?}", duration);
        
        // Compatibility checks should be very fast
        assert!(duration < Duration::from_millis(50));
    }
}

#[cfg(test)]
mod device_performance {
    use super::*;

    #[tokio::test]
    async fn test_device_manager_creation_performance() {
        let start = Instant::now();
        
        // Create multiple device managers
        for _ in 0..10 {
            let _manager = AudioDeviceManager::new().await.unwrap();
        }
        
        let duration = start.elapsed();
        println!("Created 10 device managers in {:?}", duration);
        
        // Device manager creation should be reasonably fast
        assert!(duration < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn test_device_enumeration_performance() {
        let manager = AudioDeviceManager::new().await.unwrap();
        
        let start = Instant::now();
        
        // Enumerate devices multiple times
        for _ in 0..100 {
            let _input_devices = manager.list_devices(AudioDirection::Input).await.unwrap();
            let _output_devices = manager.list_devices(AudioDirection::Output).await.unwrap();
        }
        
        let duration = start.elapsed();
        println!("Performed 200 device enumerations in {:?}", duration);
        
        // Device enumeration should be reasonably fast
        assert!(duration < Duration::from_secs(10));
    }

    #[tokio::test]
    async fn test_default_device_access_performance() {
        let manager = AudioDeviceManager::new().await.unwrap();
        
        let start = Instant::now();
        
        // Access default devices multiple times
        for _ in 0..100 {
            let _input_device = manager.get_default_device(AudioDirection::Input).await.unwrap();
            let _output_device = manager.get_default_device(AudioDirection::Output).await.unwrap();
        }
        
        let duration = start.elapsed();
        println!("Accessed default devices 200 times in {:?}", duration);
        
        // Default device access should be reasonably fast
        assert!(duration < Duration::from_secs(5));
    }
}

#[cfg(test)]
mod pipeline_performance {
    use super::*;

    #[tokio::test]
    async fn test_pipeline_creation_performance() {
        let input_format = AudioFormat::pcm_8khz_mono();
        let output_format = AudioFormat::pcm_16khz_mono();
        
        let start = Instant::now();
        
        // Create multiple pipelines
        let device_manager = AudioDeviceManager::new().await.unwrap();
        for _ in 0..100 {
            let _pipeline = AudioPipeline::builder()
                .input_format(input_format.clone())
                .output_format(output_format.clone())
                .device_manager(device_manager.clone())
                .build()
                .await
                .unwrap();
        }
        
        let duration = start.elapsed();
        println!("Created 100 pipelines in {:?}", duration);
        
        // Pipeline creation should be reasonably fast
        assert!(duration < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn test_pipeline_startup_performance() {
        let input_format = AudioFormat::pcm_8khz_mono();
        let output_format = AudioFormat::pcm_16khz_mono();
        
        let start = Instant::now();
        
        // Create and start multiple pipelines
        let device_manager = AudioDeviceManager::new().await.unwrap();
        for _ in 0..10 {
            let mut pipeline = AudioPipeline::builder()
                .input_format(input_format.clone())
                .output_format(output_format.clone())
                .device_manager(device_manager.clone())
                .build()
                .await
                .unwrap();
            
            let _result = pipeline.start().await.unwrap();
        }
        
        let duration = start.elapsed();
        println!("Started 10 pipelines in {:?}", duration);
        
        // Pipeline startup should be reasonably fast
        assert!(duration < Duration::from_secs(10));
    }
}

#[cfg(test)]
mod codec_performance {
    use super::*;

    #[test]
    fn test_codec_property_access_performance() {
        let codecs = vec![
            AudioCodec::G711U,
            AudioCodec::G711A,
            AudioCodec::G722,
            AudioCodec::Opus,
            AudioCodec::PCM,
        ];
        
        let start = Instant::now();
        
        // Access codec properties many times
        for _ in 0..10000 {
            for codec in &codecs {
                let _name = codec.name();
                let _payload_type = codec.payload_type();
                let _sample_rate = codec.typical_sample_rate();
                let _supports_vbr = codec.supports_vbr();
            }
        }
        
        let duration = start.elapsed();
        println!("Accessed codec properties 200000 times in {:?}", duration);
        
        // Codec property access should be very fast
        assert!(duration < Duration::from_millis(100));
    }
}

#[cfg(test)]
mod memory_performance {
    use super::*;

    #[test]
    fn test_audio_frame_memory_usage() {
        let format = AudioFormat::pcm_8khz_mono();
        let samples = vec![100i16; format.samples_per_frame()];
        
        // Create many frames to test memory usage
        let mut frames = Vec::new();
        for i in 0..1000 {
            frames.push(AudioFrame::new(samples.clone(), format.clone(), i as u32));
        }
        
        // Verify all frames are created
        assert_eq!(frames.len(), 1000);
        
        // Each frame should have the correct number of samples
        for frame in &frames {
            assert_eq!(frame.samples.len(), format.samples_per_frame());
        }
    }

    #[test]
    fn test_format_memory_usage() {
        // Create many formats to test memory usage
        let mut formats = Vec::new();
        for i in 0..1000 {
            formats.push(AudioFormat::new(8000 + i, 1, 16, 20));
        }
        
        // Verify all formats are created
        assert_eq!(formats.len(), 1000);
        
        // Each format should have unique sample rate
        for (i, format) in formats.iter().enumerate() {
            assert_eq!(format.sample_rate, 8000 + i as u32);
        }
    }
}

#[cfg(test)]
mod stress_tests {
    use super::*;

    #[tokio::test]
    async fn test_concurrent_device_access() {
        use tokio::task;
        
        let manager = AudioDeviceManager::new().await.unwrap();
        let manager = std::sync::Arc::new(manager);
        
        let mut handles = Vec::new();
        
        // Create multiple concurrent tasks accessing devices
        for _ in 0..10 {
            let manager_clone = manager.clone();
            let handle = task::spawn(async move {
                for _ in 0..10 {
                    let _input_device = manager_clone.get_default_device(AudioDirection::Input).await.unwrap();
                    let _output_device = manager_clone.get_default_device(AudioDirection::Output).await.unwrap();
                }
            });
            handles.push(handle);
        }
        
        // Wait for all tasks to complete
        for handle in handles {
            handle.await.unwrap();
        }
    }

    #[tokio::test]
    async fn test_concurrent_pipeline_creation() {
        use tokio::task;
        
        let mut handles = Vec::new();
        
        // Create multiple concurrent tasks creating pipelines
        let device_manager = std::sync::Arc::new(AudioDeviceManager::new().await.unwrap());
        for _ in 0..10 {
            let manager_clone = device_manager.clone();
            let handle = task::spawn(async move {
                for _ in 0..10 {
                    let _pipeline = AudioPipeline::builder()
                        .input_format(AudioFormat::pcm_8khz_mono())
                        .output_format(AudioFormat::pcm_16khz_mono())
                        .device_manager((*manager_clone).clone())
                        .build()
                        .await
                        .unwrap();
                }
            });
            handles.push(handle);
        }
        
        // Wait for all tasks to complete
        for handle in handles {
            handle.await.unwrap();
        }
    }

    #[test]
    fn test_large_audio_frame_processing() {
        // Test with a large audio frame
        let format = AudioFormat::new(48000, 2, 16, 100); // 100ms frame
        let samples = vec![100i16; format.samples_per_frame()];
        
        let start = Instant::now();
        
        let frame = AudioFrame::new(samples, format, 1000);
        let _rms = frame.rms_level();
        let _is_silent = frame.is_silent();
        let _session_frame = frame.to_session_core();
        
        let duration = start.elapsed();
        println!("Processed large frame in {:?}", duration);
        
        // Should handle large frames reasonably well
        assert!(duration < Duration::from_millis(10));
    }
} 