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
//! use rvoip_dialog_core::{DialogManager, DialogError};
//! use rvoip_transaction_core::TransactionManager;
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), DialogError> {
//!     // Create transaction manager (handles transport for us)
//!     let transaction_manager = Arc::new(TransactionManager::new().await?);
//!     
//!     // Create dialog manager
//!     let dialog_manager = DialogManager::new(transaction_manager).await?;
//!     
//!     // Set up session coordination (connects to session-core)
//!     let (session_tx, _session_rx) = tokio::sync::mpsc::channel(100);
//!     dialog_manager.set_session_coordinator(session_tx);
//!     
//!     // Start processing
//!     dialog_manager.start().await?;
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

// Re-export main types
pub use manager::DialogManager;
pub use dialog::{DialogId, Dialog, DialogState};
pub use errors::{DialogError, DialogResult};
pub use events::{SessionCoordinationEvent, DialogEvent};

// Re-export for convenience
pub use rvoip_sip_core::{Request, Response, Method, StatusCode, Uri};
pub use rvoip_transaction_core::TransactionKey; 