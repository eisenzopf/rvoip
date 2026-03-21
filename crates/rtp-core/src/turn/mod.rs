//! TURN (Traversal Using Relays around NAT) client implementation per RFC 5766.
//!
//! Provides a TURN client for relay-based NAT traversal. When direct peer-to-peer
//! connectivity fails (e.g., symmetric NAT), TURN allocates a relay address on the
//! server that both peers can use to exchange media.
//!
//! # Modules
//!
//! - [`message`]: TURN-specific message types and attribute encoding/decoding.
//! - [`client`]: Async TURN client with allocation, permission, and channel binding.
//! - [`credentials`]: Long-term credential mechanism (HMAC-SHA1 over MD5 key).
//!
//! # Quick Start
//!
//! ```rust,no_run
//! use rvoip_rtp_core::turn::client::TurnClient;
//! use std::net::SocketAddr;
//!
//! # async fn example() -> Result<(), rvoip_rtp_core::Error> {
//! let server: SocketAddr = "turn.example.com:3478".parse()
//!     .map_err(|e| rvoip_rtp_core::Error::TurnError(format!("{e}")))?;
//! let mut client = TurnClient::new(server, "user".into(), "pass".into()).await?;
//! let alloc = client.allocate().await?;
//! println!("Relay address: {}", alloc.relayed_address);
//! # Ok(())
//! # }
//! ```

pub mod message;
pub mod client;
pub mod credentials;

pub use client::{TurnClient, TurnAllocation};
pub use credentials::LongTermCredentials;
pub use message::{
    TurnMessageType, TurnAttribute,
    ALLOCATE_REQUEST, ALLOCATE_RESPONSE, ALLOCATE_ERROR_RESPONSE,
    REFRESH_REQUEST, REFRESH_RESPONSE,
    CREATE_PERMISSION_REQUEST, CREATE_PERMISSION_RESPONSE,
    CHANNEL_BIND_REQUEST, CHANNEL_BIND_RESPONSE,
    SEND_INDICATION, DATA_INDICATION,
};

use std::net::SocketAddr;

/// Configuration for a TURN relay server used during ICE candidate gathering.
///
/// Contains the server address and long-term credentials required for
/// TURN allocation requests per RFC 5766.
#[derive(Debug, Clone)]
pub struct TurnServerConfig {
    /// TURN server address (UDP).
    pub server: SocketAddr,
    /// Username for long-term credential authentication.
    pub username: String,
    /// Password for long-term credential authentication.
    pub password: String,
}
