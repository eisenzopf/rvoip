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
    remote: SocketAddr,
    inbound: AsyncMutex<mpsc::Receiver<Bytes>>,
}

impl DtlsUdpConnAdapter {
    pub(crate) fn new(
        socket: Arc<UdpSocket>,
        remote: SocketAddr,
        inbound: mpsc::Receiver<Bytes>,
    ) -> Self {
        Self {
            socket,
            remote,
            inbound: AsyncMutex::new(inbound),
        }
    }
}

#[async_trait]
impl Conn for DtlsUdpConnAdapter {
    async fn connect(&self, _addr: SocketAddr) -> webrtc_util::Result<()> {
        // This adapter is already scoped to exactly one remote peer
        // (`self.remote`, set when the transport learned the call's
        // remote RTP address); there's no separate connect step.
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
                // Matches plain UDP-socket semantics: a datagram larger
                // than the caller's buffer is truncated, not an error.
                let n = datagram.len().min(buf.len());
                buf[..n].copy_from_slice(&datagram[..n]);
                Ok((n, self.remote))
            }
            None => Err(std::io::Error::other(
                "DTLS demux channel closed (RTP transport receiver stopped)",
            )
            .into()),
        }
    }

    async fn send(&self, buf: &[u8]) -> webrtc_util::Result<usize> {
        self.send_to(buf, self.remote).await
    }

    async fn send_to(&self, buf: &[u8], target: SocketAddr) -> webrtc_util::Result<usize> {
        Ok(self.socket.send_to(buf, target).await?)
    }

    fn local_addr(&self) -> webrtc_util::Result<SocketAddr> {
        Ok(self.socket.local_addr()?)
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        Some(self.remote)
    }

    async fn close(&self) -> webrtc_util::Result<()> {
        Ok(())
    }

    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }
}
