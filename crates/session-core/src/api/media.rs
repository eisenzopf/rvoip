//! Media Control API
//!
//! Comprehensive API for managing media streams in SIP sessions, including audio
//! transmission, SDP negotiation, and quality monitoring.
//! 
//! # Overview
//! 
//! The `MediaControl` trait provides a high-level interface for:
//! - **SDP Management**: Offer/answer generation and negotiation
//! - **Media Flow**: Establishing and controlling RTP streams
//! - **Quality Monitoring**: Real-time statistics and MOS scores
//! - **Audio Control**: Mute/unmute and transmission management
//! 
//! # Architecture
//! 
//! ```text
//! Application Layer
//!        |
//!   MediaControl API
//!        |
//! ┌──────┴──────┐
//! │   Session   │
//! │ Coordinator │
//! └──────┬──────┘
//!        |
//! ┌──────┴──────┐     ┌─────────┐
//! │    Media    │────▶│   RTP   │
//! │   Manager   │     │  Core   │
//! └─────────────┘     └─────────┘
//! ```
//! 
//! # SDP Negotiation Patterns
//! 
//! ## Pattern 1: UAC (Outgoing Call) Flow
//! 
//! ```rust
//! use rvoip_session_core::{SessionCoordinator, SessionControl, MediaControl, parse_sdp_connection};
//! use std::sync::Arc;
//! use std::time::Duration;
//! 
//! async fn make_outgoing_call(
//!     coordinator: Arc<SessionCoordinator>
//! ) -> Result<(), Box<dyn std::error::Error>> {
//!     // 1. Prepare the call (allocates media resources)
//!     let prepared = SessionControl::prepare_outgoing_call(
//!         &coordinator,
//!         "sip:alice@ourserver.com",
//!         "sip:bob@example.com"
//!     ).await?;
//!     
//!     println!("Allocated RTP port: {}", prepared.local_rtp_port);
//!     println!("SDP Offer:\n{}", prepared.sdp_offer);
//!     
//!     // 2. Initiate the call (sends INVITE)
//!     let session = SessionControl::initiate_prepared_call(
//!         &coordinator,
//!         &prepared
//!     ).await?;
//!     
//!     // 3. Wait for answer (200 OK with SDP)
//!     SessionControl::wait_for_answer(
//!         &coordinator,
//!         &session.id,
//!         Duration::from_secs(30)
//!     ).await?;
//!     
//!     // 4. Media flow is automatically established when answer is received
//!     // But you can also manually control it:
//!     let media_info = MediaControl::get_media_info(
//!         &coordinator,
//!         &session.id
//!     ).await?;
//!     
//!     if let Some(info) = media_info {
//!         println!("Codec: {}", info.codec.unwrap_or("unknown".to_string()));
//!         println!("Remote RTP: {}:{}", 
//!             info.remote_sdp.as_ref().map(|_| "connected").unwrap_or("pending"),
//!             info.remote_rtp_port.unwrap_or(0)
//!         );
//!     }
//!     
//!     Ok(())
//! }
//! ```
//! 
//! ## Pattern 2: UAS (Incoming Call) Flow
//! 
//! ```rust
//! use rvoip_session_core::{SessionCoordinator, SessionControl, MediaControl, CallHandler, CallDecision, 
//!                           IncomingCall, CallSession, parse_sdp_connection};
//! use std::sync::Arc;
//! 
//! #[derive(Debug)]
//! struct MyCallHandler;
//! 
//! #[async_trait::async_trait]
//! impl CallHandler for MyCallHandler {
//!     async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
//!         // For immediate decision with SDP answer generation
//!         CallDecision::Defer  // We'll handle it programmatically
//!     }
//!     
//!     async fn on_call_ended(&self, call: CallSession, reason: &str) {
//!         println!("Call {} ended: {}", call.id(), reason);
//!     }
//! }
//! 
//! // Handle the deferred call programmatically
//! async fn handle_incoming_call(
//!     coordinator: &Arc<SessionCoordinator>,
//!     call: IncomingCall
//! ) -> Result<(), Box<dyn std::error::Error>> {
//!     // 1. Analyze the offer
//!     let offer = call.sdp.as_ref().unwrap();
//!     let offer_info = parse_sdp_connection(offer)?;
//!     println!("Caller wants to use: {:?}", offer_info.codecs);
//!     
//!     // 2. Generate answer based on our capabilities
//!     let answer = MediaControl::generate_sdp_answer(
//!         coordinator,
//!         &call.id,
//!         offer
//!     ).await?;
//!     
//!     // 3. Accept the call with our answer
//!     let session = SessionControl::accept_incoming_call(
//!         coordinator,
//!         &call,
//!         Some(answer)
//!     ).await?;
//!     
//!     // 4. Establish media flow to the caller
//!     MediaControl::establish_media_flow(
//!         coordinator,
//!         &session.id,
//!         &format!("{}:{}", offer_info.ip, offer_info.port)
//!     ).await?;
//!     
//!     Ok(())
//! }
//! ```
//! 
//! # Quality Monitoring
//! 
//! ## Real-time Quality Metrics
//! 
//! ```rust
//! use std::sync::Arc;
//! use std::time::Duration;
//! use rvoip_session_core::{SessionCoordinator, SessionId, MediaControl, SessionControl};
//! 
//! async fn monitor_call_quality(
//!     coordinator: Arc<SessionCoordinator>,
//!     session_id: SessionId
//! ) -> Result<(), Box<dyn std::error::Error>> {
//!     // Start automatic monitoring every 5 seconds
//!     MediaControl::start_statistics_monitoring(
//!         &coordinator,
//!         &session_id,
//!         Duration::from_secs(5)
//!     ).await?;
//!     
//!     // Also do manual checks
//!     let mut quality_warnings = 0;
//!     
//!     loop {
//!         tokio::time::sleep(Duration::from_secs(10)).await;
//!         
//!         // Get comprehensive statistics
//!         let stats = MediaControl::get_media_statistics(
//!             &coordinator,
//!             &session_id
//!         ).await?;
//!         
//!         if let Some(stats) = stats {
//!             // Get call statistics which includes quality metrics
//!             if let Ok(Some(call_stats)) = MediaControl::get_call_statistics(
//!                 &coordinator,
//!                 &session_id
//!             ).await {
//!                 let quality = &call_stats.quality;
//!                 let mos = quality.mos_score;
//!                 
//!                 println!("Call Quality Report:");
//!                 println!("  MOS Score: {:.1} ({})", mos, match mos {
//!                     x if x >= 4.0 => "Excellent",
//!                     x if x >= 3.5 => "Good",
//!                     x if x >= 3.0 => "Fair",
//!                     x if x >= 2.5 => "Poor",
//!                     _ => "Bad"
//!                 });
//!                 println!("  Packet Loss: {:.1}%", quality.packet_loss_rate);
//!                 println!("  Jitter: {:.1}ms", quality.jitter_ms);
//!                 println!("  Round Trip: {:.0}ms", quality.round_trip_ms);
//!                 
//!                 // Alert on poor quality
//!                 if mos < 3.0 {
//!                     quality_warnings += 1;
//!                     if quality_warnings >= 3 {
//!                         // Sustained poor quality
//!                         println!("Warning: Sustained poor quality - MOS {:.1}", mos);
//!                     }
//!                 } else {
//!                     quality_warnings = 0;
//!                 }
//!                 
//!                 // Check RTP statistics
//!                 let rtp = &call_stats.rtp;
//!                 println!("RTP Statistics:");
//!                 println!("  Packets Sent: {}", rtp.packets_sent);
//!                 println!("  Packets Received: {}", rtp.packets_received);
//!                 println!("  Packets Lost: {}", rtp.packets_lost);
//!             }
//!         }
//!         
//!         // Check if call is still active
//!         if let Ok(Some(session)) = SessionControl::get_session(&coordinator, &session_id).await {
//!             if session.state().is_final() {
//!                 break;
//!             }
//!         } else {
//!             break;
//!         }
//!     }
//!     
//!     Ok(())
//! }
//! ```
//! 
//! # Audio Control - Two API Paradigms
//! 
//! The MediaControl trait provides **two different approaches** to audio handling,
//! each designed for different use cases and levels of control.
//! 
//! ## 1. Legacy Audio Transmission API (Simple & High-Level)
//! 
//! **Use when you need**:
//! - Simple mute/unmute functionality  
//! - Playing pre-recorded audio files or tones
//! - Basic audio transmission without frame-level control
//! - Fire-and-forget audio playback
//! - Testing and debugging with generated audio
//! 
//! **Key Methods**:
//! - `start_audio_transmission()` - Simple pass-through mode
//! - `start_audio_transmission_with_tone()` - Generate test tones
//! - `start_audio_transmission_with_custom_audio()` - Play audio files
//! - `stop_audio_transmission()` - Stop all audio
//! - `set_tone_generation()`, `set_custom_audio()` - Change audio source
//! 
//! ### Example: Simple Mute/Unmute
//! 
//! ```rust
//! use std::sync::Arc;
//! use rvoip_session_core::{SessionCoordinator, SessionId, MediaControl, SessionControl};
//! 
//! async fn handle_user_controls(
//!     coordinator: Arc<SessionCoordinator>,
//!     session_id: SessionId
//! ) -> Result<(), Box<dyn std::error::Error>> {
//!     // Mute (stop sending audio)
//!     MediaControl::stop_audio_transmission(&coordinator, &session_id).await?;
//!     println!("Microphone muted");
//!     
//!     // Check mute status
//!     let is_muted = !MediaControl::is_audio_transmission_active(
//!         &coordinator, 
//!         &session_id
//!     ).await?;
//!     
//!     // Unmute (resume sending audio)
//!     MediaControl::start_audio_transmission(&coordinator, &session_id).await?;
//!     println!("Microphone unmuted");
//!     
//!     Ok(())
//! }
//! ```
//! 
//! ### Example: Test Tone Generation
//! 
//! ```rust
//! use std::sync::Arc;
//! use rvoip_session_core::{SessionCoordinator, SessionId, MediaControl};
//! 
//! async fn play_test_tone(
//!     coordinator: Arc<SessionCoordinator>,
//!     session_id: SessionId
//! ) -> Result<(), Box<dyn std::error::Error>> {
//!     // Start playing a 440Hz test tone
//!     MediaControl::start_audio_transmission_with_tone(&coordinator, &session_id).await?;
//!     
//!     // Change to a different frequency
//!     MediaControl::set_tone_generation(&coordinator, &session_id, 880.0, 0.5).await?;
//!     
//!     // Stop the tone
//!     MediaControl::stop_audio_transmission(&coordinator, &session_id).await?;
//!     
//!     Ok(())
//! }
//! ```
//! 
//! ## 2. Audio Streaming API (Advanced & Frame-Level)
//! 
//! **Use when you need**:
//! - **Real-time audio device integration** (microphones, speakers)
//! - Frame-by-frame audio processing and effects
//! - Custom audio pipelines and routing
//! - Integration with external audio libraries
//! - Low-latency audio applications
//! - Real-time audio analysis or modification
//! 
//! **Key Methods**:
//! - `subscribe_to_audio_frames()` - Get decoded frames for playback
//! - `send_audio_frame()` - Send frames for encoding/transmission
//! - `get_audio_stream_config()`, `set_audio_stream_config()` - Stream parameters
//! - `start_audio_stream()`, `stop_audio_stream()` - Frame pipeline control
//! 
//! ### Example: Audio Device Integration
//! 
//! ```rust
//! use std::sync::Arc;
//! use rvoip_session_core::{SessionCoordinator, SessionId, MediaControl};
//! use rvoip_session_core::types::{AudioFrame, AudioStreamConfig};
//! 
//! async fn setup_audio_devices(
//!     coordinator: Arc<SessionCoordinator>,
//!     session_id: SessionId
//! ) -> Result<(), Box<dyn std::error::Error>> {
//!     // Configure high-quality audio stream
//!     let config = AudioStreamConfig {
//!         sample_rate: 48000,
//!         channels: 2,
//!         codec: "Opus".to_string(),
//!         frame_size_ms: 10,
//!         enable_aec: true,
//!         enable_agc: true,
//!         enable_vad: true,
//!     };
//!     MediaControl::set_audio_stream_config(&coordinator, &session_id, config).await?;
//!     
//!     // Start the audio streaming pipeline
//!     MediaControl::start_audio_stream(&coordinator, &session_id).await?;
//!     
//!     // Subscribe to receive decoded audio frames for speaker playback
//!     let mut audio_subscriber = MediaControl::subscribe_to_audio_frames(&coordinator, &session_id).await?;
//!     
//!     // Spawn a task to handle audio playback
//!     tokio::spawn(async move {
//!         while let Some(audio_frame) = audio_subscriber.recv().await {
//!             // Send frame to speaker/audio device
//!             play_audio_frame_to_speaker(audio_frame).await;
//!         }
//!     });
//!     
//!     // Spawn a task to handle audio capture and transmission
//!     let coordinator_clone = coordinator.clone();
//!     tokio::spawn(async move {
//!         loop {
//!             // Capture audio frame from microphone
//!             if let Some(frame) = capture_audio_frame_from_microphone().await {
//!                 // Send frame for encoding and transmission
//!                 let _ = MediaControl::send_audio_frame(&coordinator_clone, &session_id, frame).await;
//!             }
//!         }
//!     });
//!     
//!     Ok(())
//! }
//! 
//! // Placeholder functions for audio device integration
//! async fn play_audio_frame_to_speaker(frame: AudioFrame) {
//!     // Implementation would use audio device APIs (e.g., cpal, portaudio)
//!     println!("Playing {} samples at {}Hz", frame.samples.len(), frame.sample_rate);
//! }
//! 
//! async fn capture_audio_frame_from_microphone() -> Option<AudioFrame> {
//!     // Implementation would capture from microphone
//!     Some(AudioFrame::new(vec![0; 480], 48000, 1, 12345)) // Placeholder
//! }
//! ```
//! 
//! ## When To Use Which API?
//! 
//! | Use Case | Recommended API | Why |
//! |----------|----------------|-----|
//! | Simple mute/unmute | **Legacy** | Simpler, fewer method calls |
//! | Playing audio files | **Legacy** | Built-in file handling |
//! | Test tone generation | **Legacy** | Built-in tone generation |
//! | Microphone integration | **Streaming** | Frame-level control needed |
//! | Speaker playback | **Streaming** | Real-time frame delivery |
//! | Audio effects/processing | **Streaming** | Frame access required |
//! | Low-latency applications | **Streaming** | Better latency control |
//! | Audio analysis | **Streaming** | Access to raw frames |
//! | Simple voice calls | **Legacy** | Easier to implement |
//! | Advanced audio apps | **Streaming** | More flexibility |
//! 
//! # Advanced Use Cases
//! 
//! ## Dynamic Codec Switching
//! 
//! ```rust
//! use std::sync::Arc;
//! use rvoip_session_core::{SessionCoordinator, SessionId, MediaControl, SessionControl};
//! 
//! async fn switch_to_hd_audio(
//!     coordinator: &Arc<SessionCoordinator>,
//!     session_id: &SessionId
//! ) -> Result<(), Box<dyn std::error::Error>> {
//!     // Generate new SDP with HD codec preference
//!     let new_sdp = r#"v=0
//! o=- 0 0 IN IP4 127.0.0.1
//! s=-
//! c=IN IP4 127.0.0.1
//! t=0 0
//! m=audio 5004 RTP/AVP 9 0 8
//! a=rtpmap:9 G722/8000
//! a=rtpmap:0 PCMU/8000
//! a=rtpmap:8 PCMA/8000"#;
//!     
//!     // Update media session
//!     SessionControl::update_media(coordinator, session_id, new_sdp).await?;
//!     
//!     Ok(())
//! }
//! ```
//! 
//! ## Network Change Handling
//! 
//! ```rust
//! use std::sync::Arc;
//! use rvoip_session_core::{SessionCoordinator, SessionId, MediaControl, SessionControl};
//! 
//! async fn handle_network_change(
//!     coordinator: &Arc<SessionCoordinator>,
//!     session_id: &SessionId,
//!     new_ip: &str
//! ) -> Result<(), Box<dyn std::error::Error>> {
//!     // Stop current transmission
//!     MediaControl::stop_audio_transmission(coordinator, session_id).await?;
//!     
//!     // Update with new network info
//!     let media_info = MediaControl::get_media_info(coordinator, session_id).await?
//!         .ok_or("No media session".to_string())?;
//!     
//!     if let Some(remote_port) = media_info.remote_rtp_port {
//!         // Re-establish with new IP
//!         MediaControl::establish_media_flow(
//!             coordinator,
//!             session_id,
//!             &format!("{}:{}", new_ip, remote_port)
//!         ).await?;
//!     }
//!     
//!     Ok(())
//! }
//! ```
//! 
//! # Best Practices
//! 
//! 1. **Always check media state** before operations
//! 2. **Monitor quality** for calls longer than 1 minute  
//! 3. **Handle network errors** gracefully with retries
//! 4. **Use proper SDP negotiation** - don't assume codecs
//! 5. **Clean up resources** when calls end
//! 
//! # Error Handling
//! 
//! ```rust
//! use std::sync::Arc;
//! use rvoip_session_core::{SessionCoordinator, SessionId, MediaControl, SessionError};
//! 
//! async fn handle_media_errors(
//!     coordinator: Arc<SessionCoordinator>,
//!     session_id: SessionId,
//!     addr: &str
//! ) {
//!     match MediaControl::establish_media_flow(&coordinator, &session_id, addr).await {
//!         Ok(_) => println!("Media established"),
//!         Err(SessionError::MediaIntegration { message }) => {
//!             eprintln!("Media error: {}", message);
//!             // Try fallback or notify user
//!         }
//!         Err(e) => eprintln!("Unexpected error: {}", e),
//!     }
//! }
//! ```

