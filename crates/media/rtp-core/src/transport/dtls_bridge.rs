//! Bridges DTLS-SRTP handshake bytes arriving on a shared, unconnected RTP
//! UDP socket into [`webrtc_util::conn::Conn`], the transport abstraction
//! `dtls_ext`'s `DTLSConn` needs.
//!
//! [`UdpRtpTransport`](super::udp::UdpRtpTransport)'s receive loop already
//! classifies incoming datagrams (RFC 7983) and, for anything classified as
//! DTLS, forwards the raw bytes into the channel this adapter reads from
//! instead of the (unrelated) RTP/RTCP handling path. One shared socket
//! serves RTP, RTCP, and DTLS, demultiplexed by the first byte of each
//! datagram — this adapter never touches the raw socket for reads itself,
//! only for writes (`send_to`).
//!
//! The separate RTCP socket of a call that does not multiplex RTCP carries
//! its own handshake (RFC 5764 §4.1) through a second adapter of the same
//! kind; its receive loop may retarget that adapter at the source a NAT'd
//! peer's handshake actually arrives from.

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, Mutex as AsyncMutex};

use webrtc_util::conn::Conn;

/// Feeds demuxed DTLS datagrams into a `Conn` implementation so a DTLS
/// handshake can run over a port that's also carrying RTP/RTCP for the
/// same call, without needing its own dedicated socket.
pub struct DtlsUdpConnAdapter {
    socket: Arc<UdpSocket>,
    remote: parking_lot::Mutex<SocketAddr>,
    inbound: AsyncMutex<mpsc::Receiver<DtlsDatagram>>,
}

/// One demultiplexed DTLS datagram, as the transport's receive loop hands it
/// to an adapter.
pub(crate) struct DtlsDatagram {
    pub(crate) bytes: Bytes,
    /// Where handshake replies go from now on, when the receive loop has
    /// accepted a new source for this peer (a NAT-mapped RTCP port). `None`
    /// keeps the current destination.
    pub(crate) retarget: Option<SocketAddr>,
}

impl DtlsDatagram {
    pub(crate) fn new(bytes: Bytes) -> Self {
        Self {
            bytes,
            retarget: None,
        }
    }
}

impl DtlsUdpConnAdapter {
    pub(crate) fn new(
        socket: Arc<UdpSocket>,
        remote: SocketAddr,
        inbound: mpsc::Receiver<DtlsDatagram>,
    ) -> Self {
        Self {
            socket,
            remote: parking_lot::Mutex::new(remote),
            inbound: AsyncMutex::new(inbound),
        }
    }

    fn current_remote(&self) -> SocketAddr {
        *self.remote.lock()
    }
}

#[async_trait]
impl Conn for DtlsUdpConnAdapter {
    async fn connect(&self, _addr: SocketAddr) -> webrtc_util::Result<()> {
        // This adapter is already scoped to exactly one remote peer
        // (`self.remote`, set when the transport learned the call's
        // remote RTP or RTCP address); there's no separate connect step.
        Ok(())
    }

    async fn recv(&self, buf: &mut [u8]) -> webrtc_util::Result<usize> {
        let (n, _addr) = self.recv_from(buf).await?;
        Ok(n)
    }

    async fn recv_from(&self, buf: &mut [u8]) -> webrtc_util::Result<(usize, SocketAddr)> {
        let mut inbound = self.inbound.lock().await;
        match inbound.recv().await {
            Some(datagram) => {
                if let Some(target) = datagram.retarget {
                    *self.remote.lock() = target;
                }
                // Matches plain UDP-socket semantics: a datagram larger
                // than the caller's buffer is truncated, not an error.
                let n = datagram.bytes.len().min(buf.len());
                buf[..n].copy_from_slice(&datagram.bytes[..n]);
                Ok((n, self.current_remote()))
            }
            None => Err(std::io::Error::other(
                "DTLS demux channel closed (RTP transport receiver stopped)",
            )
            .into()),
        }
    }

    async fn send(&self, buf: &[u8]) -> webrtc_util::Result<usize> {
        self.send_to(buf, self.current_remote()).await
    }

    async fn send_to(&self, buf: &[u8], target: SocketAddr) -> webrtc_util::Result<usize> {
        Ok(self.socket.send_to(buf, target).await?)
    }

    fn local_addr(&self) -> webrtc_util::Result<SocketAddr> {
        Ok(self.socket.local_addr()?)
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        Some(self.current_remote())
    }

    async fn close(&self) -> webrtc_util::Result<()> {
        Ok(())
    }

    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }
}
