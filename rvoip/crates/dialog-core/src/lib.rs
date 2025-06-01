//! # RVOIP Dialog-Core
//!
//! RFC 3261 SIP Dialog Management Layer for RVOIP.
//!
//! This crate implements the SIP dialog layer as defined in RFC 3261, providing
//! clean separation between session coordination (handled by `session-core`) and
//! SIP protocol operations.
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
//! ## Quick Start
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::{DialogClient, DialogServer};
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create a SIP server using the clean API
//!     let server = DialogServer::new("0.0.0.0:5060").await?;
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

// Core modules
pub mod errors;
pub mod dialog;
pub mod manager;
pub mod protocol;
pub mod routing;
pub mod sdp;
pub mod recovery;
pub mod events;

// **NEW**: Clean API layer for easy consumption
pub mod api;

// Re-export main types
pub use manager::DialogManager;
pub use dialog::{DialogId, Dialog, DialogState};
pub use errors::{DialogError, DialogResult};
pub use events::{SessionCoordinationEvent, DialogEvent};

// **NEW**: Re-export clean API types
pub use api::{DialogClient, DialogServer, DialogConfig};

// Re-export for convenience
pub use rvoip_sip_core::{Request, Response, Method, StatusCode, Uri};
pub use rvoip_transaction_core::TransactionKey; 