#![warn(rust_2018_idioms)]
#![allow(dead_code)]

//! Tokio-based WebRTC implementation in Rust
//!
//! This crate provides an async-friendly Tokio integration built on top of the
//! runtime-independent Sans-I/O [rtc](https://docs.rs/rtc) protocol core.
//!
//! # Async Runtime Support
//!
//! Tokio is always enabled. The `runtime-tokio` feature name remains as a
//! source-compatible no-op for existing manifests.

pub mod data_channel;
pub mod media_stream;
pub mod peer_connection;
pub mod rtp_transceiver;
pub mod runtime;

/// Error and Result types
///
/// Re-exports [`error::Error`] and [`error::Result`] from `rtc-shared` so that
/// callers only need to import from `webrtc::error` rather than reaching into
/// the lower-level crate directly.
pub mod error {
    pub use rtc::shared::error::{Error, Result};
}
