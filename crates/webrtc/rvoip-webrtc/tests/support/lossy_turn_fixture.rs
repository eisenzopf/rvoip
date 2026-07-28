//! G-tail closeout — lossy TURN-relay fixture.
//!
//! Composes the existing [`super::coturn_fixture::CoturnFixture`] with a
//! per-client lossy UDP proxy sitting in front of the TURN control port
//! (3478). Both peers point their TURN URL at the proxy instead of
//! coturn directly; the proxy forwards UDP datagrams in either direction
//! and drops a deterministic fraction selected by `loss_rate` and a seed.
//!
//! The relay-port hop (`coturn:50000 ↔ coturn:50001`) stays local inside
//! the TURN server — we only need to add loss on the control-channel hops where
//! Send-Indication / Data-Indication payloads carry the actual media.
//!
//! Used by [`tests/lossy_turn_nack.rs`](../../lossy_turn_nack.rs) to
//! prove the registered RTCP-NACK feedback round-trips end-to-end. Skips
//! using an entirely hermetic in-process server.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use rtc::stun::message::{is_stun_message, Message, CLASS_INDICATION, METHOD_DATA, METHOD_SEND};
use rvoip_webrtc::IceServerConfig;
use tokio::net::UdpSocket;

use super::coturn_fixture::{CoturnFixture, TURN_PASSWORD, TURN_USERNAME};

pub struct LossyTurnFixture {
    #[allow(dead_code)] // shared test fixture; used by some integration tests, not all
    coturn: CoturnFixture,
    proxy_addr: SocketAddr,
    loss: Arc<LossControl>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LossyTurnSnapshot {
    pub client_packets: u64,
    pub client_relay_packets: u64,
    pub client_packets_dropped: u64,
    pub server_packets: u64,
    pub server_relay_packets: u64,
    pub server_packets_dropped: u64,
}

#[derive(Debug)]
struct LossControl {
    enabled: AtomicBool,
    client_packets: AtomicU64,
    client_relay_packets: AtomicU64,
    client_packets_dropped: AtomicU64,
    server_packets: AtomicU64,
    server_relay_packets: AtomicU64,
    server_packets_dropped: AtomicU64,
}

impl LossControl {
    fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            client_packets: AtomicU64::new(0),
            client_relay_packets: AtomicU64::new(0),
            client_packets_dropped: AtomicU64::new(0),
            server_packets: AtomicU64::new(0),
            server_relay_packets: AtomicU64::new(0),
            server_packets_dropped: AtomicU64::new(0),
        }
    }

    fn snapshot(&self) -> LossyTurnSnapshot {
        LossyTurnSnapshot {
            client_packets: self.client_packets.load(Ordering::Relaxed),
            client_relay_packets: self.client_relay_packets.load(Ordering::Relaxed),
            client_packets_dropped: self.client_packets_dropped.load(Ordering::Relaxed),
            server_packets: self.server_packets.load(Ordering::Relaxed),
            server_relay_packets: self.server_relay_packets.load(Ordering::Relaxed),
            server_packets_dropped: self.server_packets_dropped.load(Ordering::Relaxed),
        }
    }
}

/// TURN ChannelData and Send/Data indications carry the peer datagram. Only
/// those datagrams are eligible for loss: allocation, permission, consent,
/// and refresh traffic must not make the media-loss assertion pass.
fn is_turn_relay_payload(packet: &[u8]) -> bool {
    if packet.len() >= 4 {
        let channel = u16::from_be_bytes([packet[0], packet[1]]);
        if (0x4000..=0x7fff).contains(&channel) {
            return true;
        }
    }
    if !is_stun_message(packet) {
        return false;
    }
    let mut message = Message::new();
    if message.unmarshal_binary(packet).is_err() || message.typ.class != CLASS_INDICATION {
        return false;
    }
    matches!(message.typ.method, METHOD_SEND | METHOD_DATA)
}

/// Deterministic fractional packet-loss schedule.
///
/// Accumulating the configured fraction guarantees one drop every
/// `1 / loss_rate` packets without making a release-blocking test depend on a
/// lucky pseudo-random sample. The seed selects a repeatable initial phase.
struct LossSchedule {
    loss_rate: f64,
    accumulator: f64,
}

impl LossSchedule {
    fn new(loss_rate: f64, seed: u64) -> Self {
        let phase = (seed % 10_000) as f64 / 10_000.0;
        Self {
            loss_rate,
            accumulator: phase,
        }
    }

    fn should_drop(&mut self, enabled: bool) -> bool {
        if !enabled || self.loss_rate <= 0.0 {
            return false;
        }
        self.accumulator += self.loss_rate;
        if self.accumulator >= 1.0 {
            self.accumulator -= 1.0;
            true
        } else {
            false
        }
    }
}

