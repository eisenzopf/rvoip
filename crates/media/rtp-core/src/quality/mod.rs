//! Protocol-level voice-quality primitives.
//!
//! This module deliberately contains only RTP/RTCP concepts that can be used
//! by any application. Session correlation, media-pipeline discards,
//! AudioSocket semantics, telemetry, and report publication belong to the
//! application integrating `rvoip-rtp-core`.

pub mod e_model;
pub mod monitor;
