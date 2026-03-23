//! SDP offer/answer generation, statistics, and audio streaming operations

use std::collections::HashMap;
use chrono::{DateTime, Utc};

// Import session-core APIs
use rvoip_session_core::api::{
    SessionControl,
    MediaControl,
    SessionCoordinator,
    SessionId,
    MediaInfo as SessionMediaInfo,
    CallStatistics,
    MediaSessionStats,
    RtpSessionStats,
    QualityMetrics,
    QualityThresholds,
};

// Import client-core types
use crate::{
    ClientResult, ClientError,
    call::CallId,
    events::MediaEventInfo,
};

use super::super::types::*;

/// SDP, statistics, and streaming operations implementation for ClientManager
impl super::super::manager::ClientManager {
    pub async fn generate_sdp_answer(&self, call_id: &CallId, offer: &str) -> ClientResult<String> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Validate SDP offer
        if offer.trim().is_empty() {
            return Err(ClientError::InvalidConfiguration { 
                field: "sdp_offer".to_string(),
                reason: "SDP offer cannot be empty".to_string() 
            });
        }
        
        // Before generating SDP answer, configure session-core with our media preferences
        // This ensures the generated SDP reflects our configured codecs and capabilities
        
        // TODO: Once session-core supports setting codec preferences per-session,
        // we would do something like:
        // MediaControl::set_session_codecs(&self.coordinator, &session_id, &self.media_config.preferred_codecs).await?;
        
        // For now, session-core will use the codecs configured during initialization
        // The media config was passed when building the SessionCoordinator
            
        // Use session-core to generate SDP answer
        let sdp_answer = MediaControl::generate_sdp_answer(&self.coordinator, &session_id, offer)
            .await
            .map_err(|e| ClientError::InternalError { 
                message: format!("Failed to generate SDP answer: {}", e) 
            })?;
            
        // Post-process the SDP if needed based on media configuration
        let sdp_answer = self.apply_media_config_to_sdp(sdp_answer).await;
            
        // Update call metadata
        if let Some(mut call_info) = self.call_info.get_mut(call_id) {
            call_info.metadata.insert("last_sdp_answer".to_string(), sdp_answer.clone());
            call_info.metadata.insert("sdp_answer_generated_at".to_string(), Utc::now().to_rfc3339());
        }
        