use std::sync::Arc;
use crate::api::types::{SessionId, MediaInfo};
use crate::api::control::SessionControl;
use crate::coordinator::SessionCoordinator;
use crate::errors::Result;

/// Extension trait for media control operations
pub trait MediaControl {
    // =============================================================================
    // AUDIO STREAMING API
    // =============================================================================
    
    /// Subscribe to audio frames from a session
    /// Returns a subscriber that can be used to receive decoded audio frames for playback
    async fn subscribe_to_audio_frames(&self, session_id: &SessionId) -> Result<crate::api::types::AudioFrameSubscriber>;
    
    /// Send an audio frame for encoding and transmission
    /// The frame will be encoded and sent via RTP to the remote peer
    async fn send_audio_frame(&self, session_id: &SessionId, audio_frame: crate::api::types::AudioFrame) -> Result<()>;
    
    /// Get current audio stream configuration for a session
    /// Returns None if no audio stream is configured
    async fn get_audio_stream_config(&self, session_id: &SessionId) -> Result<Option<crate::api::types::AudioStreamConfig>>;
    
    /// Set audio stream configuration for a session
    /// This updates the stream parameters like sample rate, codec, etc.
    async fn set_audio_stream_config(&self, session_id: &SessionId, config: crate::api::types::AudioStreamConfig) -> Result<()>;
    
