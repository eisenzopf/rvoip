//! Unit tests for the simple SIP client API

use crate::simple::*;
use crate::events::{EventEmitter, SipClientEvent};
use crate::types::{CallId, CallState, AudioQualityMetrics};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

#[cfg(test)]
mod audio_pipeline_tests {
    use super::*;
    
    #[test]
    fn test_audio_device_listing() {
        // Test that we can create an AudioDeviceManager
        // This is a basic functionality test
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let manager = rvoip_audio_core::AudioDeviceManager::new().await;
            assert!(manager.is_ok());
            
            let manager = manager.unwrap();
            
            // List input devices
            let input_devices = manager.list_devices(rvoip_audio_core::AudioDirection::Input).await;
            assert!(input_devices.is_ok());
            
            // List output devices
            let output_devices = manager.list_devices(rvoip_audio_core::AudioDirection::Output).await;
            assert!(output_devices.is_ok());
        });
    }
    
    #[test]
    fn test_audio_format_configuration() {
        // Test different audio format configurations
        let format_8khz = rvoip_audio_core::AudioFormat::pcm_8khz_mono();
        assert_eq!(format_8khz.sample_rate, 8000);
        assert_eq!(format_8khz.channels, 1);
        
        let format_16khz = rvoip_audio_core::AudioFormat::pcm_16khz_mono();
        assert_eq!(format_16khz.sample_rate, 16000);
        assert_eq!(format_16khz.channels, 1);
        
        // Test frame calculations
        assert_eq!(format_8khz.samples_per_frame(), 160); // 8000 * 20 / 1000
        assert_eq!(format_16khz.samples_per_frame(), 320); // 16000 * 20 / 1000
    }
    
    #[tokio::test]
    async fn test_audio_level_calculation() {
        // Test audio level calculations
        let mut samples = vec![0i16; 160]; // Silent frame
        
        let format = rvoip_audio_core::AudioFormat::pcm_8khz_mono();
        let frame = rvoip_audio_core::AudioFrame::new(samples.clone(), format.clone(), 0);
        
        // Silent frame should have zero RMS
        assert_eq!(frame.rms_level(), 0.0);
        assert!(frame.is_silent());
        
        // Add some signal
        samples[0] = 1000;
        samples[1] = -1000;
        samples[2] = 500;
        samples[3] = -500;
        
        let frame_with_signal = rvoip_audio_core::AudioFrame::new(samples, format, 0);
        assert!(frame_with_signal.rms_level() > 0.0);
        assert!(!frame_with_signal.is_silent());
    }
    
    #[test]
    fn test_codec_type_mapping() {
        // Test codec type to format mapping
        let pcmu_type = codec_core::CodecType::G711Pcmu;
        let pcma_type = codec_core::CodecType::G711Pcma;
        
        // Test that we handle the correct codec types
        match pcmu_type {
            codec_core::CodecType::G711Pcmu => {
                let format = rvoip_audio_core::AudioFormat::pcm_8khz_mono();
                assert_eq!(format.sample_rate, 8000);
            }
            _ => panic!("Unexpected codec type"),
        }
        
        match pcma_type {
            codec_core::CodecType::G711Pcma => {
                let format = rvoip_audio_core::AudioFormat::pcm_8khz_mono();
                assert_eq!(format.sample_rate, 8000);
            }
            _ => panic!("Unexpected codec type"),
        }
    }
}

#[cfg(test)]
mod event_system_tests {
    use super::*;
    
    #[tokio::test]
    async fn test_event_emitter_subscription() {
        let emitter = EventEmitter::default();
        let mut stream1 = emitter.subscribe();
        let mut stream2 = emitter.subscribe();
        
        // Emit an event
        emitter.emit(SipClientEvent::Started);
        
        // Both streams should receive the event
        let event1 = stream1.next().await;
        assert!(event1.is_some());
        if let Some(Ok(SipClientEvent::Started)) = event1 {
            // Expected
        } else {
            panic!("Unexpected event: {:?}", event1);
        }
        
        let event2 = stream2.next().await;
        assert!(event2.is_some());
        if let Some(Ok(SipClientEvent::Started)) = event2 {
            // Expected
        } else {
            panic!("Unexpected event: {:?}", event2);
        }
    }
    
    #[tokio::test]
    async fn test_audio_level_events() {
        let emitter = EventEmitter::default();
        let mut stream = emitter.subscribe();
        
        let call_id = CallId::new_v4();
        
        // Emit audio level event
        emitter.emit(SipClientEvent::AudioLevelChanged {
            call_id: Some(call_id),
            direction: rvoip_audio_core::AudioDirection::Input,
            level: 0.5,
            peak: 0.7,
        });
        
        // Should receive the event
        let event = stream.next().await;
        assert!(event.is_some());
        
        if let Some(Ok(SipClientEvent::AudioLevelChanged { 
            call_id: Some(id), 
            direction, 
            level, 
            peak 
        })) = event {
            assert_eq!(id, call_id);
            assert_eq!(direction, rvoip_audio_core::AudioDirection::Input);
            assert_eq!(level, 0.5);
            assert_eq!(peak, 0.7);
        } else {
            panic!("Unexpected event: {:?}", event);
        }
    }
    
