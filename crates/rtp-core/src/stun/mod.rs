//! STUN (Session Traversal Utilities for NAT) implementation per RFC 5389.
//!
//! Provides a STUN client for NAT binding discovery, enabling VoIP endpoints
//! to determine their public (server-reflexive) transport address. This is
//! essential for SDP negotiation when peers are behind NAT.
//!
//! # Modules
//!
//! - [`adapter`]: **Recommended.** Production adapter backed by the `stun-rs`
//!   crate (comprehensive RFC 5389 / RFC 8489 support).
//! - [`message`]: *(Deprecated)* Legacy hand-rolled STUN message encode/decode.
//! - [`client`]: *(Deprecated)* Legacy STUN Binding Request client.
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

pub mod adapter;

#[deprecated(since = "0.1.27", note = "use stun::adapter (backed by stun-rs) instead")]
pub mod message;
#[deprecated(since = "0.1.27", note = "use stun::adapter::StunClientAdapter instead")]
pub mod client;
pub mod discovery;

// Re-export the new adapter as the primary API.
pub use adapter::StunClientAdapter;

// Re-export legacy types for backward compatibility (marked deprecated at
// their definition sites or via the module-level deprecation above).
#[allow(deprecated)]
pub use client::{StunClient, StunClientConfig, StunBindingResult};
pub use discovery::{NatType, NatInfo, discover_nat_type, get_public_address, DEFAULT_STUN_SERVERS};
#[allow(deprecated)]
pub use message::{
    StunMessage, TransactionId, StunAttribute, MAGIC_COOKIE, HEADER_SIZE, ATTR_HEADER_SIZE,
    BINDING_REQUEST, BINDING_RESPONSE, BINDING_ERROR_RESPONSE,
    ATTR_USERNAME, ATTR_MESSAGE_INTEGRITY, ATTR_PRIORITY, ATTR_USE_CANDIDATE,
    ATTR_ICE_CONTROLLING, ATTR_ICE_CONTROLLED, ATTR_FINGERPRINT,
    ATTR_REALM, ATTR_NONCE, ATTR_ERROR_CODE,
    ADDR_FAMILY_IPV4, ADDR_FAMILY_IPV6,
    decode_address, encode_xor_address,
};