    /// Start audio streaming for a session
    /// This begins the audio frame pipeline and enables audio flow
    async fn start_audio_stream(&self, session_id: &SessionId) -> Result<()>;
    
    /// Stop audio streaming for a session
    /// This stops the audio frame pipeline and disables audio flow
    async fn stop_audio_stream(&self, session_id: &SessionId) -> Result<()>;
    
    // =============================================================================
    // LEGACY AUDIO CONTROL API (for backward compatibility)
    // =============================================================================
    
    /// Start audio transmission for a session
    /// This will begin audio transmission in pass-through mode (default)
    async fn start_audio_transmission(&self, session_id: &SessionId) -> Result<()>;
    
    /// Start audio transmission for a session with tone generation
    /// This will begin sending generated audio (440Hz sine wave in G.711 µ-law format)
    async fn start_audio_transmission_with_tone(&self, session_id: &SessionId) -> Result<()>;
    
    /// Start audio transmission for a session with custom audio samples
    /// This will begin transmitting the provided audio samples (must be in G.711 µ-law format)
    async fn start_audio_transmission_with_custom_audio(&self, session_id: &SessionId, samples: Vec<u8>, repeat: bool) -> Result<()>;
    
    /// Stop audio transmission for a session
    async fn stop_audio_transmission(&self, session_id: &SessionId) -> Result<()>;
    
