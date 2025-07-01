//! High-level SIP client library for VoIP applications
//! 
//! This crate provides a user-friendly API for building SIP/VoIP client applications.
//! It handles the complexity of SIP signaling, media negotiation, and call management
//! while presenting a simple, async-first interface.
//! 
//! # Architecture
//! 
//! ```text
//! ┌─────────────────────────┐
//! │    Your Application     │
//! └───────────┬─────────────┘
//!             │ 
//! ┌───────────▼─────────────┐
//! │     client-core         │ ◄── You are here
//! │ ┌─────────────────────┐ │
//! │ │   ClientManager     │ │     • High-level call control
//! │ │   Registration      │ │     • Event handling  
//! │ │   Media Control     │ │     • Clean async API
//! │ └─────────────────────┘ │
//! └───────────┬─────────────┘
//!             │
//! ┌───────────▼─────────────┐
//! │     session-core        │     • Session management
//! │                         │     • Protocol coordination
//! └───────────┬─────────────┘     • Infrastructure abstraction
//!             │
//! ┌───────────▼─────────────┐
//! │   Lower-level crates    │     • SIP, RTP, Media
//! │ (transaction, dialog,   │     • Transport layers
//! │  sip-core, etc.)        │     • Codec processing
//! └─────────────────────────┘
//! ```
//! 
//! # Quick Start
//! 
//! ## Basic Client Setup
//! 
//! ```rust
//! use rvoip_client_core::{ClientBuilder, Client, ClientEvent};
//! use std::sync::Arc;
//! 
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Build and start a client
//!     let client = ClientBuilder::new()
//!         .user_agent("MyApp/1.0")
//!         .local_address("0.0.0.0:5060".parse()?)
//!         .build()
//!         .await?;
//!         
//!     client.start().await?;
//!     
//!     // Subscribe to events
//!     let mut events = client.subscribe_events();
//!     
//!     // Make a call
//!     let call_id = client.make_call(
//!         "sip:alice@example.com".to_string(),
//!         "sip:bob@example.com".to_string(),
//!         None, // Let session-core generate SDP
//!     ).await?;
//!     
//!     // Handle events
//!     while let Ok(event) = events.recv().await {
//!         match event {
//!             ClientEvent::CallStateChanged { info, .. } => {
//!                 println!("Call {} state: {:?}", info.call_id, info.new_state);
//!             }
//!             _ => {}
//!         }
//!     }
//!     
//!     Ok(())
//! }
//! ```
//! 
//! ## Registration Example
//! 
//! ```rust
//! # use rvoip_client_core::{ClientBuilder, registration::RegistrationConfig};
//! # use std::sync::Arc;
//! # async fn example(client: Arc<rvoip_client_core::Client>) -> Result<(), Box<dyn std::error::Error>> {
//! // Register with a SIP server
//! let config = RegistrationConfig::new(
//!     "sip:registrar.example.com".to_string(),
//!     "sip:alice@example.com".to_string(),
//!     "sip:alice@192.168.1.100:5060".to_string(),
//! )
//! .with_credentials("alice".to_string(), "secret123".to_string())
//! .with_expires(3600); // 1 hour
//! 
//! let reg_id = client.register(config).await?;
//! 
//! // Later, refresh the registration
//! client.refresh_registration(reg_id).await?;
//! 
//! // Or unregister
//! client.unregister(reg_id).await?;
//! # Ok(())
//! # }
//! ```
//! 
//! # Features
//! 
//! - **Call Management**: Make, receive, hold, transfer calls
//! - **Registration**: SIP REGISTER support with authentication
//! - **Media Control**: Audio mute/unmute, codec selection, SDP handling
//! - **Event System**: Async event notifications for all operations
//! - **Clean Architecture**: All complexity handled through session-core
//! 
//! # Error Handling
//! 
//! All operations return `ClientResult<T>` which wraps `ClientError`:
//! 
//! ```rust
//! # use rvoip_client_core::{Client, ClientError};
//! # use std::sync::Arc;
//! # async fn example(client: Arc<Client>) -> Result<(), Box<dyn std::error::Error>> {
//! match client.make_call("sip:alice@example.com".to_string(), "sip:bob@example.com".to_string(), None).await {
//!     Ok(call_id) => println!("Call started: {}", call_id),
//!     Err(ClientError::NetworkError { reason }) => {
//!         eprintln!("Network problem: {}", reason);
//!         // Retry or notify user
//!     }
//!     Err(e) => eprintln!("Call failed: {}", e),
//! }
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]
#![doc(html_root_url = "https://docs.rs/rvoip-client-core/0.1.0")]