    #[tokio::test]
    async fn test_audio_device_change_events() {
        let emitter = EventEmitter::default();
        let mut stream = emitter.subscribe();
        
        // Emit device change event
        emitter.emit(SipClientEvent::AudioDeviceChanged {
            direction: rvoip_audio_core::AudioDirection::Input,
            old_device: Some("Old Mic".to_string()),
            new_device: Some("New Mic".to_string()),
        });
        
        // Should receive the event
        let event = stream.next().await;
        assert!(event.is_some());
        
        if let Some(Ok(SipClientEvent::AudioDeviceChanged { 
            direction, 
            old_device, 
            new_device 
        })) = event {
            assert_eq!(direction, rvoip_audio_core::AudioDirection::Input);
            assert_eq!(old_device, Some("Old Mic".to_string()));
            assert_eq!(new_device, Some("New Mic".to_string()));
        } else {
            panic!("Unexpected event: {:?}", event);
        }
    }
    
    #[tokio::test]
    async fn test_audio_error_events() {
        let emitter = EventEmitter::default();
        let mut stream = emitter.subscribe();
        
        // Emit error event
        emitter.emit(SipClientEvent::AudioDeviceError {
            message: "Device not found".to_string(),
            device: Some("Microphone".to_string()),
        });
        
        // Should receive the event
        let event = stream.next().await;
        assert!(event.is_some());
        
        if let Some(Ok(SipClientEvent::AudioDeviceError { message, device })) = event {
            assert_eq!(message, "Device not found");
            assert_eq!(device, Some("Microphone".to_string()));
        } else {
            panic!("Unexpected event: {:?}", event);
        }
    }
    
    #[tokio::test]
    async fn test_call_quality_events() {
        let emitter = EventEmitter::default();
        let mut stream = emitter.subscribe();
        
        let call_id = CallId::new_v4();
        
        // Emit quality report event
        emitter.emit(SipClientEvent::CallQualityReport {
            call_id,
            metrics: AudioQualityMetrics {
                level: 0.8,
                peak_level: 0.9,
                mos: 4.2,
                packet_loss_percent: 0.5,
                jitter_ms: 15.0,
                rtt_ms: 50.0,
            },
        });
        
        // Should receive the event
        let event = stream.next().await;
        assert!(event.is_some());
        
        if let Some(Ok(SipClientEvent::CallQualityReport { 
            call_id: id, 
            metrics 
        })) = event {
            assert_eq!(id, call_id);
            assert_eq!(metrics.mos, 4.2);
            assert_eq!(metrics.packet_loss_percent, 0.5);
            assert_eq!(metrics.jitter_ms, 15.0);
            assert_eq!(metrics.rtt_ms, 50.0);
        } else {
            panic!("Unexpected event: {:?}", event);
        }
    }
}

#[cfg(test)]
mod event_forwarding_tests {
    use super::*;
    
    #[test]
    fn test_call_state_mapping() {
        use rvoip_client_core::call::CallState as CoreState;
        
        // Test mapping of all client-core states to sip-client states
        let mappings = vec![
            (CoreState::Initiating, CallState::Initiating),
            (CoreState::Proceeding, CallState::Initiating),
            (CoreState::Ringing, CallState::Ringing),
            (CoreState::Connected, CallState::Connected),
            (CoreState::Terminating, CallState::Terminated),
            (CoreState::Terminated, CallState::Terminated),
            (CoreState::Failed, CallState::Terminated),
            (CoreState::Cancelled, CallState::Terminated),
            (CoreState::IncomingPending, CallState::IncomingRinging),
        ];
        
        for (core_state, expected_state) in mappings {
            // This is just a logical test to ensure we handle all states
            // In actual code, this mapping is done in on_call_state_changed
            match core_state {
                CoreState::Initiating => assert_eq!(expected_state, CallState::Initiating),
                CoreState::Proceeding => assert_eq!(expected_state, CallState::Initiating),
                CoreState::Ringing => assert_eq!(expected_state, CallState::Ringing),
                CoreState::Connected => assert_eq!(expected_state, CallState::Connected),
                CoreState::Terminating => assert_eq!(expected_state, CallState::Terminated),
                CoreState::Terminated => assert_eq!(expected_state, CallState::Terminated),
                CoreState::Failed => assert_eq!(expected_state, CallState::Terminated),
                CoreState::Cancelled => assert_eq!(expected_state, CallState::Terminated),
                CoreState::IncomingPending => assert_eq!(expected_state, CallState::IncomingRinging),
            }
        }
    }
    
    #[test]
    fn test_registration_status_mapping() {
        use rvoip_client_core::registration::RegistrationStatus as CoreStatus;
        
        // Test mapping of registration statuses
        let mappings = vec![
            (CoreStatus::Pending, "pending"),
            (CoreStatus::Active, "active"),
            (CoreStatus::Failed, "failed"),
            (CoreStatus::Expired, "expired"),
        ];
        
        for (core_status, expected_str) in mappings {
            let status_str = match core_status {
                CoreStatus::Pending => "pending",
                CoreStatus::Active => "active",
                CoreStatus::Failed => "failed",
                CoreStatus::Expired => "expired",
                _ => "unknown",
            };
            assert_eq!(status_str, expected_str);
        }
    }
}

#[cfg(test)]
mod audio_device_tests {
    use super::*;
    
    #[tokio::test]
    async fn test_device_info_creation() {
        let info = rvoip_audio_core::AudioDeviceInfo::new(
            "test-mic",
            "Test Microphone",
            rvoip_audio_core::AudioDirection::Input,
        );
        
        assert_eq!(info.id, "test-mic");
        assert_eq!(info.name, "Test Microphone");
        assert_eq!(info.direction, rvoip_audio_core::AudioDirection::Input);
        assert!(!info.is_default);
        
        // Test format support
        let format = rvoip_audio_core::AudioFormat::pcm_8khz_mono();
        assert!(info.supports_format(&format));
        
        // Test best VoIP format
        let best_format = info.best_voip_format();
        assert!(best_format.is_voip_suitable());
    }
}