    /// Set custom audio samples for an active transmission session
    /// This allows changing the audio source during transmission
    async fn set_custom_audio(&self, session_id: &SessionId, samples: Vec<u8>, repeat: bool) -> Result<()>;
    
    /// Set tone generation parameters for an active transmission session
    /// This allows changing to tone generation during transmission
    async fn set_tone_generation(&self, session_id: &SessionId, frequency: f64, amplitude: f64) -> Result<()>;
    
    /// Enable pass-through mode for an active transmission session
    /// This stops audio generation and allows RTP pass-through
    async fn set_pass_through_mode(&self, session_id: &SessionId) -> Result<()>;
    
    /// Establish media flow by setting remote RTP address and starting audio
    /// The remote address should be in the format "ip:port" (e.g., "127.0.0.1:30000")
    async fn establish_media_flow(&self, session_id: &SessionId, remote_addr: &str) -> Result<()>;
    
    /// Update media session with SDP answer/offer
    /// This will parse the SDP to extract remote RTP address and codec information
    async fn update_media_with_sdp(&self, session_id: &SessionId, sdp: &str) -> Result<()>;
    
    /// Check if audio transmission is active for a session
    async fn is_audio_transmission_active(&self, session_id: &SessionId) -> Result<bool>;
    
    /// Get detailed media information for a session
    async fn get_media_info(&self, session_id: &SessionId) -> Result<Option<MediaInfo>>;
    
    /// Generate SDP offer for a session using dynamically allocated ports
    /// This creates a media session if needed and returns SDP with the allocated port
    async fn generate_sdp_offer(&self, session_id: &SessionId) -> Result<String>;
    
    /// Get RTP/RTCP statistics for a session
    async fn get_rtp_statistics(&self, session_id: &SessionId) -> Result<Option<crate::media::stats::RtpSessionStats>>;
    
    /// Get comprehensive media statistics including quality metrics
    async fn get_media_statistics(&self, session_id: &SessionId) -> Result<Option<crate::media::stats::MediaStatistics>>;
    
    /// Start periodic statistics monitoring with the specified interval
    async fn start_statistics_monitoring(&self, session_id: &SessionId, interval: std::time::Duration) -> Result<()>;
    
    /// Create a media session without generating SDP
    /// This is useful when you need to prepare media before SDP negotiation,
    /// particularly in UAS scenarios where you receive an offer first
    async fn create_media_session(&self, session_id: &SessionId) -> Result<()>;
    
    /// Update media session with remote SDP without starting transmission
    /// This separates SDP handling from media flow establishment, allowing
    /// more control over when audio transmission begins
    async fn update_remote_sdp(&self, session_id: &SessionId, remote_sdp: &str) -> Result<()>;
    
    /// Generate SDP answer based on received offer
    /// This provides proper offer/answer negotiation for UAS scenarios
    /// without requiring direct access to internal components
    async fn generate_sdp_answer(&self, session_id: &SessionId, offer: &str) -> Result<String>;
    
    /// Get all statistics in one call
    /// This aggregates statistics from multiple sources into a single comprehensive result
    async fn get_call_statistics(&self, session_id: &SessionId) -> Result<Option<crate::media::stats::CallStatistics>>;
    
    /// Convenience: Get just the MOS score
    /// Returns None if no quality metrics are available
    async fn get_call_quality_score(&self, session_id: &SessionId) -> Result<Option<f32>>;
    