pub mod client;
pub mod call;
pub mod registration;
pub mod events;
pub mod error;

// Public API exports (only high-level client-core types)
pub use client::{
    ClientManager,
    ClientCallHandler,
    Client,
    ClientBuilder,
    // All types are now re-exported from types module via client::mod
    ClientStats,
    CallCapabilities,
    CallMediaInfo,
    AudioCodecInfo,
    AudioDirection,
    AudioQualityMetrics,
    MediaCapabilities,
    MediaSessionInfo,
    NegotiatedMediaParams,
    EnhancedMediaCapabilities,
    ClientConfig,
    MediaConfig,
    MediaPreset,
    MediaConfigBuilder,
};
pub use call::{CallId, CallInfo, CallDirection, CallState};
pub use registration::{RegistrationConfig, RegistrationInfo, RegistrationStatus};
pub use events::{
    ClientEventHandler, 
    ClientEvent, 
    IncomingCallInfo, 
    CallStatusInfo, 
    RegistrationStatusInfo,
    CallAction,
    MediaEventType,
    MediaEventInfo,
    EventFilter,
    EventPriority,
    EventSubscription,
    EventEmitter,
};
pub use error::{ClientError, ClientResult};

// Re-export commonly used types from session-core (for convenience)
pub use rvoip_session_core::api::types::SessionId;

