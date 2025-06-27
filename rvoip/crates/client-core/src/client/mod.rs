//! High-level SIP client implementation
//! 
//! This module provides the core client functionality for VoIP applications.
//! 
//! # Architecture Overview
//! 
//! The client module is organized into several sub-modules:
//! 
//! - **`manager`** - The main ClientManager that coordinates all operations
//! - **`calls`** - Call management functionality (make, answer, hangup)
//! - **`registration`** - SIP registration handling  
//! - **`media`** - Media session control (audio mute, codec selection)
//! - **`handlers`** - Event handlers and callbacks
//! 
//! # Usage Guide
//! 
//! ## Basic Call Flow
//! 
//! ```rust,no_run
//! # use rvoip_client_core::{ClientBuilder, ClientEvent, call::CallState};
//! # use std::time::Duration;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // 1. Create and start client
//! let client = ClientBuilder::new()
//!     .user_agent("MyApp/1.0")
//!     .build()
//!     .await?;
//! 
//! client.start().await?;
//! 
//! // 2. Subscribe to events
//! let mut events = client.subscribe_events();
//! 
//! // 3. Make a call
//! let call_id = client.make_call(
//!     "sip:alice@example.com".to_string(),
//!     "sip:bob@example.com".to_string(),
//!     None,
//! ).await?;
//! 
//! // 4. Handle events
//! tokio::spawn(async move {
//!     while let Ok(event) = events.recv().await {
//!         match event {
//!             ClientEvent::CallStateChanged { info, .. } => {
//!                 match info.new_state {
//!                     CallState::Connected => {
//!                         println!("Call connected!");
//!                     }
//!                     CallState::Terminated => {
//!                         println!("Call ended");
//!                         break;
//!                     }
//!                     _ => {}
//!                 }
//!             }
//!             _ => {}
//!         }
//!     }
//! });
//! 
//! // 5. Control the call
//! tokio::time::sleep(Duration::from_secs(30)).await;
//! client.hangup_call(&call_id).await?;
//! # Ok(())
//! # }
//! ```
//! 
//! ## Best Practices
//! 
//! ### 1. Always Handle Events
//! 
//! The event system is crucial for tracking call state and handling errors:
//! 
//! ```rust,no_run
//! # use rvoip_client_core::{Client, ClientEvent};
//! # use std::sync::Arc;
//! # async fn example(client: Arc<Client>) {
//! let mut events = client.subscribe_events();
//! 
//! // Spawn a dedicated event handler task
//! tokio::spawn(async move {
//!     while let Ok(event) = events.recv().await {
//!         match event {
//!             ClientEvent::Error { details, .. } => {
//!                 eprintln!("Client error: {}", details);
//!                 // Implement recovery logic here
//!             }
//!             ClientEvent::NetworkStateChanged { connected, .. } => {
//!                 if !connected {
//!                     // Handle network disconnection
//!                 }
//!             }
//!             _ => {}
//!         }
//!     }
//! });
//! # }
//! ```
//! 
//! ### 2. Proper Resource Cleanup
//! 
//! Always clean up resources when shutting down:
//! 
//! ```rust,no_run
//! # use rvoip_client_core::Client;
//! # use std::sync::Arc;
//! # async fn example(client: Arc<Client>) -> Result<(), Box<dyn std::error::Error>> {
//! // Unregister all active registrations
//! for reg_id in client.get_all_registrations().await {
//!     let _ = client.unregister(reg_id).await;
//! }
//! 
//! // Hang up all active calls
//! for call in client.get_active_calls().await {
//!     let _ = client.hangup_call(&call.id).await;
//! }
//! 
//! // Stop the client
//! client.stop().await?;
//! # Ok(())
//! # }
//! ```
//! 
//! ### 3. Registration Management
//! 
//! Keep registrations fresh and handle failures:
//! 
//! ```rust,no_run
//! # use rvoip_client_core::Client;
//! # use std::sync::Arc;
//! # use std::time::Duration;
//! # async fn example(client: Arc<Client>) -> Result<(), Box<dyn std::error::Error>> {
//! // Register with retry logic
//! let mut attempts = 0;
//! let reg_id = loop {
//!     match client.register_simple(
//!         "sip:alice@example.com",
//!         "registrar.example.com:5060",
//!         Duration::from_secs(3600)
//!     ).await {
//!         Ok(id) => break id,
//!         Err(e) if attempts < 3 => {
//!             attempts += 1;
//!             eprintln!("Registration attempt {} failed: {}", attempts, e);
//!             tokio::time::sleep(Duration::from_secs(5)).await;
//!         }
//!         Err(e) => return Err(e.into()),
//!     }
//! };
//! 
//! // Refresh registration periodically
//! let refresh_client = client.clone();
//! tokio::spawn(async move {
//!     loop {
//!         tokio::time::sleep(Duration::from_secs(3000)).await; // 50 minutes
//!         if let Err(e) = refresh_client.refresh_registration(reg_id).await {
//!             eprintln!("Failed to refresh registration: {}", e);
//!         }
//!     }
//! });
//! # Ok(())
//! # }
//! ```
//! 
//! ### 4. Media Control
//! 
//! Handle media operations gracefully:
//! 
//! ```rust,no_run
//! # use rvoip_client_core::Client;
//! # use std::sync::Arc;
//! # async fn example(client: Arc<Client>, call_id: rvoip_client_core::CallId) -> Result<(), Box<dyn std::error::Error>> {
//! // Mute/unmute with error handling
//! match client.set_microphone_mute(&call_id, true).await {
//!     Ok(_) => println!("Microphone muted"),
//!     Err(e) => eprintln!("Failed to mute: {}", e),
//! }
//! 
//! // Get media info before operations
//! if let Ok(Some(info)) = client.get_call_media_info(&call_id).await {
//!     println!("Current codec: {:?}", info.codec);
//!     println!("RTP port: {:?}", info.local_rtp_port);
//! }
//! # Ok(())
//! # }
//! ```
//! 
//! ## Common Patterns
//! 
//! ### Auto-Answer Incoming Calls
//! 
//! ```rust,no_run
//! # use rvoip_client_core::{Client, ClientEvent};
//! # use std::sync::Arc;
//! # async fn example(client: Arc<Client>) {
//! let answer_client = client.clone();
//! let mut events = client.subscribe_events();
//! 
//! tokio::spawn(async move {
//!     while let Ok(event) = events.recv().await {
//!         if let ClientEvent::IncomingCall { info, .. } = event {
//!             // Auto-answer after 2 seconds
//!             let client = answer_client.clone();
//!             let call_id = info.call_id.clone();
//!             tokio::spawn(async move {
//!                 tokio::time::sleep(std::time::Duration::from_secs(2)).await;
//!                 let _ = client.answer_call(&call_id).await;
//!             });
//!         }
//!     }
//! });
//! # }
//! ```
//! 
//! ### Call Transfer
//! 
//! ```rust,no_run
//! # use rvoip_client_core::Client;
//! # use std::sync::Arc;
//! # async fn example(client: Arc<Client>, call_id: rvoip_client_core::CallId) -> Result<(), Box<dyn std::error::Error>> {
//! // Attended transfer
//! client.hold_call(&call_id).await?;
//! let new_call = client.make_call(
//!     "sip:alice@example.com".to_string(),
//!     "sip:charlie@example.com".to_string(),
//!     Some("Transferring Bob's call".to_string()),
//! ).await?;
//! 
//! // Wait for answer...
//! // Then transfer
//! client.transfer_call(&call_id, "sip:charlie@example.com").await?;
//! # Ok(())
//! # }
//! ```

pub mod calls;
pub mod config;
pub mod controls;
pub mod events;
pub mod manager;
pub mod media;
pub mod media_builder;
pub mod recovery;
pub mod registration;
pub mod types;
pub mod builder;

#[cfg(test)]
pub mod tests;

pub use manager::ClientManager;
pub use config::{ClientConfig, MediaConfig, MediaPreset};
pub use media_builder::MediaConfigBuilder;

// Re-export all types from types.rs
pub use types::{
    ClientStats,
    CallMediaInfo,
    AudioCodecInfo,
    AudioQualityMetrics,
    MediaCapabilities,
    CallCapabilities,
    MediaSessionInfo,
    NegotiatedMediaParams,
    EnhancedMediaCapabilities,
    AudioDirection,
};

// Re-export event types from events.rs
pub use events::{
    ClientCallHandler,
};

// Re-export builder module
pub use builder::ClientBuilder;

// Re-export recovery utilities
pub use recovery::{
    RetryConfig,
    RecoveryAction,
    RecoveryStrategies,
    ErrorContext,
    retry_with_backoff,
    with_timeout,
};

// Type alias for convenient use
pub type Client = ClientManager;

// Note: Individual operation methods are implemented as impl blocks in separate files
// and will be automatically available on ClientManager instances