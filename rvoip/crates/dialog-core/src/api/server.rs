//! Dialog Server API
//!
//! This module provides a high-level server interface for SIP dialog management,
//! abstracting the complexity of the underlying DialogManager for server use cases.
//!
//! The implementation has been modularized into focused submodules for better
//! maintainability and separation of concerns:
//!
//! - **Core**: Server struct, constructors, and configuration 
//! - **Call Operations**: Call lifecycle management (handle, accept, reject, terminate)
//! - **Dialog Operations**: Dialog management operations (create, query, list, terminate)
//! - **Response Builder**: Response building and sending functionality
//! - **SIP Methods**: Specialized SIP method handlers (BYE, REFER, NOTIFY, etc.)
//!
//! ## Example Usage
//! 
//! ```rust,no_run
//! use rvoip_dialog_core::api::{DialogServer, DialogApi};
//! use tokio::sync::mpsc;
//! 
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create server with simple configuration
//!     let server = DialogServer::new("0.0.0.0:5060").await?;
//!     
//!     // Set up session coordination
//!     let (session_tx, session_rx) = mpsc::channel(100);
//!     server.set_session_coordinator(session_tx).await?;
//!     
//!     // Start processing SIP messages
//!     server.start().await?;
//!     
//!     Ok(())
//! }
//! ```

// Re-export everything from the modular implementation
pub use self::server::{DialogServer, ServerStats};

// Include the modular implementation
mod server;