/// Client-core version information
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    
    #[tokio::test]
    async fn test_client_manager_creation() {
        // Test that we can create a ClientManager
        let config = ClientConfig::new()
            .with_sip_addr("127.0.0.1:5080".parse().unwrap())
            .with_media_addr("127.0.0.1:5081".parse().unwrap());
        
        let client_result = ClientManager::new(config).await;
        assert!(client_result.is_ok(), "ClientManager creation should succeed");
        
        let _client = client_result.unwrap();
        // Test passes if we can create the client without panicking
    }
    
    #[tokio::test]
    async fn test_client_manager_lifecycle() {
        // Test start/stop lifecycle
        let config = ClientConfig::new()
            .with_sip_addr("127.0.0.1:5082".parse().unwrap())
            .with_media_addr("127.0.0.1:5083".parse().unwrap());
        
        let client = ClientManager::new(config).await
            .expect("Failed to create client");
        
        // Test start
        let start_result = client.start().await;
        assert!(start_result.is_ok(), "Client start should succeed");
        
        // Test stop
        let stop_result = client.stop().await;
        assert!(stop_result.is_ok(), "Client stop should succeed");
    }
    
    // Helper event handler for testing
    struct TestEventHandler;
    
    #[async_trait::async_trait]
    impl ClientEventHandler for TestEventHandler {
        async fn on_incoming_call(&self, _call_info: IncomingCallInfo) -> CallAction {
            CallAction::Accept
        }
        
        async fn on_call_state_changed(&self, _status_info: CallStatusInfo) {
            // No-op for test
        }
        
        async fn on_registration_status_changed(&self, _status_info: RegistrationStatusInfo) {
            // No-op for test
        }
        
        async fn on_media_event(&self, _media_info: MediaEventInfo) {
            // No-op for test  
        }
        
        async fn on_client_error(&self, _error: ClientError, _call_id: Option<CallId>) {
            // No-op for test
        }
        
        async fn on_network_event(&self, _connected: bool, _reason: Option<String>) {
            // No-op for test
        }
    }

    #[tokio::test]
    async fn test_client_core_compiles() {
        // Basic compilation test
        assert!(true);
    }
    
    #[tokio::test]
    async fn test_registration_not_implemented() {
        // Test that registration operations return NotImplemented error
        let config = ClientConfig::new()
            .with_sip_addr("127.0.0.1:5084".parse().unwrap())
            .with_media_addr("127.0.0.1:5085".parse().unwrap());
        
        let client = ClientManager::new(config).await.unwrap();
        let reg_config = RegistrationConfig::new(
            "sip:server.example.com".to_string(),
            "sip:user@example.com".to_string(),
            "sip:user@127.0.0.1:5084".to_string(),
        );
        
        let result = client.register(reg_config).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ClientError::NotImplemented { .. } => {
                // Expected - registration not available in session-core
            }
            ClientError::InternalError { message } if message.contains("Registration failed") => {
                // Also acceptable - session-core might have registration support now
            }
            other => panic!("Expected NotImplemented or InternalError, got: {:?}", other),
        }
    }
    
    #[tokio::test]
    async fn test_priority_4_1_media_integration() {
        // Test Priority 4.1: Enhanced Media Integration APIs
        let config = ClientConfig::new()
            .with_sip_addr("127.0.0.1:5086".parse().unwrap())
            .with_media_addr("127.0.0.1:5087".parse().unwrap());
        
        let client = ClientManager::new(config).await.unwrap();
        let handler = Arc::new(TestEventHandler);
        client.set_event_handler(handler).await;
        
        // Start the client
        client.start().await.unwrap();
        
        // Test media capabilities API
        let capabilities = client.get_media_capabilities().await;
        assert!(capabilities.can_mute_microphone);
        assert!(capabilities.can_mute_speaker);
        assert!(capabilities.can_hold);
        assert!(capabilities.can_send_dtmf);
        assert!(capabilities.supports_rtp);
        assert!(!capabilities.supported_codecs.is_empty());
        
        // Test codec enumeration
        let codecs = client.get_available_codecs().await;
        assert!(!codecs.is_empty());
        
        // Verify standard codecs are available
        let codec_names: Vec<String> = codecs.iter().map(|c| c.name.clone()).collect();
        assert!(codec_names.contains(&"PCMU".to_string()));
        assert!(codec_names.contains(&"PCMA".to_string()));
        assert!(codec_names.contains(&"OPUS".to_string()));
        
        // Test codec preference setting
        let preferred_codecs = vec!["OPUS".to_string(), "PCMU".to_string()];
        let result = client.set_preferred_codecs(preferred_codecs).await;
        assert!(result.is_ok());
        
        // Test with a mock call (this would normally fail without an actual session)
        // but we're testing the API structure
        let fake_call_id = CallId::new_v4();
        
        // Test media info for non-existent call (should fail gracefully)
        let media_info_result = client.get_call_media_info(&fake_call_id).await;
        assert!(media_info_result.is_err());
        match media_info_result.unwrap_err() {
            ClientError::CallNotFound { .. } => {
                // Expected for non-existent call
            }
            other => panic!("Expected CallNotFound error, got: {:?}", other),
        }
        
        // Test mute state for non-existent call
        let mute_result = client.get_microphone_mute_state(&fake_call_id).await;
        assert!(mute_result.is_err());
        
        // Test speaker mute state for non-existent call
        let speaker_mute_result = client.get_speaker_mute_state(&fake_call_id).await;
        assert!(speaker_mute_result.is_err());
        
        // Test audio transmission status for non-existent call
        let audio_active_result = client.is_audio_transmission_active(&fake_call_id).await;
        assert!(audio_active_result.is_err());
        
        client.stop().await.unwrap();
        
        println!("✅ Priority 4.1 Media Integration APIs validated successfully!");
    }
    
    #[tokio::test]
    async fn test_priority_4_2_media_session_coordination() {
        // Test Priority 4.2: Media Session Coordination APIs
        let config = ClientConfig::new()
            .with_sip_addr("127.0.0.1:5088".parse().unwrap())
            .with_media_addr("127.0.0.1:5089".parse().unwrap());
        
        let client = ClientManager::new(config).await.unwrap();
        let handler = Arc::new(TestEventHandler);
        client.set_event_handler(handler).await;
        
        // Start the client
        client.start().await.unwrap();
        
        // Test enhanced media capabilities
        let enhanced_capabilities = client.get_enhanced_media_capabilities().await;
        assert!(enhanced_capabilities.supports_sdp_offer_answer);
        assert!(enhanced_capabilities.supports_media_session_lifecycle);
        assert!(enhanced_capabilities.supports_sdp_renegotiation);
        assert!(enhanced_capabilities.supports_early_media);
        assert!(enhanced_capabilities.supports_media_session_updates);
        assert!(enhanced_capabilities.supports_codec_negotiation);
        assert_eq!(enhanced_capabilities.supported_sdp_version, "0");
        assert_eq!(enhanced_capabilities.preferred_rtp_port_range, (10000, 20000));
        assert!(enhanced_capabilities.supported_transport_protocols.contains(&"RTP/AVP".to_string()));
        
        // Test with fake call ID (APIs should fail gracefully)
        let fake_call_id = CallId::new_v4();
        
        // Test SDP generation for non-existent call
        let sdp_offer_result = client.generate_sdp_offer(&fake_call_id).await;
        assert!(sdp_offer_result.is_err());
        match sdp_offer_result.unwrap_err() {
            ClientError::CallNotFound { .. } => {
                // Expected for non-existent call
            }
            other => panic!("Expected CallNotFound error, got: {:?}", other),
        }
        
        // Test SDP answer processing for non-existent call
        let sdp_answer_result = client.process_sdp_answer(&fake_call_id, "v=0\r\nm=audio 5004 RTP/AVP 0\r\n").await;
        assert!(sdp_answer_result.is_err());
        
        // Test empty SDP answer validation
        let empty_sdp_result = client.process_sdp_answer(&fake_call_id, "").await;
        assert!(empty_sdp_result.is_err());
        match empty_sdp_result.unwrap_err() {
            ClientError::InvalidConfiguration { field, .. } => {
                assert_eq!(field, "sdp_answer");
            }
            other => panic!("Expected InvalidConfiguration error, got: {:?}", other),
        }
        
        // Test media session lifecycle for non-existent call
        let start_media_result = client.start_media_session(&fake_call_id).await;
        assert!(start_media_result.is_err());
        
        let stop_media_result = client.stop_media_session(&fake_call_id).await;
        assert!(stop_media_result.is_err());
        
        // Test media session status for non-existent call
        let is_active_result = client.is_media_session_active(&fake_call_id).await;
        assert!(is_active_result.is_err());
        
        // Test media session info for non-existent call
        let session_info_result = client.get_media_session_info(&fake_call_id).await;
        assert!(session_info_result.is_err());
        
        // Test negotiated media params for non-existent call
        let negotiated_params_result = client.get_negotiated_media_params(&fake_call_id).await;
        assert!(negotiated_params_result.is_err());
        
        // Test media session update for non-existent call
        let update_result = client.update_media_session(&fake_call_id, "v=0\r\nm=audio 5006 RTP/AVP 0\r\n").await;
        assert!(update_result.is_err());
        
        // Test empty SDP update validation
        let empty_update_result = client.update_media_session(&fake_call_id, "").await;
        assert!(empty_update_result.is_err());
        match empty_update_result.unwrap_err() {
            ClientError::InvalidConfiguration { field, .. } => {
                assert_eq!(field, "new_sdp");
            }
            other => panic!("Expected InvalidConfiguration error, got: {:?}", other),
        }
        
        client.stop().await.unwrap();
        
        println!("✅ Priority 4.2 Media Session Coordination APIs validated successfully!");
    }
} 