impl LossyTurnFixture {
    pub async fn start(loss_rate: f64, seed: u64) -> Option<Self> {
        let coturn = CoturnFixture::start().await.ok()?;
        let upstream: SocketAddr = format!("127.0.0.1:{}", coturn.host_port()).parse().ok()?;
        let loss = Arc::new(LossControl::new());
        let proxy_addr =
            spawn_lossy_turn_proxy(upstream, loss_rate, seed, Arc::clone(&loss)).await?;
        Some(Self {
            coturn,
            proxy_addr,
            loss,
        })
    }

    pub fn ice_config(&self) -> IceServerConfig {
        IceServerConfig::turn(
            format!("turn:127.0.0.1:{}?transport=udp", self.proxy_addr.port()),
            TURN_USERNAME,
            TURN_PASSWORD,
        )
    }

    /// Begin deterministic loss after ICE/TURN setup has completed.
    ///
    /// Keeping setup loss-free isolates the media/NACK contract and prevents
    /// retransmitted allocation traffic from satisfying the drop assertion.
    pub fn enable_loss(&self) {
        self.loss.enabled.store(true, Ordering::Release);
    }

    pub fn snapshot(&self) -> LossyTurnSnapshot {
        self.loss.snapshot()
    }

    #[allow(dead_code)] // shared test fixture; used by some integration tests, not all
    pub fn proxy_port(&self) -> u16 {
        self.proxy_addr.port()
    }

    #[allow(dead_code)] // shared test fixture accessor
    pub fn coturn(&self) -> &CoturnFixture {
        &self.coturn
    }

    pub async fn close(self) -> Result<(), turn_server::Error> {
        self.coturn.close().await
    }
}

/// Spawn a UDP proxy listening on a random localhost port. For every
/// source-addr that sends a datagram, the proxy lazily binds a fresh
/// upstream socket and forwards to `upstream`. Reply traffic on that
/// upstream socket is forwarded back to the same source-addr. Each
/// Each direction independently drops a deterministic fraction of datagrams
/// after [`LossyTurnFixture::enable_loss`] is called.
async fn spawn_lossy_turn_proxy(
    upstream: SocketAddr,
    loss_rate: f64,
    seed: u64,
    loss: Arc<LossControl>,
) -> Option<SocketAddr> {
    let listener = Arc::new(UdpSocket::bind("127.0.0.1:0").await.ok()?);
    let listen_addr = listener.local_addr().ok()?;

    let socket = Arc::clone(&listener);
    tokio::spawn(async move {
        let mut schedule = LossSchedule::new(loss_rate, seed);
        let mut clients: HashMap<SocketAddr, Arc<UdpSocket>> = HashMap::new();
        let mut buf = vec![0u8; 8192];
        loop {
            let (n, peer) = match socket.recv_from(&mut buf).await {
                Ok(v) => v,
                Err(_) => return,
            };

            loss.client_packets.fetch_add(1, Ordering::Relaxed);
            let relay_payload = is_turn_relay_payload(&buf[..n]);
            if relay_payload {
                loss.client_relay_packets.fetch_add(1, Ordering::Relaxed);
            }
            // Drop on ingress (peer → upstream).
            if schedule.should_drop(relay_payload && loss.enabled.load(Ordering::Acquire)) {
                loss.client_packets_dropped.fetch_add(1, Ordering::Relaxed);
                continue;
            }

            let upstream_sock = match clients.get(&peer) {
                Some(s) => Arc::clone(s),
                None => {
                    let s = match UdpSocket::bind("127.0.0.1:0").await {
                        Ok(s) => Arc::new(s),
                        Err(_) => continue,
                    };
                    clients.insert(peer, Arc::clone(&s));

                    // Reply pump: upstream → peer with its own seeded RNG.
                    let s_reply = Arc::clone(&s);
                    let socket_back = Arc::clone(&socket);
                    let loss_for_reply = Arc::clone(&loss);
                    let reply_seed = seed.wrapping_add((peer.port() as u64) << 32);
                    tokio::spawn(async move {
                        let mut schedule = LossSchedule::new(loss_rate, reply_seed);
                        let mut buf = vec![0u8; 8192];
                        loop {
                            let n = match s_reply.recv(&mut buf).await {
                                Ok(n) => n,
                                Err(_) => return,
                            };
                            loss_for_reply
                                .server_packets
                                .fetch_add(1, Ordering::Relaxed);
                            let relay_payload = is_turn_relay_payload(&buf[..n]);
                            if relay_payload {
                                loss_for_reply
                                    .server_relay_packets
                                    .fetch_add(1, Ordering::Relaxed);
                            }
                            if schedule.should_drop(
                                relay_payload && loss_for_reply.enabled.load(Ordering::Acquire),
                            ) {
                                loss_for_reply
                                    .server_packets_dropped
                                    .fetch_add(1, Ordering::Relaxed);
                                continue;
                            }
                            let _ = socket_back.send_to(&buf[..n], peer).await;
                        }
                    });
                    s
                }
            };

            let _ = upstream_sock.send_to(&buf[..n], upstream).await;
        }
    });

    Some(listen_addr)
}
