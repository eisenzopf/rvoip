//! RTCP feedback generation for media quality (moved from rtp-core)
//!
//! This module contains RTCP feedback mechanisms moved from rtp-core
//! as part of the Transport/Media plane separation. These components handle
//! media quality feedback including PLI, FIR, REMB, and other quality-related feedback.

pub mod feedback;
pub mod generators;
pub mod packets;
pub mod algorithms;

pub use feedback::*;
pub use generators::*;
pub use packets::*;
pub use algorithms::*;