//! NAT-traversal primitives for the RTP path.
//!
//! Today this module hosts the RFC 8489 STUN client used to discover
//! the post-NAT mapping for the local RTP socket before SDP
//! generation. Future ICE (RFC 8445) and TURN (RFC 8656) clients land
//! alongside it under this same module tree.

pub mod stun;

pub use stun::{StunClient, StunError};
