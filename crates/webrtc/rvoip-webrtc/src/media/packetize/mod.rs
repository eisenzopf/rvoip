//! D3 — RTP packetizers for video codecs.
//!
//! Each codec has its own RFC-specified payload format; the modules here
//! own the wire-level details so `VideoSource`
//! implementations don't have to.

pub mod h264;
pub mod vp8;

pub use h264::{packetize_h264, H264Packet};
pub use vp8::{packetize_vp8, payload_is_start_of_partition, Vp8Packet, DEFAULT_MTU};
