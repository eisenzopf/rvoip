//! SDP Attribute Parsers
//! 
//! This module contains parsers for SDP attributes as defined in RFC 8866
//! and related RFCs.

// Media attribute modules
pub mod rtpmap;
pub mod fmtp;
pub mod ptime;
pub mod direction;

// Transport modules
pub mod candidate;
pub mod ice;
pub mod dtls;

// Identification modules
pub mod ssrc;
pub mod mid;
pub mod msid;

// Grouping and stream management
pub mod group;
pub mod simulcast;
pub mod rid;

// RTCP-related
pub mod rtcp;

// Extension modules
pub mod extmap;
pub mod bandwidth;

// Data channel
pub mod datachannel;
pub mod sctp;

// SVC-related 
pub mod scalability;
pub mod sctpmap;

// Common utilities
pub mod common;

// Publicly expose all attribute parsers
pub use rtpmap::*;
pub use fmtp::*;
pub use ptime::*;
pub use direction::*;
pub use candidate::*;
pub use ice::*;
pub use dtls::*;
pub use ssrc::*;
pub use mid::*;
pub use msid::*;
pub use group::*;
pub use simulcast::*;
pub use rid::*;
pub use rtcp::*;
pub use extmap::*;
pub use bandwidth::*;
pub use datachannel::*;
pub use sctp::*;
pub use scalability::*;
pub use sctpmap::*;
pub use common::*;

// Re-export MediaDirection enum for backward compatibility
pub use crate::sdp::attributes::direction::MediaDirection; 