    /// Convenience: Get packet loss percentage
    /// Returns None if no RTP statistics are available
    async fn get_packet_loss_rate(&self, session_id: &SessionId) -> Result<Option<f32>>;
    
    /// Convenience: Get current bitrate in kbps
    /// Returns None if no RTP statistics are available
    async fn get_current_bitrate(&self, session_id: &SessionId) -> Result<Option<u32>>;
    
    /// Start quality monitoring with automatic alerts
    /// This will emit MediaQuality events when thresholds are exceeded
    async fn monitor_call_quality(&self, session_id: &SessionId, thresholds: crate::media::stats::QualityThresholds) -> Result<()>;
}

impl MediaControl for Arc<SessionCoordinator> {
    // =============================================================================
    // AUDIO STREAMING API IMPLEMENTATION
    // =============================================================================
    
    async fn subscribe_to_audio_frames(&self, session_id: &SessionId) -> Result<crate::api::types::AudioFrameSubscriber> {
        // Validate session exists
        if SessionControl::get_session(self, session_id).await?.is_none() {
            return Err(crate::errors::SessionError::SessionNotFound(session_id.to_string()));
        }
        
        // Create tokio channel for audio frames - no conversion needed anymore!
        let (subscriber_sender, subscriber_receiver) = tokio::sync::mpsc::channel::<crate::api::types::AudioFrame>(1000);
        
        // Set up the callback through MediaManager directly
        let media_manager = &self.media_manager;
        media_manager.set_audio_frame_callback(session_id, subscriber_sender).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to set audio frame callback: {}", e) 
            })?;
        
        // Create subscriber
        let subscriber = crate::api::types::AudioFrameSubscriber::new(
            session_id.clone(),
            subscriber_receiver,
        );
        
        // Publish AudioStreamStarted event
        if let Some(event_processor) = self.event_processor() {
            let config = crate::api::types::AudioStreamConfig::default();
            let _ = event_processor.publish_audio_stream_started(
                session_id.clone(),
                config,
                format!("stream-{}", session_id),
                crate::manager::events::MediaFlowDirection::Both,
            ).await;
        }
        
        tracing::info!("🎧 Created audio frame subscriber for session {}", session_id);
        Ok(subscriber)
    }
    
    async fn send_audio_frame(&self, session_id: &SessionId, audio_frame: crate::api::types::AudioFrame) -> Result<()> {
        // Check session state and handle terminating sessions gracefully
        match SessionControl::get_session(self, session_id).await? {
            Some(session) => {
                match session.state() {
                    crate::api::types::CallState::Terminating => {
                        // Phase 1: Session is terminating, stop sending audio gracefully
                        tracing::debug!("Session {} is terminating, stopping audio transmission gracefully", session_id);
                        return Ok(()); // Don't error, just silently drop the frame
                    }
                    crate::api::types::CallState::Terminated => {
                        // Phase 2: Session fully terminated
                        tracing::debug!("Session {} terminated, ignoring audio frame", session_id);
                        return Ok(()); // Don't error, just silently drop the frame
                    }
                    _ => {
                        // Normal operation - continue processing
                    }
                }
            }
            None => {
                // Session not found - likely terminated, handle gracefully
                tracing::debug!("Session {} not found (likely terminated), ignoring audio frame", session_id);
                return Ok(()); // Don't error, just silently drop the frame
            }
        }
        
        // Get the media manager
        let media_manager = &self.media_manager;
        
        // Log before moving the frame
        tracing::debug!("🎤 Sent audio frame for session {} ({} samples, {}Hz)", 
                       session_id, audio_frame.samples.len(), audio_frame.sample_rate);
        
        // Send the frame to the media manager for encoding and transmission
        media_manager.send_audio_frame_for_transmission(session_id, audio_frame.clone()).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to send audio frame for transmission: {}", e) 
            })?;
        
        // Publish AudioFrameRequested event (indicating we're handling the frame)
        if let Some(event_processor) = self.event_processor() {
            // Create a config based on the frame properties
            let config = crate::api::types::AudioStreamConfig {
                sample_rate: audio_frame.sample_rate,
                channels: audio_frame.channels,
                codec: "PCMU".to_string(),
                frame_size_ms: 20,
                enable_aec: true,
                enable_agc: true,
                enable_vad: true,
            };
            let _ = event_processor.publish_audio_frame_requested(
                session_id.clone(),
                config,
                Some(format!("stream-{}", session_id)),
            ).await;
        }
        Ok(())
    }
    
    async fn get_audio_stream_config(&self, session_id: &SessionId) -> Result<Option<crate::api::types::AudioStreamConfig>> {
        // Validate session exists
        if SessionControl::get_session(self, session_id).await?.is_none() {
            return Err(crate::errors::SessionError::SessionNotFound(session_id.to_string()));
        }
        
        // Get actual configuration from media manager
        let media_manager = &self.media_manager;
        let config = media_manager.get_audio_stream_config_internal(session_id).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to get audio stream config: {}", e) 
            })?
            .unwrap_or_else(|| crate::api::types::AudioStreamConfig::default());
        
        tracing::debug!("📊 Retrieved audio stream config for session {}: {:?}", session_id, config);
        Ok(Some(config))
    }
    
    async fn set_audio_stream_config(&self, session_id: &SessionId, config: crate::api::types::AudioStreamConfig) -> Result<()> {
        // Validate session exists
        if SessionControl::get_session(self, session_id).await?.is_none() {
            return Err(crate::errors::SessionError::SessionNotFound(session_id.to_string()));
        }
        
        // Apply configuration to media manager
        let media_manager = &self.media_manager;
        media_manager.set_audio_stream_config_internal(session_id, config.clone()).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to set audio stream config: {}", e) 
            })?;
        
        // Publish AudioStreamConfigChanged event
        if let Some(event_processor) = self.event_processor() {
            let old_config = crate::api::types::AudioStreamConfig::default();
            let _ = event_processor.publish_audio_stream_config_changed(
                session_id.clone(),
                old_config,
                config.clone(),
                Some(format!("stream-{}", session_id)),
            ).await;
        }
        
        tracing::info!("📊 Set audio stream config for session {}: {}Hz, {} channels, codec: {}", 
                      session_id, config.sample_rate, config.channels, config.codec);
        Ok(())
    }
    
    async fn start_audio_stream(&self, session_id: &SessionId) -> Result<()> {
        // Validate session exists
        if SessionControl::get_session(self, session_id).await?.is_none() {
            return Err(crate::errors::SessionError::SessionNotFound(session_id.to_string()));
        }
        
        // Start the audio streaming pipeline in the media manager
        let media_manager = &self.media_manager;
        media_manager.start_audio_streaming(session_id).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to start audio streaming: {}", e) 
            })?;
        
        // Publish AudioStreamStarted event
        if let Some(event_processor) = self.event_processor() {
            let config = crate::api::types::AudioStreamConfig::default();
            let _ = event_processor.publish_audio_stream_started(
                session_id.clone(),
                config,
                format!("stream-{}", session_id),
                crate::manager::events::MediaFlowDirection::Both,
            ).await;
        }
        
        tracing::info!("🎵 Started audio stream for session {}", session_id);
        Ok(())
    }
    
    async fn stop_audio_stream(&self, session_id: &SessionId) -> Result<()> {
        // Validate session exists
        if SessionControl::get_session(self, session_id).await?.is_none() {
            return Err(crate::errors::SessionError::SessionNotFound(session_id.to_string()));
        }
        
        // Stop the audio streaming pipeline in the media manager
        let media_manager = &self.media_manager;
        media_manager.stop_audio_streaming(session_id).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to stop audio streaming: {}", e) 
            })?;
        
        // Publish AudioStreamStopped event
        if let Some(event_processor) = self.event_processor() {
            let _ = event_processor.publish_audio_stream_stopped(
                session_id.clone(),
                format!("stream-{}", session_id),
                "User requested stop".to_string(),
            ).await;
        }
        
        tracing::info!("🛑 Stopped audio stream for session {}", session_id);
        Ok(())
    }
    
    // =============================================================================
    // LEGACY AUDIO CONTROL API IMPLEMENTATION
    // =============================================================================
    
    async fn start_audio_transmission(&self, session_id: &SessionId) -> Result<()> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Start audio transmission
        media_manager.start_audio_transmission(session_id).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to start audio transmission: {}", e) 
            })?;
        
        tracing::info!("Started audio transmission for session {}", session_id);
        Ok(())
    }
    
    async fn start_audio_transmission_with_tone(&self, session_id: &SessionId) -> Result<()> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Start audio transmission with tone generation
        media_manager.start_audio_transmission_with_tone(session_id).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to start audio transmission with tone: {}", e) 
            })?;
        
        tracing::info!("Started audio transmission with tone for session {}", session_id);
        Ok(())
    }
    
    async fn start_audio_transmission_with_custom_audio(&self, session_id: &SessionId, samples: Vec<u8>, repeat: bool) -> Result<()> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Start audio transmission with custom audio samples
        media_manager.start_audio_transmission_with_custom_audio(session_id, samples, repeat).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to start audio transmission with custom audio: {}", e) 
            })?;
        
        tracing::info!("Started audio transmission with custom audio for session {}", session_id);
        Ok(())
    }
    
    async fn stop_audio_transmission(&self, session_id: &SessionId) -> Result<()> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Stop audio transmission
        media_manager.stop_audio_transmission(session_id).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to stop audio transmission: {}", e) 
            })?;
        
        tracing::info!("Stopped audio transmission for session {}", session_id);
        Ok(())
    }
    
    async fn set_custom_audio(&self, session_id: &SessionId, samples: Vec<u8>, repeat: bool) -> Result<()> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Set custom audio samples for an active transmission session
        media_manager.set_custom_audio(session_id, samples, repeat).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to set custom audio: {}", e) 
            })?;
        
        tracing::info!("Set custom audio for session {}", session_id);
        Ok(())
    }
    
    async fn set_tone_generation(&self, session_id: &SessionId, frequency: f64, amplitude: f64) -> Result<()> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Set tone generation parameters for an active transmission session
        media_manager.set_tone_generation(session_id, frequency, amplitude).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to set tone generation: {}", e) 
            })?;
        
        tracing::info!("Set tone generation for session {}", session_id);
        Ok(())
    }
    
    async fn set_pass_through_mode(&self, session_id: &SessionId) -> Result<()> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Enable pass-through mode for an active transmission session
        media_manager.set_pass_through_mode(session_id).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to set pass-through mode: {}", e) 
            })?;
        
        tracing::info!("Set pass-through mode for session {}", session_id);
        Ok(())
    }
    
    async fn establish_media_flow(&self, session_id: &SessionId, remote_addr: &str) -> Result<()> {
        // Parse the remote address
        let socket_addr: std::net::SocketAddr = remote_addr.parse()
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Invalid remote address '{}': {}", remote_addr, e) 
            })?;
        
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Get the dialog ID for this session
        let dialog_id = {
            let mapping = media_manager.session_mapping.read().await;
            mapping.get(session_id).cloned()
                .ok_or_else(|| crate::errors::SessionError::MediaIntegration { 
                    message: format!("No media session found for {}", session_id) 
                })?
        };
        
        // Call the controller's establish_media_flow which handles everything
        media_manager.controller.establish_media_flow(&dialog_id, socket_addr).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to establish media flow: {}", e) 
            })?;
        
        // Don't overwrite the remote SDP if it already exists
        // Only create a minimal SDP if we don't have one already
        {
            let mut sdp_storage = media_manager.sdp_storage.write().await;
            let entry = sdp_storage.entry(session_id.clone()).or_insert((None, None));
            if entry.1.is_none() {
                // Only set minimal SDP if we don't have a remote SDP already
                let minimal_sdp = format!(
                    "v=0\r\nc=IN IP4 {}\r\nm=audio {} RTP/AVP 0\r\n",
                    socket_addr.ip(),
                    socket_addr.port()
                );
                entry.1 = Some(minimal_sdp);
            }
        }
        
        tracing::info!("Established media flow for session {} to {}", session_id, remote_addr);
        Ok(())
    }
    
    async fn update_media_with_sdp(&self, session_id: &SessionId, sdp: &str) -> Result<()> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Update media session with SDP
        media_manager.update_media_session(session_id, sdp).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to update media with SDP: {}", e) 
            })?;
        
        // Extract remote RTP address from SDP
        if let Ok(sdp_info) = crate::api::parse_sdp_connection(sdp) {
            let remote_addr = format!("{}:{}", sdp_info.ip, sdp_info.port);
            
            // Start audio transmission
            media_manager.start_audio_transmission(session_id).await
                .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                    message: format!("Failed to start audio transmission: {}", e) 
                })?;
            
            tracing::info!("Updated media and started audio for session {} with remote {}", 
                         session_id, remote_addr);
        }
        
        Ok(())
    }
    
    async fn is_audio_transmission_active(&self, session_id: &SessionId) -> Result<bool> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Check if audio streaming is active using the MediaManager
        media_manager.is_audio_streaming_active(session_id).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to check audio transmission status: {}", e) 
            })
    }
    
    async fn get_media_info(&self, session_id: &SessionId) -> Result<Option<MediaInfo>> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Get media info
        if let Some(media_session_info) = media_manager.get_media_info(session_id).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to get media info: {}", e) 
            })? {
            
            // Get RTP statistics
            let rtp_stats = self.get_rtp_statistics(session_id).await.ok().flatten();
            
            // Get quality metrics from media statistics
            let media_stats = media_manager.get_media_statistics(session_id).await.ok().flatten();
            let quality_metrics = media_stats.and_then(|stats| stats.quality_metrics)
                .map(|qm| crate::media::stats::QualityMetrics {
                    mos_score: qm.mos_score.unwrap_or(5.0),
                    packet_loss_rate: qm.packet_loss_percent,
                    jitter_ms: qm.jitter_ms as f32,
                    round_trip_ms: qm.rtt_ms.unwrap_or(0.0) as f32,
                    network_effectiveness: 1.0 - (qm.packet_loss_percent / 100.0),
                    is_acceptable: qm.mos_score.unwrap_or(5.0) >= 3.0,
                });
            
            // Convert to API MediaInfo type
            Ok(Some(MediaInfo {
                local_sdp: media_session_info.local_sdp,
                remote_sdp: media_session_info.remote_sdp,
                local_rtp_port: media_session_info.local_rtp_port,
                remote_rtp_port: media_session_info.remote_rtp_port,
                codec: media_session_info.codec,
                rtp_stats,
                quality_metrics,
            }))
        } else {
            Ok(None)
        }
    }
    
    async fn generate_sdp_offer(&self, session_id: &SessionId) -> Result<String> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Generate SDP offer using the media manager
        // This will create a media session if needed and use the allocated port
        media_manager.generate_sdp_offer(session_id).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to generate SDP offer: {}", e) 
            })
    }
    
    async fn get_rtp_statistics(&self, session_id: &SessionId) -> Result<Option<crate::media::stats::RtpSessionStats>> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Get RTP statistics from media manager
        if let Some(rtp_stats) = media_manager.get_rtp_statistics(session_id).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to get RTP statistics: {}", e) 
            })? {
            // Convert rtp-core stats to session-core stats
            Ok(Some(crate::media::stats::RtpSessionStats {
                packets_sent: rtp_stats.packets_sent,
                packets_received: rtp_stats.packets_received,
                bytes_sent: rtp_stats.bytes_sent,
                bytes_received: rtp_stats.bytes_received,
                packets_lost: rtp_stats.packets_lost,
                packets_out_of_order: 0, // Not available in rtp-core
                jitter_buffer_ms: 0.0,   // Not available in rtp-core
                current_bitrate_kbps: 0, // Would need to calculate
            }))
        } else {
            Ok(None)
        }
    }
    
    async fn get_media_statistics(&self, session_id: &SessionId) -> Result<Option<crate::media::stats::MediaStatistics>> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Get media info to build our statistics
        if let Some(media_info) = media_manager.get_media_info(session_id).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to get media info: {}", e) 
            })? {
            
            // Get stored SDP to extract actual addresses
            let (local_sdp, remote_sdp) = {
                let sdp_storage = media_manager.sdp_storage.read().await;
                sdp_storage.get(session_id).cloned().unwrap_or((None, None))
            };
            
            // Extract local address from local SDP or use bind address
            let local_addr = if let Some(ref sdp) = local_sdp {
                if let Ok(info) = crate::api::parse_sdp_connection(sdp) {
                    Some(format!("{}:{}", info.ip, info.port))
                } else {
                    media_info.local_rtp_port.map(|p| format!("{}:{}", media_manager.local_bind_addr.ip(), p))
                }
            } else {
                media_info.local_rtp_port.map(|p| format!("{}:{}", media_manager.local_bind_addr.ip(), p))
            };
            
            // Extract remote address from remote SDP
            let remote_addr = if let Some(ref sdp) = remote_sdp {
                if let Ok(info) = crate::api::parse_sdp_connection(sdp) {
                    Some(format!("{}:{}", info.ip, info.port))
                } else {
                    None
                }
            } else {
                None
            };
            
            // Build session-core MediaStatistics from available info
            Ok(Some(crate::media::stats::MediaStatistics {
                local_addr,
                remote_addr,
                codec: media_info.codec,
                media_flowing: true, // Assume media is flowing if session exists
            }))
        } else {
            Ok(None)
        }
    }
    
    async fn start_statistics_monitoring(&self, session_id: &SessionId, interval: std::time::Duration) -> Result<()> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Start statistics monitoring
        media_manager.start_statistics_monitoring(session_id, interval).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to start statistics monitoring: {}", e) 
            })
    }
    
    async fn create_media_session(&self, session_id: &SessionId) -> Result<()> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Create media session without generating SDP
        media_manager.create_media_session(session_id).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to create media session: {}", e) 
            })?;
        
        tracing::info!("Created media session for {}", session_id);
        Ok(())
    }
    
    async fn update_remote_sdp(&self, session_id: &SessionId, remote_sdp: &str) -> Result<()> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Update media session with remote SDP but don't start transmission
        media_manager.update_media_session(session_id, remote_sdp).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to update remote SDP: {}", e) 
            })?;
        
        // Store the remote SDP
        {
            let mut sdp_storage = media_manager.sdp_storage.write().await;
            let entry = sdp_storage.entry(session_id.clone()).or_insert((None, None));
            entry.1 = Some(remote_sdp.to_string());
        }
        
        tracing::info!("Updated remote SDP for session {}", session_id);
        Ok(())
    }
    
    async fn generate_sdp_answer(&self, session_id: &SessionId, offer: &str) -> Result<String> {
        // First ensure we have a media session
        let media_manager = &self.media_manager;
        
        // Create media session if it doesn't exist
        if media_manager.get_media_info(session_id).await.ok().flatten().is_none() {
            self.create_media_session(session_id).await?;
        }
        
        // Get the allocated RTP port for this session
        let media_info = media_manager.get_media_info(session_id).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to get media info: {}", e) 
            })?
            .ok_or_else(|| crate::errors::SessionError::MediaIntegration { 
                message: "Media session not found".to_string() 
            })?;
        
        let local_port = media_info.local_rtp_port
            .ok_or_else(|| crate::errors::SessionError::MediaIntegration { 
                message: "No local RTP port allocated".to_string() 
            })?;
        
        // Generate answer using MediaConfigConverter with our media config
        use crate::media::config::MediaConfigConverter;
        let mut converter = MediaConfigConverter::with_media_config(&media_manager.media_config);
        let local_ip = media_manager.local_bind_addr.ip().to_string();
        let answer = converter.generate_sdp_answer(offer, &local_ip, local_port)
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to generate SDP answer: {}", e) 
            })?;
        
        // Store both the offer and our answer
        {
            let mut sdp_storage = media_manager.sdp_storage.write().await;
            let entry = sdp_storage.entry(session_id.clone()).or_insert((None, None));
            entry.0 = Some(answer.clone());  // Our answer
            entry.1 = Some(offer.to_string()); // Their offer
        }
        
        // Update media session with the offer to configure remote endpoint
        media_manager.update_media_session(session_id, offer).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to update media session: {}", e) 
            })?;
        
        tracing::info!("Generated SDP answer for session {} with codecs: {:?}", 
                      session_id, media_manager.media_config.preferred_codecs);
        Ok(answer)
    }
    
    async fn get_call_statistics(&self, session_id: &SessionId) -> Result<Option<crate::media::stats::CallStatistics>> {
        // Get session info
        let session = match SessionControl::get_session(self, session_id).await? {
            Some(s) => s,
            None => return Ok(None),
        };
        
        // Get media info
        let media_info = MediaControl::get_media_info(self, session_id).await?;
        
        // Get media statistics
        let media_stats = self.get_media_statistics(session_id).await?;
        
        // Get RTP statistics
        let rtp_stats = self.get_rtp_statistics(session_id).await?;
        
        // Build comprehensive statistics
        let call_stats = crate::media::stats::CallStatistics {
            session_id: session_id.clone(),
            duration: session.started_at.map(|t| t.elapsed()),
            state: session.state,
            media: media_stats.unwrap_or_default(),
            rtp: rtp_stats.unwrap_or_default(),
            quality: if let Some(quality) = media_info.as_ref().and_then(|m| m.quality_metrics.as_ref()) {
                quality.clone()
            } else {
                crate::media::stats::QualityMetrics::default()
            },
        };
        
        Ok(Some(call_stats))
    }
    
    async fn get_call_quality_score(&self, session_id: &SessionId) -> Result<Option<f32>> {
        if let Some(stats) = self.get_call_statistics(session_id).await? {
            Ok(Some(stats.quality.mos_score))
        } else {
            Ok(None)
        }
    }
    
    async fn get_packet_loss_rate(&self, session_id: &SessionId) -> Result<Option<f32>> {
        if let Some(stats) = self.get_call_statistics(session_id).await? {
            Ok(Some(stats.quality.packet_loss_rate))
        } else {
            Ok(None)
        }
    }
    
    async fn get_current_bitrate(&self, session_id: &SessionId) -> Result<Option<u32>> {
        if let Some(stats) = self.get_call_statistics(session_id).await? {
            Ok(Some(stats.rtp.current_bitrate_kbps))
        } else {
            Ok(None)
        }
    }
    
    async fn monitor_call_quality(&self, session_id: &SessionId, thresholds: crate::media::stats::QualityThresholds) -> Result<()> {
        let session_id = session_id.clone();
        let coordinator = self.clone();
        
        // Spawn a monitoring task
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(thresholds.check_interval);
            
            loop {
                interval.tick().await;
                
                // Get current statistics
                match coordinator.get_call_statistics(&session_id).await {
                    Ok(Some(stats)) => {
                        let quality = &stats.quality;
                        
                        // Check thresholds and emit events if exceeded
                        if quality.mos_score < thresholds.min_mos 
                            || quality.packet_loss_rate > thresholds.max_packet_loss
                            || quality.jitter_ms > thresholds.max_jitter_ms {
                            
                            // Publish media quality event
                            if let Some(event_processor) = coordinator.event_processor() {
                                let _ = event_processor.publish_media_quality(
                                    session_id.clone(),
                                    quality.mos_score,
                                    quality.packet_loss_rate,
                                    quality.jitter_ms,
                                    quality.round_trip_ms,
                                ).await;
                            }
                        }
                    }
                    Ok(None) => break, // Session ended
                    Err(_) => break,   // Error occurred
                }
            }
        });
        
        Ok(())
    }
}

/// Helper function to parse RTP port from SDP
fn parse_rtp_port_from_sdp(sdp: &str) -> Option<u16> {
    for line in sdp.lines() {
        if line.starts_with("m=audio ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                return parts[1].parse().ok();
            }
        }
    }
    None
} 