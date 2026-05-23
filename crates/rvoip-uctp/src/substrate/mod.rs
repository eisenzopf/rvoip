//! Substrate-agnostic helpers consumed by adapter crates: quinn endpoint
//! setup, length-prefixed framing, TLS config, datagram pack/unpack,
//! envelope-id correlation.
//!
//! See `UCTP_IMPLEMENTATION_PLAN.md` §3.7 for the design.

pub mod correlation;
pub mod datagram;
pub mod framing;
pub mod quinn;
pub mod tls;

pub use correlation::Pending;
pub use datagram::{pack, unpack, MediaDatagram};
pub use framing::{envelope_reader, envelope_writer, length_prefixed_codec};
pub use quinn::{
    dispatch_by_alpn, make_client_endpoint, make_server_endpoint, spawn_stats_sampler, AlpnRoutes,
    ALPN_ACCEPT_CAP, DEFAULT_QUINN_STATS_INTERVAL,
};
pub use tls::{dev_client_config_trusting, self_signed_for_dev};

#[cfg(feature = "dev-dangerous")]
pub use tls::dangerous_no_verify;
