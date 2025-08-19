//! Media-specific RTP processing (moved from rtp-core)
//!
//! This module contains media-layer RTP functionality moved from rtp-core
//! as part of the Transport/Media plane separation. These components handle
//! media-specific operations like mixing, CSRC management, and header extensions.

pub mod mixing;
pub mod csrc;
pub mod extensions;

pub use mixing::*;
pub use csrc::*;
pub use extensions::*;