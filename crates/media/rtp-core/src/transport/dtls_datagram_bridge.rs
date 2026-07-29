//! A generic, socket-free bridge from raw UDP datagrams to
//! [`webrtc_util::conn::Conn`], for embedding a DTLS-SRTP handshake
//! (`dtls_srtp::handshake_client`/`handshake_server`) into a caller-owned
//! reactor (e.g. a `mio` event loop) instead of this crate's own
//! `UdpRtpTransport`/`tokio::net::UdpSocket`.
//!
//! Unlike [`super::dtls_bridge::DtlsUdpConnAdapter`] (which shares a real
//! `tokio::net::UdpSocket` that `UdpRtpTransport` owns), this bridge owns
//! no socket at all: the caller pushes inbound datagrams in via
//! [`DtlsDatagramBridge::feed_inbound`] as its own reactor reads them off
//! the wire, and drains outbound datagrams back out of the channel handed
//! to [`DtlsDatagramBridge::new`] to write wherever its socket actually
//! lives. This crate keeps ownership of the handshake state machine and
//! the derived SRTP keys; the caller keeps ownership of the socket.
//!
//! Use [`super::classify_rtp_mux_packet`] to sort DTLS bytes out from
//! RTP/RTCP/STUN on a shared port the same way `UdpRtpTransport`'s own
//! receive loop does, before handing DTLS-classified datagrams to
//! [`DtlsDatagramBridge::feed_inbound`].

use std::net::SocketAddr;

use async_trait::async_trait;
use bytes::Bytes;
use tokio::sync::{mpsc, Mutex as AsyncMutex};

use webrtc_util::conn::Conn;

/// Inbound channel capacity. Sized for handshake traffic only (a handful
/// of flights, not a sustained media-plane rate) — a full channel means
/// the handshake consumer has stalled, not that the caller's reactor is
/// too fast.
const INBOUND_CHANNEL_CAPACITY: usize = 32;

/// Bridges a caller-owned UDP reactor to [`Conn`] without this crate ever
/// touching a real socket. See the module docs for the full picture.
pub struct DtlsDatagramBridge {
    local_addr: SocketAddr,
    remote_addr: SocketAddr,
    outbound: mpsc::Sender<(Bytes, SocketAddr)>,
    inbound_tx: mpsc::Sender<Bytes>,
    inbound_rx: AsyncMutex<mpsc::Receiver<Bytes>>,
}

impl DtlsDatagramBridge {
    /// `local_addr`/`remote_addr` are reported back through
    /// [`Conn::local_addr`]/[`Conn::remote_addr`] only — this bridge never
    /// binds or connects a real socket, so these can be whatever addresses
    /// the caller's own socket is actually using.
    ///
    /// `outbound` is the caller's own channel: every byte this bridge (and
    /// the DTLS handshake running over it) needs to send is pushed there,
    /// paired with the destination address, for the caller's reactor to
    /// actually put on the wire.
    pub fn new(
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
        outbound: mpsc::Sender<(Bytes, SocketAddr)>,
    ) -> Self {
        let (inbound_tx, inbound_rx) = mpsc::channel(INBOUND_CHANNEL_CAPACITY);
        Self {
            local_addr,
            remote_addr,
            outbound,
            inbound_tx,
            inbound_rx: AsyncMutex::new(inbound_rx),
        }
    }

    /// Push one inbound datagram the caller's own reactor received from
    /// `remote_addr`. Non-blocking (`try_send`) — safe to call from a
    /// synchronous `mio` poll loop, never awaits. Returns `false` if the
    /// datagram was dropped (inbound channel full, or the `Conn` side has
    /// already been dropped) — both mean the handshake consumer isn't
    /// keeping up or is gone, never a signal to retry the same datagram
    /// later.
    pub fn feed_inbound(&self, data: Bytes) -> bool {
        self.inbound_tx.try_send(data).is_ok()
    }
}

#[async_trait]
impl Conn for DtlsDatagramBridge {
    async fn connect(&self, _addr: SocketAddr) -> webrtc_util::Result<()> {
        // Scoped to exactly one remote peer at construction; no separate
        // connect step.
        Ok(())
    }

    async fn recv(&self, buf: &mut [u8]) -> webrtc_util::Result<usize> {
        let (n, _addr) = self.recv_from(buf).await?;
        Ok(n)
    }

    async fn recv_from(&self, buf: &mut [u8]) -> webrtc_util::Result<(usize, SocketAddr)> {
        let mut inbound = self.inbound_rx.lock().await;
        match inbound.recv().await {
            Some(datagram) => {
                // Matches plain UDP-socket semantics: a datagram larger
                // than the caller's buffer is truncated, not an error.
                let n = datagram.len().min(buf.len());
                buf[..n].copy_from_slice(&datagram[..n]);
                Ok((n, self.remote_addr))
            }
            None => {
                Err(std::io::Error::other("DTLS datagram bridge inbound channel closed").into())
            }
        }
    }

    async fn send(&self, buf: &[u8]) -> webrtc_util::Result<usize> {
        self.send_to(buf, self.remote_addr).await
    }

    async fn send_to(&self, buf: &[u8], target: SocketAddr) -> webrtc_util::Result<usize> {
        let len = buf.len();
        self.outbound
            .send((Bytes::copy_from_slice(buf), target))
            .await
            .map_err(|_| std::io::Error::other("DTLS datagram bridge outbound channel closed"))?;
        Ok(len)
    }

    fn local_addr(&self) -> webrtc_util::Result<SocketAddr> {
        Ok(self.local_addr)
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        Some(self.remote_addr)
    }

    async fn close(&self) -> webrtc_util::Result<()> {
        Ok(())
    }

    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }
}
