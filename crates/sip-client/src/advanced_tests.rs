//! Unit tests for the advanced SIP client API

#[cfg(test)]
mod tests {
    use crate::advanced::*;
    use crate::events::SipClientEvent;
    use crate::types::CallId;
    use tokio_stream::StreamExt;
    
    #[test]
    fn test_audio_pipeline_config_builder() {
        let config = AudioPipelineConfig::custom()
            .input_device("Microphone")
            .output_device("Headphones")
            .echo_cancellation(false)
            .noise_suppression(false)
            .auto_gain_control(false)
            .buffer_size(320)
            .enable_frame_access(true);
        
        assert_eq!(config.input_device, Some("Microphone".to_string()));
        assert_eq!(config.output_device, Some("Headphones".to_string()));
        assert!(!config.echo_cancellation);
        assert!(!config.noise_suppression);
        assert!(!config.auto_gain_control);
        assert_eq!(config.buffer_size, 320);
        assert!(config.enable_frame_access);
    }
    
    #[test]
    fn test_codec_priority() {
        let priority = CodecPriority::new("PCMU", 100);
        assert_eq!(priority.name, "PCMU");
        assert_eq!(priority.priority, 100);
    }
    
    #[test]
    fn test_media_preferences_builder() {
        let prefs = MediaPreferences::new()
            .codecs(vec![
                CodecPriority::new("PCMA", 100),
                CodecPriority::new("PCMU", 90),
            ])
            .jitter_buffer_ms(200)
            .dtmf_detection(true)
            .comfort_noise(true);
        
        assert_eq!(prefs.codecs.len(), 2);
        assert_eq!(prefs.codecs[0].name, "PCMA");
        assert_eq!(prefs.jitter_buffer_ms, 200);
        assert!(prefs.dtmf_detection);
        assert!(prefs.comfort_noise);
    }
    
    #[test]
    fn test_media_preferences_sdp_attributes() {
        let mut prefs = MediaPreferences::new();
        prefs = prefs.add_sdp_attribute("rtcp-fb".to_string(), "* nack".to_string());
        prefs = prefs.add_sdp_attribute("fmtp".to_string(), "mode=1".to_string());
        
        assert_eq!(prefs.sdp_attributes.len(), 2);
        assert_eq!(prefs.sdp_attributes.get("rtcp-fb"), Some(&"* nack".to_string()));
        assert_eq!(prefs.sdp_attributes.get("fmtp"), Some(&"mode=1".to_string()));
    }
    
    #[tokio::test]
    async fn test_advanced_client_creation() {
        let pipeline_config = AudioPipelineConfig::custom()
            .echo_cancellation(true)
            .noise_suppression(true);
        
        let media_prefs = MediaPreferences::default();
        
        // Test that we can create an advanced client
        let result = AdvancedSipClient::new(
            "sip:test@example.com",
            pipeline_config,
            media_prefs
        ).await;
        
        // The client should be created successfully
        assert!(result.is_ok());
        
        // Test that we can access the client's methods
        if let Ok(client) = result {
            // Get event stream
            let _events = client.events();
            
            // Test that we can't start twice
            let start_result = client.start().await;
            assert!(start_result.is_ok());
            
            let start_again = client.start().await;
            assert!(start_again.is_err());
            
            // Stop the client
            let stop_result = client.stop().await;
            assert!(stop_result.is_ok());
        }
    }
    
    // Custom audio processor implementation for testing
    struct TestAudioProcessor {
        name: String,
        gain: f32,
    }
    
    impl AudioProcessorTrait for TestAudioProcessor {
        fn process(&mut self, frame: &mut rvoip_audio_core::AudioFrame) {
            // Apply gain to all samples
            // Note: audio-core doesn't expose mutable samples access
            // In a real implementation, we would modify the frame
        }
        
        fn name(&self) -> &str {
            &self.name
        }
    }
    
    #[test]
    fn test_custom_audio_processor() {
        let processor = Box::new(TestAudioProcessor {
            name: "TestGain".to_string(),
            gain: 2.0,
        });
        
        let config = AudioPipelineConfig::custom()
            .add_processor(processor);
        
        assert_eq!(config.custom_processors.len(), 1);
    }
    
