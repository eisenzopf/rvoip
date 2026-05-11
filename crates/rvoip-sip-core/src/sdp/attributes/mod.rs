//! SDP Attribute Parsers
//!
//! This module contains parsers for SDP attributes as defined in RFC 8866
//! and related RFCs.

// Media attribute modules
pub mod direction;
pub mod fmtp;
pub mod ptime;
pub mod rtpmap;

// Transport modules
pub mod candidate;
pub mod dtls;
pub mod ice;

// Identification modules
pub mod mid;
pub mod msid;
pub mod ssrc;

// Grouping and stream management
pub mod group;
pub mod rid;
pub mod simulcast;

// RTCP-related
pub mod rtcp;

// Extension modules
pub mod bandwidth;
pub mod extmap;

// Data channel
pub mod datachannel;
pub mod sctp;

// SVC-related
pub mod scalability;
pub mod sctpmap;

// Common utilities
pub mod common;

// Publicly expose all attribute parsers
pub use bandwidth::*;
pub use candidate::*;
pub use common::*;
pub use datachannel::*;
pub use direction::*;
pub use dtls::*;
pub use extmap::*;
pub use fmtp::*;
pub use group::*;
pub use ice::*;
pub use mid::*;
pub use msid::*;
pub use ptime::*;
pub use rid::*;
pub use rtcp::*;
pub use rtpmap::*;
pub use scalability::*;
pub use sctp::*;
pub use sctpmap::*;
pub use simulcast::*;
pub use ssrc::*;

// Re-export MediaDirection enum for backward compatibility
pub use crate::sdp::attributes::direction::MediaDirection;
