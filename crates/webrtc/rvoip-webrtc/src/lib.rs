//! # rvoip-webrtc
//!
//! WebRTC **interop adapter** for [`rvoip_core::ConnectionAdapter`], built on
//! webrtc-rs **0.20.0-alpha.1**.
//!
//! See [`docs/archived/IMPLEMENTATION_PLAN.md`](docs/archived/IMPLEMENTATION_PLAN.md) for architecture
//! and [`docs/archived/HARDENING_PLAN.md`](docs/archived/HARDENING_PLAN.md) for the path to production.

// Phase H1: deny ad-hoc panic paths in the library. Tests / examples may still
// use unwrap/expect via `#[cfg(test)]` and the unwrap_used lint exemption.
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

pub mod adapter;
pub mod config;
pub mod errors;
pub mod identity;
pub mod media;
pub mod observability;
pub mod peer;
pub mod sdp;
#[cfg(feature = "tls-rustls")]
pub mod tls;
pub mod turn_rest;

#[cfg(any(feature = "signaling-whip", feature = "signaling-ws"))]
pub mod signaling;

#[cfg(any(feature = "signaling-whip", feature = "signaling-ws"))]
pub mod server;

#[cfg(feature = "client")]
pub mod client;

pub use adapter::{WebRtcAdapter, WebRtcMetrics, WebRtcTransportHandle, ADAPTER_EVENT_CAP};
pub use config::{IceServerConfig, OpusSettings, WebRtcConfig};
pub use errors::{Result, WebRtcError};
pub use media::WebRtcStatsSnapshot;
pub use peer::{
    connect_loopback, DataChannelOptions, IceCandidateLog, PeerRole, RvoipDataChannel,
    RvoipPeerConnection, WebRtcFeatureSupport,
};
pub use sdp::{sdp_has_inline_ice_candidates, sdp_has_media_line, sdp_indicates_simulcast};
pub use webrtc::peer_connection::{RTCIceCandidate, RTCIceCandidateInit};

#[cfg(any(feature = "signaling-whip", feature = "signaling-ws"))]
pub use server::{WebRtcServer, WebRtcServerBuilder};

#[cfg(feature = "client")]
pub use client::{
    run_audio, Answer, AudioPacing, AudioSink, AudioSource, CallTarget, ComprehensiveReport,
    CountingAudioSink, FixtureAudioSource, IceCandidate, NegotiationAction, NullAudioSink, Offer,
    PerfectNegotiation, SessionHandle, SessionMedium, Signaler, SignalingPool, WebRtcClient,
};
#[cfg(all(feature = "client", feature = "signaling-ws"))]
pub use client::{WsSignaler, WsSignalerConfig};
