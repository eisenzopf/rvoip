//! # rvoip-amazon-connect
//!
//! Amazon Connect **interop adapter** for [`rvoip_core::ConnectionAdapter`].
//!
//! Delivers a call arriving over any rvoip transport (SIP, WebRTC, UCTP/QUIC)
//! to a live Amazon Connect agent, carrying SIP-header-derived data as Connect
//! **contact attributes** — the channel that drives an agent **screen pop**.
//!
//! The integration has two planes:
//!
//! 1. **Control plane** — [`control`] calls the Amazon Connect
//!    `StartWebRTCContact` API (passing the translated attributes) and returns
//!    the Amazon Chime SDK meeting + attendee `ConnectionData`.
//! 2. **Media plane** — [`signaling`] joins that Chime meeting over the
//!    proprietary protobuf-over-secure-WebSocket protocol and drives a
//!    `webrtc-rs` peer connection (reusing `rvoip-webrtc`'s peer/media plane)
//!    so the audio can be bridged to the inbound leg by the orchestrator.
//!
//! See `crates/webrtc/rvoip-amazon-connect/README.md` for the end-to-end flow.

#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

pub mod adapter;
pub mod config;
pub mod control;
pub mod errors;
pub mod mapping;
pub mod signaling;

#[cfg(feature = "server")]
pub mod bridge;
#[cfg(feature = "server")]
pub mod server;

pub use adapter::{
    AmazonConnectAdapter, ConnectMetrics, ContactSetupObserver, ContactSetupStage, ContactTarget,
    ADAPTER_EVENT_CAP,
};
pub use config::ConnectConfig;
pub use control::{
    ConnectContactStarter, ConnectionData, MediaPlacement, StartContactRequest, StopContactRequest,
};
pub use errors::{ConnectError, Result};
pub use mapping::{AttributeMapping, MappedAttributes, UnmappedPolicy, MAX_ATTRIBUTE_BYTES};

#[cfg(feature = "aws-control")]
pub use control::AwsConnectStarter;

#[cfg(feature = "server")]
pub use server::{
    request_uri_user, to_uri_user, uri_user_part, ConnectScreenPopServer, ContactRoute,
    ContactRouter, RouteDecision, RouteMetrics, ScreenPopLifecycleEvent, ScreenPopLifecycleStage,
    ScreenPopMediaLeg, ScreenPopServerConfig,
};

/// Re-export of the SIP UAS config (`rvoip_sip::Config`) so callers can build a
/// [`ScreenPopServerConfig`] without depending on `rvoip-sip` directly. Build
/// one with `SipConfig::local(name, port)`.
#[cfg(feature = "server")]
pub use rvoip_sip::Config as SipConfig;

/// Re-export of the inbound-INVITE wrapper so a [`server::ContactRouter`]
/// closure can be written without a direct `rvoip-sip` dependency.
#[cfg(feature = "server")]
pub use rvoip_sip::IncomingCall;
