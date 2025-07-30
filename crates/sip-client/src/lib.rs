//! # SIP Client - Unified VoIP Client Library
//!
//! This crate provides a unified, production-ready SIP client implementation that orchestrates:
//! - **client-core**: High-level SIP protocol handling and session management
//! - **audio-core**: Audio device management, format conversion, and pipeline processing  
//! - **codec-core**: Audio codec encoding/decoding (G.711, etc.)
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use rvoip_sip_client::SipClient;
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Simple one-line setup
//!     let client = SipClient::new("sip:alice@example.com").await?;
//!     
//!     // Make a call
//!     let call = client.call("sip:bob@example.com").await?;
//!     
//!     // Wait for answer
//!     call.wait_for_answer().await?;
//!     
//!     // Let the call run
//!     tokio::time::sleep(Duration::from_secs(30)).await;
//!     
//!     // Hangup
//!     call.hangup().await?;
//!     
//!     Ok(())
//! }
//! ```
//!
//! ## Architecture
//!
//! The library provides three levels of API:
//!
//! 1. **Simple API**: Quick setup with sensible defaults
//! 2. **Advanced API**: Full control over audio pipeline and codecs
//! 3. **Builder API**: Progressive disclosure of configuration options
//!
//! ## Features
//!
//! - Automatic codec negotiation
//! - Built-in echo cancellation and noise suppression
//! - Event-driven architecture for UI integration
//! - Zero-copy audio processing
//! - Comprehensive error handling

#![warn(missing_docs)]
#![doc(html_root_url = "https://docs.rs/rvoip-sip-client/0.1.0")]

pub mod error;
pub mod builder;
pub mod simple;
pub mod advanced;
pub mod events;
pub mod types;

// Re-export main types
pub use error::{SipClientError, SipClientResult};
pub use builder::SipClientBuilder;
pub use types::{Call, CallId, CallState, AudioConfig, CodecConfig};
pub use events::{SipClientEvent, EventStream};

#[cfg(feature = "simple-api")]
pub use simple::SipClient;

#[cfg(feature = "advanced-api")]
pub use advanced::{AdvancedSipClient, AudioPipelineConfig, CodecPriority};

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Initialize the SIP client library
///
/// This should be called once at application startup to initialize
/// all underlying components.
pub async fn init() -> SipClientResult<()> {
    // Initialize codec tables
    codec_core::init()?;
    
    // Initialize audio subsystem
    // audio_core doesn't have an init, but we could add one if needed
    
    tracing::info!("SIP Client library v{} initialized", VERSION);
    Ok(())
}