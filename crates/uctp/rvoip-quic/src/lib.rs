//! # rvoip-quic
//!
//! `rvoip_core::ConnectionAdapter` implementation over raw QUIC, speaking
//! the UCTP application protocol. ALPN = `uctp/1`.
//!
//! See `crates/uctp/rvoip-uctp/UCTP_IMPLEMENTATION_PLAN.md` §4 for the design.

pub mod adapter;
pub mod broadcast;
pub mod client;
pub mod errors;
pub mod media_stream;
pub mod server;

pub use adapter::{UctpQuicAdapter, UctpQuicConfig, ADAPTER_EVENT_CAP};
pub use broadcast::{UctpBroadcastPublisher, UCTP_QUIC_PROTOCOL_VERSION};
pub use client::UctpQuicClient;
pub use errors::{Result, UctpQuicError};
pub use media_stream::{
    spawn_datagram_reader, spawn_datagram_reader_with_cancel, QuicDatagramMediaStream,
};
pub use rvoip_core::adapter::{AdapterEvent, ConnectionAdapter};
pub use server::UctpQuicServer;
