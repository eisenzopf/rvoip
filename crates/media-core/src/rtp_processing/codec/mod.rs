//! Codec negotiation and SDP media handling (moved from rtp-core)
//!
//! This module contains codec negotiation, payload type registry,
//! and SDP media handling functionality moved from rtp-core as part
//! of the Transport/Media plane separation.

pub mod registry;
pub mod negotiation;
pub mod sdp;

pub use registry::*;
pub use negotiation::*;
pub use sdp::*;