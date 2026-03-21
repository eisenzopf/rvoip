//! STUN (Session Traversal Utilities for NAT) implementation per RFC 5389.
//!
//! Provides a STUN client for NAT binding discovery, enabling VoIP endpoints
//! to determine their public (server-reflexive) transport address. This is
//! essential for SDP negotiation when peers are behind NAT.
//!
//! # Modules
//!
//! - [`message`]: Low-level STUN message encoding and decoding.
//! - [`client`]: STUN Binding Request client with retransmission.
//! - [`discovery`]: Higher-level NAT type discovery and convenience functions.
//!
//! # Quick Start
//!
//! ```rust,no_run
//! use rvoip_rtp_core::stun;
//!
//! # async fn example() -> Result<(), rvoip_rtp_core::Error> {
//! // Get your public address for SDP
//! let local_addr = "0.0.0.0:0".parse().unwrap();
//! let public_addr = stun::discovery::get_public_address(local_addr).await?;
//! println!("My public address: {public_addr}");
//!
//! // Or do full NAT discovery
//! let nat_info = stun::discovery::discover_nat_type(&[]).await?;
//! println!("NAT type: {}", nat_info.nat_type);
//! # Ok(())
//! # }
//! ```

pub mod message;
pub mod client;
pub mod discovery;

// Re-export key types at module level for convenience.
pub use client::{StunClient, StunClientConfig, StunBindingResult};
pub use discovery::{NatType, NatInfo, discover_nat_type, get_public_address, DEFAULT_STUN_SERVERS};
pub use message::{
    StunMessage, TransactionId, StunAttribute, MAGIC_COOKIE, HEADER_SIZE, ATTR_HEADER_SIZE,
    BINDING_REQUEST, BINDING_RESPONSE, BINDING_ERROR_RESPONSE,
    ATTR_USERNAME, ATTR_MESSAGE_INTEGRITY, ATTR_PRIORITY, ATTR_USE_CANDIDATE,
    ATTR_ICE_CONTROLLING, ATTR_ICE_CONTROLLED, ATTR_FINGERPRINT,
    ATTR_REALM, ATTR_NONCE, ATTR_ERROR_CODE,
    ADDR_FAMILY_IPV4, ADDR_FAMILY_IPV6,
    decode_address, encode_xor_address,
};