        tracing::info!("Generated SDP answer for call {}: {} bytes", call_id, sdp_answer.len());
        Ok(sdp_answer)
    }
    
    /// Apply media configuration to generated SDP
    /// 
    /// Post-processes a generated SDP description by applying the client's media
    /// configuration preferences. This includes adding custom SDP attributes,
    /// bandwidth constraints, packet time (ptime) preferences, and other
    /// media-specific configuration options that customize the SDP for this client.
    /// 
    /// # Arguments
    /// 
    /// * `sdp` - The base SDP string to be modified
    /// 
    /// # Returns
    /// 
    /// Returns the modified SDP string with applied configuration preferences.
    /// This function always succeeds and returns a valid SDP.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # fn main() {
    /// // Basic SDP modification
    /// let base_sdp = "v=0\r\no=- 123 456 IN IP4 192.168.1.1\r\nm=audio 5004 RTP/AVP 0\r\n";
    /// println!("Base SDP: {} bytes", base_sdp.len());
    /// 
    /// // Example configuration applications
    /// let has_custom_attrs = true;
    /// let has_bandwidth_limit = true;
    /// let has_ptime_pref = true;
    /// 
    /// if has_custom_attrs {
    ///     println!("Would add custom SDP attributes");
    /// }
    /// if has_bandwidth_limit {
    ///     println!("Would add bandwidth constraint (b=AS:64)");
    /// }
    /// if has_ptime_pref {
    ///     println!("Would add packet time preference (a=ptime:20)");
    /// }
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # fn main() {
    /// // Enterprise customization example
    /// let original_sdp = "v=0\r\nm=audio 8000 RTP/AVP 111\r\na=rtpmap:111 OPUS/48000/2\r\n";
    /// 
    /// // Simulated configuration
    /// let max_bandwidth = 128; // kbps
    /// let preferred_ptime = 20; // ms
    /// let custom_attrs = vec![("a=sendrecv", ""), ("a=tool", "rvoip-client")];
    /// 
    /// println!("Original SDP: {} bytes", original_sdp.len());
    /// println!("Applying enterprise configuration:");
    /// println!("  Max bandwidth: {}kbps", max_bandwidth);
    /// println!("  Packet time: {}ms", preferred_ptime);
    /// println!("  Custom attributes: {} items", custom_attrs.len());
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # fn main() {
    /// // Quality optimization
    /// let sdp = "v=0\r\nm=audio 5004 RTP/AVP 0 8\r\n";
    /// 
    /// // Configuration for different scenarios
    /// let scenario = "low_bandwidth";
    /// match scenario {
    ///     "low_bandwidth" => {
    ///         println!("Applying low bandwidth optimizations");
    ///         println!("Would add: b=AS:32");
    ///     }
    ///     "high_quality" => {
    ///         println!("Applying high quality settings");
    ///         println!("Would add: b=AS:256");
    ///     }
    ///     _ => println!("Using default settings"),
    /// }
    /// # }
    /// ```
    /// 
    /// # Configuration Applied
    /// 
    /// 1. **Custom SDP Attributes**: Adds configured custom attributes after the media line
    /// 2. **Bandwidth Constraints**: Inserts `b=AS:` lines for bandwidth limits
    /// 3. **Packet Time**: Adds `a=ptime:` attributes for timing preferences
    /// 4. **Format Compliance**: Ensures proper SDP formatting with CRLF line endings
    /// 
    /// # Use Cases
    /// 
    /// - Enterprise policy enforcement in SDP
    /// - Network optimization for specific environments
    /// - Codec-specific parameter tuning
    /// - Quality of Service (QoS) configuration
    /// - Compliance with specific SIP server requirements
    async fn apply_media_config_to_sdp(&self, mut sdp: String) -> String {
        // Add custom attributes if configured
        if !self.media_config.custom_sdp_attributes.is_empty() {
            let mut lines: Vec<String> = sdp.lines().map(|s| s.to_string()).collect();
            
            // Find where to insert attributes (after the first m= line)
            if let Some(m_line_idx) = lines.iter().position(|line| line.starts_with("m=")) {
                let mut insert_idx = m_line_idx + 1;
                
                // Insert custom attributes
                for (key, value) in &self.media_config.custom_sdp_attributes {
                    lines.insert(insert_idx, format!("{}:{}", key, value));
                    insert_idx += 1;
                }
            }
            
            sdp = lines.join("\r\n");
            if !sdp.ends_with("\r\n") {
                sdp.push_str("\r\n");
            }
        }
        
        // Add bandwidth constraint if configured
        if let Some(max_bw) = self.media_config.max_bandwidth_kbps {
            if !sdp.contains("b=AS:") {
                let mut lines: Vec<String> = sdp.lines().map(|s| s.to_string()).collect();
                
                // Insert bandwidth after c= line
                if let Some(c_line_idx) = lines.iter().position(|line| line.starts_with("c=")) {
                    lines.insert(c_line_idx + 1, format!("b=AS:{}", max_bw));
                }
                
                sdp = lines.join("\r\n");
                if !sdp.ends_with("\r\n") {
                    sdp.push_str("\r\n");
                }
            }
        }
        
        // Add ptime if configured
        if let Some(ptime) = self.media_config.preferred_ptime {
            if !sdp.contains("a=ptime:") {
                // Add ptime attribute after the last a=rtpmap line
                let mut lines: Vec<String> = sdp.lines().map(|s| s.to_string()).collect();
                
                if let Some(last_rtpmap_idx) = lines.iter().rposition(|line| line.starts_with("a=rtpmap:")) {
                    lines.insert(last_rtpmap_idx + 1, format!("a=ptime:{}", ptime));
                }
                
                sdp = lines.join("\r\n");
                if !sdp.ends_with("\r\n") {
                    sdp.push_str("\r\n");
                }
            }
        }
        
        sdp
    }
    
    /// Establish media flow to a remote address
    /// 
    /// Establishes the actual media flow (RTP streams) between the local client and
    /// a specified remote address. This function configures the media session to
    /// begin transmitting and receiving audio packets to/from the designated endpoint.
    /// This is typically called after SDP negotiation is complete.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to establish media flow for
    /// * `remote_addr` - The remote address (IP:port) to establish media flow with
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` on successful establishment, or a `ClientError` if:
    /// - The call is not found
    /// - The remote address is invalid or unreachable
    /// - The underlying session-core fails to establish the media flow
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Establish media flow after SDP negotiation
    /// let call_id: CallId = Uuid::new_v4();
    /// let remote_addr = "192.168.1.20:5004";
    /// 
    /// println!("Would establish media flow for call {}", call_id);
    /// println!("Target remote address: {}", remote_addr);
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Direct media establishment for P2P calls
    /// let call_id: CallId = Uuid::new_v4();
    /// let peer_endpoint = "10.0.1.100:12000";
    /// 
    /// println!("Establishing direct P2P media to {}", peer_endpoint);
    /// println!("Call ID: {}", call_id);
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Media relay configuration
    /// let call_id: CallId = Uuid::new_v4();
    /// let relay_address = "relay.example.com:8000";
    /// 
    /// println!("Configuring media relay for call {}", call_id);
    /// println!("Relay endpoint: {}", relay_address);
    /// println!("Would establish RTP flows through media relay");
    /// # }
    /// ```
    /// 
    /// # Side Effects
    /// 
    /// - Updates call metadata with media flow status and remote address
    /// - Initiates RTP packet transmission/reception
    /// - Configures network routing for media streams
    /// - May trigger firewall/NAT traversal procedures
    /// 
    /// # Use Cases
    /// 
    /// - Completing call setup after SDP negotiation
    /// - Direct peer-to-peer media establishment
    /// - Media relay and proxy configurations
    /// - Network topology adaptation
    /// - Quality of Service (QoS) path establishment
    /// 
    /// # Network Considerations
    /// 
    /// The remote address should be reachable and the specified port should be
    /// available for RTP traffic. This function may trigger network discovery
    /// and NAT traversal procedures if required by the network topology.
    pub async fn establish_media(&self, call_id: &CallId, remote_addr: &str) -> ClientResult<()> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Use session-core to establish media flow
        MediaControl::establish_media_flow(&self.coordinator, &session_id, remote_addr)
            .await
            .map_err(|e| ClientError::InternalError { 
                message: format!("Failed to establish media flow: {}", e) 
            })?;
            
        // Update call metadata
        if let Some(mut call_info) = self.call_info.get_mut(call_id) {
            call_info.metadata.insert("media_flow_established".to_string(), "true".to_string());
            call_info.metadata.insert("remote_media_addr".to_string(), remote_addr.to_string());
        }
        
        tracing::info!("Established media flow for call {} to {}", call_id, remote_addr);
        Ok(())
    }
    
    /// Get RTP statistics for a call
    /// 
    /// Retrieves detailed Real-time Transport Protocol (RTP) statistics for the specified call,
    /// including packet counts, byte counts, jitter measurements, and packet loss metrics.
    /// This information is crucial for monitoring call quality and diagnosing network issues.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to get RTP statistics for
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(Some(RtpSessionStats))` with detailed RTP metrics if available,
    /// `Ok(None)` if no RTP session exists, or `ClientError` if the call is not found
    /// or statistics cannot be retrieved.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Monitor call quality
    /// let call_id: CallId = Uuid::new_v4();
    /// println!("Would get RTP statistics for call {}", call_id);
    /// 
    /// // Example statistics evaluation
    /// let packets_sent = 1000u64;
    /// let packets_lost = 5u64;
    /// let loss_rate = (packets_lost as f64 / packets_sent as f64) * 100.0;
    /// 
    /// if loss_rate > 5.0 {
    ///     println!("High packet loss detected: {:.2}%", loss_rate);
    /// } else {
    ///     println!("Good quality: {:.2}% packet loss", loss_rate);
    /// }
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Network diagnostics
    /// let call_id: CallId = Uuid::new_v4();
    /// println!("Running network diagnostics for call {}", call_id);
    /// println!("Would analyze jitter, latency, and throughput");
    /// # }
    /// ```
    /// 
    /// # Use Cases
    /// 
    /// - Real-time call quality monitoring
    /// - Network performance analysis
    /// - Troubleshooting audio issues
    /// - Quality of Service (QoS) reporting
    pub async fn get_rtp_statistics(&self, call_id: &CallId) -> ClientResult<Option<RtpSessionStats>> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        MediaControl::get_rtp_statistics(&self.coordinator, &session_id)
            .await
            .map_err(|e| ClientError::InternalError { 
                message: format!("Failed to get RTP statistics: {}", e) 
            })
    }
    
    /// Get comprehensive media statistics for a call
    /// 
    /// Retrieves complete media session statistics including RTP/RTCP metrics, quality
    /// measurements, and performance indicators. This provides a holistic view of the
    /// media session's health and performance characteristics.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to get media statistics for
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(Some(MediaSessionStats))` with comprehensive media metrics if available,
    /// `Ok(None)` if no media session exists, or `ClientError` if the call is not found
    /// or statistics cannot be retrieved.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Comprehensive call analysis
    /// let call_id: CallId = Uuid::new_v4();
    /// println!("Would get comprehensive media statistics for call {}", call_id);
    /// 
    /// // Example quality assessment
    /// let audio_quality_score = 4.2; // Out of 5.0
    /// let network_quality = "Good"; // Based on metrics
    /// 
    /// println!("Audio Quality: {:.1}/5.0", audio_quality_score);
    /// println!("Network Quality: {}", network_quality);
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Performance monitoring dashboard
    /// let call_id: CallId = Uuid::new_v4();
    /// println!("Updating performance dashboard for call {}", call_id);
    /// println!("Would include RTP, RTCP, jitter, and codec metrics");
    /// # }
    /// ```
    /// 
    /// # Use Cases
    /// 
    /// - Call quality dashboards
    /// - Performance monitoring systems
    /// - Troubleshooting complex media issues
    /// - Historical call quality analysis
    pub async fn get_media_statistics(&self, call_id: &CallId) -> ClientResult<Option<MediaSessionStats>> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        MediaControl::get_media_statistics(&self.coordinator, &session_id)
            .await
            .map_err(|e| ClientError::InternalError { 
                message: format!("Failed to get media statistics: {}", e) 
            })
    }
    
    /// Get comprehensive call statistics for a call
    /// 
    /// Retrieves complete call statistics encompassing all aspects of the call including
    /// RTP metrics, quality measurements, call duration, and detailed performance data.
    /// This provides the most comprehensive view of call performance and quality.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to get complete statistics for
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(Some(CallStatistics))` with complete call metrics if available,
    /// `Ok(None)` if no call statistics exist, or `ClientError` if the call is not found
    /// or statistics cannot be retrieved.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # use std::time::Duration;
    /// # fn main() {
    /// // Complete call analysis
    /// let call_id: CallId = Uuid::new_v4();
    /// println!("Would get complete call statistics for call {}", call_id);
    /// 
    /// // Example comprehensive metrics
    /// let call_duration = Duration::from_secs(300); // 5 minutes
    /// let avg_jitter = 15; // milliseconds
    /// let packet_loss = 0.8; // percent
    /// 
    /// println!("Call Duration: {:?}", call_duration);
    /// println!("Average Jitter: {}ms", avg_jitter);
    /// println!("Packet Loss: {:.1}%", packet_loss);
    /// # }
    /// ```
    /// 
    /// ```rust
    /// # use uuid::Uuid;
    /// # use rvoip_client_core::call::CallId;
    /// # fn main() {
    /// // Call quality reporting
    /// let call_id: CallId = Uuid::new_v4();
    /// println!("Generating call quality report for call {}", call_id);
    /// println!("Would include all RTP, quality, and performance metrics");
    /// # }
    /// ```
    /// 
    /// # Use Cases
    /// 
    /// - Post-call quality reports
    /// - Billing and usage analytics
    /// - Network performance analysis
    /// - Customer experience metrics
    /// - SLA compliance monitoring
    pub async fn get_call_statistics(&self, call_id: &CallId) -> ClientResult<Option<CallStatistics>> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        MediaControl::get_call_statistics(&self.coordinator, &session_id)
            .await
            .map_err(|e| ClientError::InternalError { 
                message: format!("Failed to get call statistics: {}", e) 
            })
    }
    

    
    // =============================================================================
    // REAL-TIME AUDIO STREAMING API
    // =============================================================================
    
    /// Subscribe to audio frames from a call for real-time playback
    /// 
    /// Returns a subscriber that receives decoded audio frames from the RTP stream
    /// for the specified call. These frames can be played through speakers or
    /// processed for audio analysis.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to subscribe to
    /// 
    /// # Returns
    /// 
    /// Returns an `AudioFrameSubscriber` that can be used to receive audio frames.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::{ClientManager, call::CallId};
    /// # use std::sync::Arc;
    /// # async fn example(client: Arc<ClientManager>, call_id: CallId) -> Result<(), Box<dyn std::error::Error>> {
    /// // Subscribe to incoming audio frames
    /// let subscriber = client.subscribe_to_audio_frames(&call_id).await?;
    /// 
    /// // Process frames in a background task
    /// tokio::spawn(async move {
    ///     while let Ok(frame) = subscriber.recv() {
    ///         // Play frame through speakers or process it
    ///         println!("Received audio frame: {} samples at {}Hz", 
    ///                  frame.samples.len(), frame.sample_rate);
    ///     }
    /// });
    /// # Ok(())
    /// # }
    /// ```
    pub async fn subscribe_to_audio_frames(&self, call_id: &CallId) -> ClientResult<rvoip_session_core::api::types::AudioFrameSubscriber> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Use session-core to subscribe to audio frames
        MediaControl::subscribe_to_audio_frames(&self.coordinator, &session_id)
            .await
            .map_err(|e| ClientError::CallSetupFailed { 
                reason: format!("Failed to subscribe to audio frames: {}", e) 
            })
    }
    
    /// Send an audio frame for encoding and transmission
    /// 
    /// Sends an audio frame to be encoded and transmitted via RTP for the specified call.
    /// This is typically used for microphone input or generated audio content.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to send audio on
    /// * `audio_frame` - The audio frame to send
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` if the frame was sent successfully.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::{ClientManager, call::CallId};
    /// # use rvoip_session_core::api::types::AudioFrame;
    /// # use std::sync::Arc;
    /// # async fn example(client: Arc<ClientManager>, call_id: CallId) -> Result<(), Box<dyn std::error::Error>> {
    /// // Create an audio frame (typically from microphone)
    /// let samples = vec![0; 160]; // 20ms of silence at 8kHz
    /// let frame = AudioFrame::new(samples, 8000, 1, 12345);
    /// 
    /// // Send the frame for transmission
    /// client.send_audio_frame(&call_id, frame).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_audio_frame(&self, call_id: &CallId, audio_frame: rvoip_session_core::api::types::AudioFrame) -> ClientResult<()> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Use session-core to send audio frame
        MediaControl::send_audio_frame(&self.coordinator, &session_id, audio_frame)
            .await
            .map_err(|e| ClientError::CallSetupFailed { 
                reason: format!("Failed to send audio frame: {}", e) 
            })
    }
    
    /// Get current audio stream configuration for a call
    /// 
    /// Returns the current audio streaming configuration for the specified call,
    /// including sample rate, channels, codec, and processing settings.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call
    /// 
    /// # Returns
    /// 
    /// Returns the current `AudioStreamConfig` or `None` if no stream is configured.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::{ClientManager, call::CallId};
    /// # use std::sync::Arc;
    /// # async fn example(client: Arc<ClientManager>, call_id: CallId) -> Result<(), Box<dyn std::error::Error>> {
    /// if let Some(config) = client.get_audio_stream_config(&call_id).await? {
    ///     println!("Audio stream: {}Hz, {} channels, codec: {}", 
    ///              config.sample_rate, config.channels, config.codec);
    /// } else {
    ///     println!("No audio stream configured for call");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_audio_stream_config(&self, call_id: &CallId) -> ClientResult<Option<rvoip_session_core::api::types::AudioStreamConfig>> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Use session-core to get audio stream config
        MediaControl::get_audio_stream_config(&self.coordinator, &session_id)
            .await
            .map_err(|e| ClientError::CallSetupFailed { 
                reason: format!("Failed to get audio stream config: {}", e) 
            })
    }
    
    /// Set audio stream configuration for a call
    /// 
    /// Configures the audio streaming parameters for the specified call,
    /// including sample rate, channels, codec preferences, and audio processing settings.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call
    /// * `config` - The audio stream configuration to apply
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` if the configuration was applied successfully.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::{ClientManager, call::CallId};
    /// # use rvoip_session_core::api::types::AudioStreamConfig;
    /// # use std::sync::Arc;
    /// # async fn example(client: Arc<ClientManager>, call_id: CallId) -> Result<(), Box<dyn std::error::Error>> {
    /// // Configure high-quality audio stream
    /// let config = AudioStreamConfig {
    ///     sample_rate: 48000,
    ///     channels: 1,
    ///     codec: "Opus".to_string(),
    ///     frame_size_ms: 20,
    ///     enable_aec: true,
    ///     enable_agc: true,
    ///     enable_vad: true,
    /// };
    /// 
    /// client.set_audio_stream_config(&call_id, config).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_audio_stream_config(&self, call_id: &CallId, config: rvoip_session_core::api::types::AudioStreamConfig) -> ClientResult<()> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Use session-core to set audio stream config
        MediaControl::set_audio_stream_config(&self.coordinator, &session_id, config)
            .await
            .map_err(|e| ClientError::CallSetupFailed { 
                reason: format!("Failed to set audio stream config: {}", e) 
            })
    }
    
    /// Start audio streaming for a call
    /// 
    /// Begins the audio streaming pipeline for the specified call, enabling
    /// real-time audio frame processing. This must be called before audio frames
    /// can be sent or received.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` if the audio stream started successfully.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::{ClientManager, call::CallId};
    /// # use rvoip_session_core::api::types::AudioStreamConfig;
    /// # use std::sync::Arc;
    /// # async fn example(client: Arc<ClientManager>, call_id: CallId) -> Result<(), Box<dyn std::error::Error>> {
    /// // Configure and start audio streaming
    /// let config = AudioStreamConfig {
    ///     sample_rate: 8000,
    ///     channels: 1,
    ///     codec: "PCMU".to_string(),
    ///     frame_size_ms: 20,
    ///     enable_aec: true,
    ///     enable_agc: true,
    ///     enable_vad: true,
    /// };
    /// 
    /// client.set_audio_stream_config(&call_id, config).await?;
    /// client.start_audio_stream(&call_id).await?;
    /// println!("Audio streaming started for call {}", call_id);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn start_audio_stream(&self, call_id: &CallId) -> ClientResult<()> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Use session-core to start audio stream
        MediaControl::start_audio_stream(&self.coordinator, &session_id)
            .await
            .map_err(|e| ClientError::CallSetupFailed { 
                reason: format!("Failed to start audio stream: {}", e) 
            })
    }
    
    /// Stop audio streaming for a call
    /// 
    /// Stops the audio streaming pipeline for the specified call, disabling
    /// real-time audio frame processing. This cleans up resources and stops
    /// audio transmission.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` if the audio stream stopped successfully.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::{ClientManager, call::CallId};
    /// # use std::sync::Arc;
    /// # async fn example(client: Arc<ClientManager>, call_id: CallId) -> Result<(), Box<dyn std::error::Error>> {
    /// // Stop audio streaming
    /// client.stop_audio_stream(&call_id).await?;
    /// println!("Audio streaming stopped for call {}", call_id);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn stop_audio_stream(&self, call_id: &CallId) -> ClientResult<()> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Use session-core to stop audio stream
        MediaControl::stop_audio_stream(&self.coordinator, &session_id)
            .await
            .map_err(|e| ClientError::CallSetupFailed { 
                reason: format!("Failed to stop audio stream: {}", e) 
            })
    }
}
