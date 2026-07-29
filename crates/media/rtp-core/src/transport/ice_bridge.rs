//! Bridges ICE (RFC 8445) connectivity checks onto
//! [`UdpRtpTransport`](super::udp::UdpRtpTransport)'s shared RTP/RTCP/STUN
//! socket, so an `IceAgent` (`rvoip-nat-core`) doesn't need a dedicated
//! port of its own.
//!
//! The transport's receive loop already classifies incoming datagrams
//! (RFC 7983) and, for anything classified as STUN, forwards the raw
//! bytes and source address into the channel a caller subscribes to via
//! [`UdpRtpTransport::subscribe_stun_datagrams`]. This adapter only
//! implements the outbound half (`SharedIceSocket::send_to`); routing
//! inbound bytes to the right per-ufrag ICE connection is
//! `rvoip_nat_core::bridge::SharedIceMux`'s job, driven by the caller
//! pumping the STUN subscription into `IceAgent::handle_incoming_stun` —
//! mirrored exactly in `tests/ice_transport_bridge_test.rs`.

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::net::UdpSocket;

use rvoip_nat_core::SharedIceSocket;

/// Adapts a shared RTP UDP socket to `rvoip-nat-core`'s outbound-only
/// [`SharedIceSocket`] trait.
pub struct IceUdpSocketAdapter {
    socket: Arc<UdpSocket>,
}

impl IceUdpSocketAdapter {
    pub(crate) fn new(socket: Arc<UdpSocket>) -> Self {
        Self { socket }
    }
}

#[async_trait]
impl SharedIceSocket for IceUdpSocketAdapter {
    async fn send_to(&self, buf: &[u8], target: SocketAddr) -> std::io::Result<usize> {
        self.socket.send_to(buf, target).await
    }

    fn local_addr(&self) -> SocketAddr {
        // The RTP socket is always bound to a concrete local address by
        // the time a caller can obtain this adapter (see
        // `UdpRtpTransport::ice_conn_adapter`), so this can't fail in
        // practice; falling back to the unspecified address rather than
        // panicking keeps this infallible for a trait that has no
        // `Result` in its signature.
        self.socket
            .local_addr()
            .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)))
    }
}
