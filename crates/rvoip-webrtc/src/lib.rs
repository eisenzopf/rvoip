//! # rvoip-webrtc
//!
//! WebRTC **interop adapter** for [`rvoip_core::ConnectionAdapter`], built on
//! webrtc-rs **0.20.0-alpha.1**.
//!
//! See [`docs/IMPLEMENTATION_PLAN.md`](docs/IMPLEMENTATION_PLAN.md) for architecture.

pub mod adapter;
pub mod config;
pub mod errors;
pub mod media;
pub mod peer;
pub mod sdp;

#[cfg(any(feature = "signaling-whip", feature = "signaling-ws"))]
pub mod signaling;

#[cfg(any(feature = "signaling-whip", feature = "signaling-ws"))]
pub mod server;

#[cfg(feature = "client")]
pub mod client;

pub use adapter::{WebRtcAdapter, ADAPTER_EVENT_CAP};
pub use config::WebRtcConfig;
pub use errors::{Result, WebRtcError};
pub use peer::{connect_loopback, PeerRole, RvoipPeerConnection};

#[cfg(any(feature = "signaling-whip", feature = "signaling-ws"))]
pub use server::{WebRtcServer, WebRtcServerBuilder};

#[cfg(feature = "client")]
pub use client::{Answer, CallTarget, IceCandidate, Offer, SessionHandle, SessionMedium, Signaler, WebRtcClient};
