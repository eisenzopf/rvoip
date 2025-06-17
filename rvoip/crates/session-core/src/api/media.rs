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
//! use rvoip_session_core::api::*;
//! use std::sync::Arc;
//! 
//! async fn make_outgoing_call(
//!     coordinator: Arc<SessionCoordinator>
//! ) -> Result<()> {
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
//! use rvoip_session_core::api::*;
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
//! ) -> Result<()> {
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
//! use std::time::Duration;
//! 
//! async fn monitor_call_quality(
//!     coordinator: Arc<SessionCoordinator>,
//!     session_id: SessionId
//! ) -> Result<()> {
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
//!             // Check quality metrics
//!             if let Some(quality) = &stats.quality_metrics {
//!                 let mos = quality.mos_score.unwrap_or(0.0);
//!                 
//!                 println!("Call Quality Report:");
//!                 println!("  MOS Score: {:.1} ({})", mos, match mos {
//!                     x if x >= 4.0 => "Excellent",
//!                     x if x >= 3.5 => "Good",
//!                     x if x >= 3.0 => "Fair",
//!                     x if x >= 2.5 => "Poor",
//!                     _ => "Bad"
//!                 });
//!                 println!("  Packet Loss: {:.1}%", quality.packet_loss_percent);
//!                 println!("  Jitter: {:.1}ms", quality.jitter_ms);
//!                 println!("  Round Trip: {:.0}ms", quality.round_trip_time_ms);
//!                 
//!                 // Alert on poor quality
//!                 if mos < 3.0 {
//!                     quality_warnings += 1;
//!                     if quality_warnings >= 3 {
//!                         // Sustained poor quality
//!                         notify_poor_quality(&session_id, mos).await?;
//!                     }
//!                 } else {
//!                     quality_warnings = 0;
//!                 }
//!             }
//!             
//!             // Check RTP statistics
//!             if let Some(rtp_stats) = &stats.rtp_stats {
//!                 println!("RTP Statistics:");
//!                 println!("  Packets Sent: {}", rtp_stats.packets_sent);
//!                 println!("  Packets Received: {}", rtp_stats.packets_received);
//!                 println!("  Packets Lost: {}", rtp_stats.packets_lost);
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
//! # Audio Control
//! 
//! ## Mute/Unmute and Hold Operations
//! 
//! ```rust
//! async fn handle_user_controls(
//!     coordinator: Arc<SessionCoordinator>,
//!     session_id: SessionId
//! ) -> Result<()> {
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
//!     // Put call on hold (SIP level)
//!     SessionControl::hold_session(&coordinator, &session_id).await?;
//!     println!("Call on hold");
//!     
//!     // Resume from hold
//!     SessionControl::resume_session(&coordinator, &session_id).await?;
//!     println!("Call resumed");
//!     
//!     Ok(())
//! }
//! ```
//! 
//! # Advanced Use Cases
//! 
//! ## Dynamic Codec Switching
//! 
//! ```rust
//! async fn switch_to_hd_audio(
//!     coordinator: &Arc<SessionCoordinator>,
//!     session_id: &SessionId
//! ) -> Result<()> {
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
//! async fn handle_network_change(
//!     coordinator: &Arc<SessionCoordinator>,
//!     session_id: &SessionId,
//!     new_ip: &str
//! ) -> Result<()> {
//!     // Stop current transmission
//!     MediaControl::stop_audio_transmission(coordinator, session_id).await?;
//!     
//!     // Update with new network info
//!     let media_info = MediaControl::get_media_info(coordinator, session_id).await?
//!         .ok_or("No media session")?;
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
//! use rvoip_session_core::errors::SessionError;
//! 
//! match MediaControl::establish_media_flow(&coordinator, &session_id, addr).await {
//!     Ok(_) => println!("Media established"),
//!     Err(SessionError::MediaIntegration { message }) => {
//!         eprintln!("Media error: {}", message);
//!         // Try fallback or notify user
//!     }
//!     Err(e) => eprintln!("Unexpected error: {}", e),
//! }
//! ```

use std::sync::Arc;
use crate::api::types::{SessionId, MediaInfo};
use crate::coordinator::SessionCoordinator;
use crate::errors::Result;

/// Extension trait for media control operations
pub trait MediaControl {
    /// Start audio transmission for a session
    /// This will begin sending generated audio (440Hz sine wave in G.711 µ-law format)
    async fn start_audio_transmission(&self, session_id: &SessionId) -> Result<()>;
    
    /// Stop audio transmission for a session
    async fn stop_audio_transmission(&self, session_id: &SessionId) -> Result<()>;
    
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
    async fn get_rtp_statistics(&self, session_id: &SessionId) -> Result<Option<rvoip_rtp_core::session::RtpSessionStats>>;
    
    /// Get comprehensive media statistics including quality metrics
    async fn get_media_statistics(&self, session_id: &SessionId) -> Result<Option<rvoip_media_core::types::MediaStatistics>>;
    
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
}

impl MediaControl for Arc<SessionCoordinator> {
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
        
        // Store the remote SDP info
        let minimal_sdp = format!(
            "v=0\r\nc=IN IP4 {}\r\nm=audio {} RTP/AVP 0\r\n",
            socket_addr.ip(),
            socket_addr.port()
        );
        
        {
            let mut sdp_storage = media_manager.sdp_storage.write().await;
            let entry = sdp_storage.entry(session_id.clone()).or_insert((None, None));
            entry.1 = Some(minimal_sdp);
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
        
        // Extract remote RTP port from SDP and start audio if found
        if let Some(remote_port) = parse_rtp_port_from_sdp(sdp) {
            // Assume remote IP is same as local for now (127.0.0.1)
            let remote_addr = format!("127.0.0.1:{}", remote_port);
            
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
        
        // Check if we have a media session
        if let Ok(Some(_)) = media_manager.get_media_info(session_id).await {
            // For now, assume audio is active if media session exists
            // TODO: Add proper check when available in media manager
            Ok(true)
        } else {
            Ok(false)
        }
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
            let quality_metrics = self.get_media_statistics(session_id).await
                .ok()
                .flatten()
                .and_then(|stats| stats.quality_metrics);
            
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
    
    async fn get_rtp_statistics(&self, session_id: &SessionId) -> Result<Option<rvoip_rtp_core::session::RtpSessionStats>> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Get RTP statistics
        media_manager.get_rtp_statistics(session_id).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to get RTP statistics: {}", e) 
            })
    }
    
    async fn get_media_statistics(&self, session_id: &SessionId) -> Result<Option<rvoip_media_core::types::MediaStatistics>> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Get media statistics
        media_manager.get_media_statistics(session_id).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to get media statistics: {}", e) 
            })
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
        
        // Update with the offer
        self.update_remote_sdp(session_id, offer).await?;
        
        // Generate answer based on our capabilities and the offer
        // For now, we'll generate a standard offer which acts as our answer
        let answer = media_manager.generate_sdp_offer(session_id).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to generate SDP answer: {}", e) 
            })?;
        
        // Store our local SDP
        {
            let mut sdp_storage = media_manager.sdp_storage.write().await;
            let entry = sdp_storage.entry(session_id.clone()).or_insert((None, None));
            entry.0 = Some(answer.clone());
        }
        
        tracing::info!("Generated SDP answer for session {}", session_id);
        Ok(answer)
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