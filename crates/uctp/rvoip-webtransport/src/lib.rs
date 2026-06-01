//! # rvoip-webtransport
//!
//! `rvoip_core::ConnectionAdapter` implementation over WebTransport,
//! speaking the UCTP application protocol. ALPN = `h3`, mount path
//! defaults to `/uctp`.
//!
//! See `crates/uctp/rvoip-uctp/UCTP_IMPLEMENTATION_PLAN.md` §5.

pub mod adapter;
pub mod client;
pub mod errors;
pub mod media_stream;
pub mod server;

pub use adapter::{UctpWtAdapter, UctpWtConfig, ADAPTER_EVENT_CAP};
pub use client::UctpWtClient;
pub use errors::{Result, UctpWtError};
pub use media_stream::{spawn_datagram_reader, FanoutContext, WebTransportDatagramMediaStream};
pub use server::UctpWtServer;
