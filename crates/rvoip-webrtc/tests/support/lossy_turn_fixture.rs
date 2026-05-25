//! G-tail closeout — lossy TURN-relay fixture.
//!
//! Composes the existing [`super::coturn_fixture::CoturnFixture`] with a
//! per-client lossy UDP proxy sitting in front of coturn's control port
//! (3478). Both peers point their TURN URL at the proxy instead of
//! coturn directly; the proxy forwards UDP datagrams in either direction
//! and drops each one with probability `loss_rate` using a seeded RNG.
//!
//! The relay-port hop (`coturn:50000 ↔ coturn:50001`) stays local inside
//! coturn — we only need to add loss on the control-channel hops where
//! Send-Indication / Data-Indication payloads carry the actual media.
//!
//! Used by [`tests/lossy_turn_nack.rs`](../../lossy_turn_nack.rs) to
//! prove the registered RTCP-NACK feedback round-trips end-to-end. Skips
//! gracefully when Docker isn't reachable (inherits the [`CoturnFixture`]
//! contract).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use rand::{rngs::StdRng, Rng, SeedableRng};
use rvoip_webrtc::IceServerConfig;
use tokio::net::UdpSocket;

use super::coturn_fixture::{CoturnFixture, TURN_PASSWORD, TURN_USERNAME};

pub struct LossyTurnFixture {
    coturn: CoturnFixture,
    proxy_addr: SocketAddr,
}

impl LossyTurnFixture {
    pub async fn start(loss_rate: f64, seed: u64) -> Option<Self> {
        let coturn = CoturnFixture::start().await?;
        let upstream: SocketAddr = format!("127.0.0.1:{}", coturn.host_port())
            .parse()
            .ok()?;
        let proxy_addr = spawn_lossy_turn_proxy(upstream, loss_rate, seed).await?;
        Some(Self { coturn, proxy_addr })
    }

    pub fn ice_config(&self) -> IceServerConfig {
        IceServerConfig::turn(
            format!("turn:127.0.0.1:{}?transport=udp", self.proxy_addr.port()),
            TURN_USERNAME,
            TURN_PASSWORD,
        )
    }

    pub fn proxy_port(&self) -> u16 {
        self.proxy_addr.port()
    }

    pub fn coturn(&self) -> &CoturnFixture {
        &self.coturn
    }
}

/// Spawn a UDP proxy listening on a random localhost port. For every
/// source-addr that sends a datagram, the proxy lazily binds a fresh
/// upstream socket and forwards to `upstream`. Reply traffic on that
/// upstream socket is forwarded back to the same source-addr. Each
/// direction independently drops datagrams with probability `loss_rate`.
async fn spawn_lossy_turn_proxy(
    upstream: SocketAddr,
    loss_rate: f64,
    seed: u64,
) -> Option<SocketAddr> {
    let listener = Arc::new(UdpSocket::bind("127.0.0.1:0").await.ok()?);
    let listen_addr = listener.local_addr().ok()?;

    let socket = Arc::clone(&listener);
    tokio::spawn(async move {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut clients: HashMap<SocketAddr, Arc<UdpSocket>> = HashMap::new();
        let mut buf = vec![0u8; 8192];
        loop {
            let (n, peer) = match socket.recv_from(&mut buf).await {
                Ok(v) => v,
                Err(_) => return,
            };

            // Drop on ingress (peer → upstream).
            if rng.gen::<f64>() < loss_rate {
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
                    let reply_seed = seed.wrapping_add((peer.port() as u64) << 32);
                    tokio::spawn(async move {
                        let mut rng = StdRng::seed_from_u64(reply_seed);
                        let mut buf = vec![0u8; 8192];
                        loop {
                            let n = match s_reply.recv(&mut buf).await {
                                Ok(n) => n,
                                Err(_) => return,
                            };
                            if rng.gen::<f64>() < loss_rate {
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