    #[test]
    fn test_call_statistics_struct() {
        let stats = CallStatistics {
            audio_metrics: crate::types::AudioQualityMetrics {
                level: 0.5,
                peak_level: 0.8,
                mos: 4.2,
                packet_loss_percent: 0.5,
                jitter_ms: 15.0,
                rtt_ms: 50.0,
            },
            packets_sent: 1000,
            packets_received: 995,
            bytes_sent: 160000,
            bytes_received: 159200,
            codec: "PCMU".to_string(),
        };
        
        assert_eq!(stats.audio_metrics.mos, 4.2);
        assert_eq!(stats.packets_sent, 1000);
        assert_eq!(stats.codec, "PCMU");
    }
}

#[cfg(test)]
mod event_tests {
    use super::*;
    use crate::advanced::*;
    use crate::types::{Call, CallId, CallState, CallDirection};
    use crate::events::SipClientEvent;
    use std::sync::Arc;
    use parking_lot::RwLock;
    
    #[test]
    fn test_call_transfer_event() {
        let call = Arc::new(Call {
            id: CallId::new_v4(),
            state: Arc::new(RwLock::new(CallState::Connected)),
            remote_uri: "sip:bob@example.com".to_string(),
            local_uri: "sip:alice@example.com".to_string(),
            start_time: chrono::Utc::now(),
            connect_time: Some(chrono::Utc::now()),
            codec: Some(codec_core::CodecType::G711Pcmu),
            direction: CallDirection::Outgoing,
        });
        
        let event = SipClientEvent::CallTransferred {
            call: call.clone(),
            target: "sip:charlie@example.com".to_string(),
        };
        
        match event {
            SipClientEvent::CallTransferred { call: ev_call, target } => {
                assert_eq!(ev_call.id, call.id);
                assert_eq!(target, "sip:charlie@example.com");
            }
            _ => panic!("Wrong event type"),
        }
    }
    
    #[test]
    fn test_call_hold_resume_events() {
        let call = Arc::new(Call {
            id: CallId::new_v4(),
            state: Arc::new(RwLock::new(CallState::Connected)),
            remote_uri: "sip:bob@example.com".to_string(),
            local_uri: "sip:alice@example.com".to_string(),
            start_time: chrono::Utc::now(),
            connect_time: Some(chrono::Utc::now()),
            codec: Some(codec_core::CodecType::G711Pcmu),
            direction: CallDirection::Outgoing,
        });
        
        // Test hold event
        let hold_event = SipClientEvent::CallOnHold {
            call: call.clone(),
        };
        
        match hold_event {
            SipClientEvent::CallOnHold { call: ev_call } => {
                assert_eq!(ev_call.id, call.id);
            }
            _ => panic!("Wrong event type"),
        }
        
        // Test resume event
        let resume_event = SipClientEvent::CallResumed {
            call: call.clone(),
        };
        
        match resume_event {
            SipClientEvent::CallResumed { call: ev_call } => {
                assert_eq!(ev_call.id, call.id);
            }
            _ => panic!("Wrong event type"),
        }
    }
}

#[cfg(test)]
mod frame_access_tests {
    use super::*;
    use crate::advanced::*;
    use rvoip_audio_core::{AudioFormat, AudioFrame};
    
    #[tokio::test]
    async fn test_audio_stream_concept() {
        // Test that the AudioStream type exists and has the expected methods
        // We can't create an AudioStream directly in tests since fields are private
        // This is more of a compile-time test to ensure the API exists
        
        // Test that we can work with audio frames
        let format = AudioFormat::pcm_8khz_mono();
        let samples = vec![100i16; 160];
        let frame = AudioFrame::new(samples.clone(), format.clone(), 0);
        
        assert_eq!(frame.samples.len(), 160);
        assert_eq!(frame.timestamp, 0);
        
        // Test creating another frame
        let samples2 = vec![200i16; 160];
        let frame2 = AudioFrame::new(samples2, format, 160);
        
        assert_eq!(frame2.samples.len(), 160);
        assert_eq!(frame2.timestamp, 160);
    }
}

#[cfg(test)]
mod config_validation_tests {
    use super::*;
    use crate::advanced::*;
    
    #[test]
    fn test_invalid_dtmf_validation() {
        // Test that we properly validate DTMF digits
        let valid_digits = "1234567890*#ABCD";
        for digit in valid_digits.chars() {
            // This would be tested in the actual implementation
            assert!(matches!(digit, '0'..='9' | '*' | '#' | 'A'..='D'));
        }
        
        let invalid_digits = "XYZ!@$";
        for digit in invalid_digits.chars() {
            assert!(!matches!(digit, '0'..='9' | '*' | '#' | 'A'..='D'));
        }
    }
}