//! Codec negotiation and SDP media handling (moved from rtp-core)
//!
//! This module contains codec negotiation, payload type registry,
//! and SDP media handling functionality moved from rtp-core as part
//! of the Transport/Media plane separation.

pub mod negotiation;
pub mod registry;
pub mod sdp;

pub use negotiation::*;
pub use registry::*;
pub use sdp::*;
