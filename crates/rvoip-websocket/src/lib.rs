//! # rvoip-websocket
//!
//! `rvoip_core::ConnectionAdapter` implementation over WebSocket (signaling)
//! plus a co-located `webrtc-rs` PeerConnection for media. Per
//! CONVERSATION_PROTOCOL.md §4.3 (WS = "fallback for older browsers and
//! constrained networks") and §10.2 (media tunnels through WebRTC
//! signaled via `connection.offer.substrate_setup`).
//!
//! Two distinct subsystems:
//!
//! 1. **Signaling**: UCTP envelopes carried as WebSocket **text frames**.
//!    Structurally identical to `rvoip-quic` but uses `tokio-tungstenite`.
//!    No ALPN involvement (WS uses HTTP Upgrade).
//!
//! 2. **Media**: per-Connection `webrtc::peer_connection::RTCPeerConnection`.
//!    SDP/ICE/DTLS exchange rides inside `connection.offer.substrate_setup`.
//!    `MediaFrame.payload` (already RTP-shaped) bridges to
//!    `TrackLocalStaticRTP::write_rtp` and back via `TrackRemote::read_rtp`.
//!
//! See `crates/rvoip-uctp/UCTP_IMPLEMENTATION_PLAN.md` for the design.

pub mod adapter;
pub mod client;
pub mod errors;
pub mod media_bridge;
pub mod server;

pub use adapter::{UctpWsAdapter, UctpWsConfig, ADAPTER_EVENT_CAP};
pub use client::UctpWsClient;
pub use errors::{Result, UctpWsError};
pub use server::UctpWsServer;
