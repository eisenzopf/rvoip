//! Substrate-agnostic helpers consumed by adapter crates: quinn endpoint
//! setup, length-prefixed framing, TLS config, datagram pack/unpack,
//! envelope-id correlation.
//!
//! See `UCTP_IMPLEMENTATION_PLAN.md` §3.7 for the design.

pub mod correlation;
pub mod datagram;
pub mod framing;
pub mod peer_media_router;
pub mod quinn;
pub mod tls;

pub use correlation::{send_and_wait, Pending};
pub use datagram::{pack_rtp_datagram, unpack_rtp_datagram, RtpDatagram, RtpMediaPayload};
// Alpha compatibility only: these helpers do not validate the opaque payload.
#[doc(hidden)]
pub use datagram::{pack, unpack, MediaDatagram};
pub use framing::{envelope_reader, envelope_writer, length_prefixed_codec};
pub use peer_media_router::{
    PeerMediaBinding, PeerMediaBindingSnapshot, PeerMediaConnectionKey, PeerMediaFanoutKey,
    PeerMediaRegistration, PeerMediaReservation, PeerMediaRouteKey, PeerMediaRouter,
    PeerMediaRouterError, PeerMediaRouterSnapshot,
};
pub use quinn::{
    dispatch_by_alpn, make_client_endpoint, make_server_endpoint, spawn_stats_sampler, AlpnRoutes,
    ALPN_ACCEPT_CAP, DEFAULT_QUINN_STATS_INTERVAL,
};
pub use tls::{
    dev_client_config_trusting, enable_client_key_log_from_env, enable_server_key_log_from_env,
    self_signed_for_dev,
};

#[cfg(feature = "dev-dangerous")]
pub use tls::dangerous_no_verify;
