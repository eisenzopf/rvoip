//! # RVOIP Dialog-Core
//!
//! RFC 3261 SIP Dialog Management Layer for RVOIP.
//!
//! This crate implements the SIP dialog layer as defined in RFC 3261, providing
//! clean separation between session coordination (handled by `session-core`) and
//! SIP protocol operations. It manages dialog state, handles in-dialog requests,
//! and coordinates with transaction-core for reliable SIP message delivery.
//!
//! ## Architecture Position
//!
//! ```text
//! session-core (Session Coordination)
//!      ↓
//! dialog-core (SIP Protocol)  ← THIS CRATE
//!      ↓  
//! transaction-core (Reliability)
//!      ↓
//! sip-transport (Network)
//! ```
//!
//! ## What This Crate Does
//!
//! - **Dialog State Management**: Tracks dialog lifecycle (Initial → Early → Confirmed → Terminated)
//! - **CSeq Management**: Handles sequence number generation and validation
//! - **Route Set Management**: Maintains dialog route sets from Record-Route headers
//! - **In-Dialog Requests**: Creates properly formatted requests within established dialogs
//! - **Recovery Handling**: Manages dialog recovery from network failures
//! - **Session Integration**: Coordinates with session-core for high-level call management
//!
//! ## Quick Start - Server
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::{DialogServer, DialogApi};
//! use rvoip_transaction_core::{TransactionManager};
//! use rvoip_transaction_core::transport::{TransportManager, TransportManagerConfig};
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Set up transport layer
//!     let config = TransportManagerConfig {
//!         enable_udp: true,
//!         bind_addresses: vec!["0.0.0.0:5060".parse()?],
//!         ..Default::default()
//!     };
//!     
//!     let (transport, transport_rx) = TransportManager::new(config).await?;
//!     let (transaction_manager, global_rx) = TransactionManager::with_transport_manager(
//!         transport, transport_rx, Some(100)
//!     ).await?;
//!     
//!     // Create server using recommended global events pattern
//!     let server_config = rvoip_dialog_core::api::config::ServerConfig::default();
//!     let server = DialogServer::with_global_events(
//!         Arc::new(transaction_manager),
//!         global_rx,
//!         server_config
//!     ).await?;
//!     
//!     // Set up session coordination (connects to session-core)
//!     let (session_tx, _session_rx) = tokio::sync::mpsc::channel(100);
//!     server.set_session_coordinator(session_tx).await?;
//!     
//!     // Start processing
//!     server.start().await?;
//!     
//!     Ok(())
//! }
//! ```
//!
//! ## Quick Start - Client  
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::{DialogClient, DialogApi};
//! use rvoip_transaction_core::{TransactionManager};
//! use rvoip_transaction_core::transport::{TransportManager, TransportManagerConfig};
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Set up transport and transaction layers
//!     let config = TransportManagerConfig::default();
//!     let (transport, transport_rx) = TransportManager::new(config).await?;
//!     let (transaction_manager, global_rx) = TransactionManager::with_transport_manager(
//!         transport, transport_rx, Some(100)
//!     ).await?;
//!     
//!     // Create client using recommended global events pattern
//!     let client_config = rvoip_dialog_core::api::config::ClientConfig::default();
//!     let client = DialogClient::with_global_events(
//!         Arc::new(transaction_manager),
//!         global_rx,
//!         client_config
//!     ).await?;
//!     
//!     // Start the client
//!     client.start().await?;
//!     
//!     // Create an outgoing dialog and make a call
//!     let local_uri = "sip:alice@example.com";
//!     let remote_uri = "sip:bob@example.com";
//!     let dialog = client.create_dialog(local_uri, remote_uri).await?;
//!     let dialog_id = dialog.id().clone();
//!     
//!     // Send in-dialog requests using Phase 3 one-liner functions
//!     let _info_tx = client.send_info(&dialog_id, "Application data".to_string()).await?;
//!     let _bye_tx = client.send_bye(&dialog_id).await?;
//!     
//!     Ok(())
//! }
//! ```
//!
//! ## Key Features
//!
//! - **Phase 3 Integration**: Uses transaction-core helper functions for simplified SIP operations
//! - **Global Events Pattern**: Recommended architecture for event-driven operation
//! - **RFC 3261 Compliance**: Full compliance with SIP dialog specifications
//! - **Recovery Support**: Built-in dialog recovery from network failures
//! - **Clean API**: High-level `DialogClient` and `DialogServer` for easy integration

// Core modules
pub mod errors;
pub mod dialog;
pub mod manager;
pub mod protocol;
pub mod routing;
pub mod sdp;
pub mod recovery;
pub mod events;

// **NEW**: Configuration system (unified and legacy)
pub mod config;

// **NEW**: Clean API layer for easy consumption
pub mod api;

// Re-export main types
pub use manager::{DialogManager, UnifiedDialogManager};
pub use dialog::{DialogId, Dialog, DialogState};
pub use errors::{DialogError, DialogResult};
pub use events::{SessionCoordinationEvent, DialogEvent};

// **NEW**: Re-export unified configuration types
pub use config::{
    DialogManagerConfig, 
    ClientBehavior, 
    ServerBehavior, 
    HybridBehavior,
};

// **NEW**: Re-export clean API types
pub use api::{DialogClient, DialogServer, UnifiedDialogApi};
pub use api::config::{ClientConfig, ServerConfig};
pub use api::{ApiResult, ApiError, DialogStats};

// Re-export for convenience
pub use rvoip_sip_core::{Request, Response, Method, StatusCode, Uri};
pub use rvoip_transaction_core::TransactionKey; 