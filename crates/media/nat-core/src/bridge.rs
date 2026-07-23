//! Bridges ICE connectivity checks onto a UDP socket that isn't owned
//! exclusively by this crate — e.g. `rvoip-rtp-core`'s existing shared
//! RTP/RTCP/DTLS/STUN socket (RFC 7983 demux), the same socket the DTLS-SRTP
//! handshake is bridged onto via `dtls_bridge::DtlsUdpConnAdapter`.
//!
//! `webrtc_ice::udp_mux::UDPMuxDefault` (the crate's own ready-made
//! `UDPMux` impl) can't be reused directly: it spawns its own task that
//! calls `recv_from` on the socket it's given, which would steal every
//! non-STUN datagram (RTP, RTCP, DTLS) arriving on a socket that's
//! actually shared. [`SharedIceMux`] instead reuses `UDPMuxDefault`'s own
//! internal building blocks — `UDPMuxConn`/`UDPMuxConnParams` for the
//! per-ufrag buffering `Conn`, `UDPMuxWriter` for outbound sends — but
//! never reads the socket itself. Inbound bytes the caller's own demux
//! loop has already classified as STUN (RFC 7983, leading two bits `00`)
//! are pushed in via [`SharedIceMux::handle_incoming`].

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex as AsyncMutex;
use webrtc_ice::udp_mux::{UDPMux, UDPMuxConn, UDPMuxConnParams, UDPMuxWriter};
use webrtc_util::{Conn, Error as UtilError};

/// The caller's side of the shared socket: outbound sends only. Inbound
/// bytes are not read here — the caller's own demux loop owns the read
/// half and pushes STUN datagrams in via [`SharedIceMux::handle_incoming`].
#[async_trait]
pub trait SharedIceSocket: Send + Sync {
    async fn send_to(&self, buf: &[u8], target: SocketAddr) -> std::io::Result<usize>;
    fn local_addr(&self) -> SocketAddr;
}

/// A `UDPMux` + `UDPMuxWriter` implementation over a socket this crate
/// doesn't own the read half of.
pub struct SharedIceMux {
    socket: Arc<dyn SharedIceSocket>,
    conns: AsyncMutex<HashMap<String, UDPMuxConn>>,
    address_map: parking_lot::RwLock<HashMap<SocketAddr, UDPMuxConn>>,
}

impl SharedIceMux {
    pub fn new(socket: Arc<dyn SharedIceSocket>) -> Arc<Self> {
        Arc::new(Self {
            socket,
            conns: AsyncMutex::new(HashMap::new()),
            address_map: parking_lot::RwLock::new(HashMap::new()),
        })
    }

    /// Feed one inbound datagram the caller's demux loop has already
    /// classified as STUN. Looks up the destination `Conn` first by
    /// learned source address, then — for STUN messages only — by the
    /// ufrag prefix of the STUN `USERNAME` attribute (`"ufrag:remoteufrag"`
    /// per RFC 8445 §7.1), same two-step lookup `UDPMuxDefault` runs in
    /// its own read task.
    pub async fn handle_incoming(&self, buf: &[u8], addr: SocketAddr) {
        let conn = {
            let map = self.address_map.read();
            map.get(&addr).cloned()
        };
        let conn = match conn {
            Some(c) => Some(c),
            None if stun::message::is_message(buf) => self.conn_from_stun_message(buf).await,
            _ => None,
        };
        if let Some(conn) = conn {
            if let Err(err) = conn.write_packet(buf, addr).await {
                tracing::debug!(%addr, %err, "failed to hand inbound STUN packet to ICE conn");
            }
        } else {
            tracing::trace!(%addr, "dropping unroutable inbound STUN packet");
        }
    }

    async fn conn_from_stun_message(&self, buf: &[u8]) -> Option<UDPMuxConn> {
        let mut message = stun::message::Message::default();
        message.unmarshal_binary(buf).ok()?;
        let username = message.get(stun::attributes::ATTR_USERNAME).ok()?;
        let username = String::from_utf8(username).ok()?;
        let ufrag = username.split(':').next()?;
        let conns = self.conns.lock().await;
        conns.get(ufrag).cloned()
    }
}

#[async_trait]
impl UDPMux for SharedIceMux {
    async fn close(&self) -> Result<(), UtilError> {
        let old = {
            let mut conns = self.conns.lock().await;
            std::mem::take(&mut *conns)
        };
        for (_, conn) in old {
            conn.close();
        }
        *self.address_map.write() = HashMap::new();
        Ok(())
    }

    async fn get_conn(
        self: Arc<Self>,
        ufrag: &str,
    ) -> Result<Arc<dyn Conn + Send + Sync>, UtilError> {
        let mut conns = self.conns.lock().await;
        if let Some(c) = conns.get(ufrag) {
            return Ok(Arc::new(c.clone()) as Arc<dyn Conn + Send + Sync>);
        }

        let params = UDPMuxConnParams {
            local_addr: self.socket.local_addr(),
            key: ufrag.to_string(),
            udp_mux: Arc::downgrade(&self) as _,
        };
        let muxed = UDPMuxConn::new(params);
        conns.insert(ufrag.to_string(), muxed.clone());
        Ok(Arc::new(muxed) as Arc<dyn Conn + Send + Sync>)
    }

    async fn remove_conn_by_ufrag(&self, ufrag: &str) {
        let removed = {
            let mut conns = self.conns.lock().await;
            conns.remove(ufrag)
        };
        if let Some(conn) = removed {
            let mut map = self.address_map.write();
            for addr in conn.get_addresses() {
                map.remove(&addr);
            }
        }
    }
}

#[async_trait]
impl UDPMuxWriter for SharedIceMux {
    async fn register_conn_for_address(&self, conn: &UDPMuxConn, addr: SocketAddr) {
        let key = conn.key().to_string();
        let mut map = self.address_map.write();
        map.entry(addr)
            .and_modify(|e| {
                if e.key() != key {
                    e.remove_address(&addr);
                    *e = conn.clone();
                }
            })
            .or_insert_with(|| conn.clone());
    }

    async fn send_to(&self, buf: &[u8], target: &SocketAddr) -> Result<usize, UtilError> {
        Ok(self.socket.send_to(buf, *target).await?)
    }
}
