//! Presence support for SIP SIMPLE (RFC 3903, RFC 3856)
//!
//! This module provides support for presence operations including:
//! - PUBLISH for publishing presence information
//! - SUBSCRIBE/NOTIFY handling (via subscription module)
//! - Presence document (PIDF) handling

pub mod publish;

pub use publish::{
    PublishBuilder,
    PublishResponse,
    PresencePublisher,
};