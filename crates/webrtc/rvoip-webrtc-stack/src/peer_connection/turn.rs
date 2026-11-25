//! Owned UDP TURN transport. The ICE core sees the allocated relay address as
//! a socket; this adapter carries every ICE, DTLS and media packet through TURN.
use super::{UdpSocketDriverOptions, bind_udp_from_range, ice_gatherer::RTCIceGatherOptions};
use crate::runtime::{AsyncUdpSocket, Runtime};
use rtc::ice::candidate::CandidateConfig;
use rtc::peer_connection::configuration::RTCIceTransportPolicy;
use rtc::peer_connection::transport::{CandidateRelayConfig, RTCIceCandidate, RTCIceCandidateInit};
use rtc::shared::error::{Error, Result};
use std::{
    collections::HashMap, fmt, future::Future, io, net::SocketAddr, pin::Pin, sync::Arc,
    time::Duration,
};
use tokio::sync::Mutex;
use turn_client::client::{Client, ClientConfig};
use turn_util::conn::Conn;

/// Deployment-controlled bounds for the UDP TURN client. No external service
/// or credentials are implied; servers come exclusively from RTCConfiguration.
#[derive(Clone, Debug)]
pub struct TurnOptions {
    /// One absolute deadline shared by DNS and all allocation attempts.
    pub allocation_timeout: Duration,
    /// Per-address budget; permits fallback within the total allocation deadline.
    pub attempt_timeout: Duration,
    /// Bound each permission/channel transaction on the media path.
    pub send_timeout: Duration,
    /// Bound best-effort allocation release before closing the control socket.
    pub close_timeout: Duration,
}
impl Default for TurnOptions {
    fn default() -> Self {
        Self {
            allocation_timeout: Duration::from_secs(5),
            attempt_timeout: Duration::from_secs(2),
            send_timeout: Duration::from_secs(2),
            close_timeout: Duration::from_secs(2),
        }
    }
}

// Restrict the control socket to the selected TURN server. In particular,
// never pass an unsolicited DATA indication from another source to the client.
struct ServerConn {
    socket: Arc<dyn AsyncUdpSocket>,
    server: SocketAddr,
    closed: tokio_util::sync::CancellationToken,
}
#[async_trait::async_trait]
impl Conn for ServerConn {
    async fn connect(&self, _: SocketAddr) -> turn_util::Result<()> {
        Err(io::Error::other("TURN control socket is fixed").into())
    }
    async fn recv(&self, _: &mut [u8]) -> turn_util::Result<usize> {
        Err(io::Error::other("use recv_from").into())
    }
    async fn send(&self, _: &[u8]) -> turn_util::Result<usize> {
        Err(io::Error::other("use send_to").into())
    }
    async fn recv_from(&self, buf: &mut [u8]) -> turn_util::Result<(usize, SocketAddr)> {
        loop {
            tokio::select! {
                biased;
                _ = self.closed.cancelled() => return Err(io::Error::other("TURN control closed").into()),
                result = self.socket.recv_from(buf) => {
                    let (n, from) = result?;
                    if from == self.server { return Ok((n, from)); }
                }
            }
        }
    }
    async fn send_to(&self, buf: &[u8], addr: SocketAddr) -> turn_util::Result<usize> {
        if addr != self.server || self.closed.is_cancelled() {
            return Err(io::Error::other("TURN destination rejected").into());
        }
        Ok(self.socket.send_to(buf, addr).await?)
    }
    fn local_addr(&self) -> turn_util::Result<SocketAddr> {
        Ok(self.socket.local_addr()?)
    }
    fn remote_addr(&self) -> Option<SocketAddr> {
        Some(self.server)
    }
    async fn close(&self) -> turn_util::Result<()> {
        self.closed.cancel();
        Ok(())
    }
    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }
}

struct Allocation {
    client: Client,
    control: Arc<ServerConn>,
    relay: Option<Arc<dyn Conn + Send + Sync>>,
    close_timeout: Duration,
}
impl Allocation {
    async fn close(self) {
        // Stop refresh tasks and send a zero-lifetime Refresh while the
        // listener is still alive. Always cancel it even when release times out.
        if let Some(relay) = self.relay {
            let _ = tokio::time::timeout(self.close_timeout, relay.close()).await;
        }
        self.control.closed.cancel();
        let _ = tokio::time::timeout(self.close_timeout, self.client.close()).await;
    }
}
/// The lease, not the driver/relay socket, owns client background tasks. It
/// cleans up even if build, signaling, or a call task is cancelled mid-await.
#[derive(Default)]
pub(crate) struct TurnLease {
    allocation: Mutex<Option<Allocation>>,
    candidate: Option<RTCIceCandidateInit>,
}
impl TurnLease {
    pub(crate) fn candidate(&self) -> Option<RTCIceCandidateInit> {
        self.candidate.clone()
    }
    pub(crate) async fn close(&self) {
        if let Some(allocation) = self.allocation.lock().await.take() {
            // An aborted caller must not cancel allocation/task cleanup.
            let _ = tokio::spawn(allocation.close()).await;
        }
    }
}
impl Drop for TurnLease {
    fn drop(&mut self) {
        if let Some(allocation) = self.allocation.get_mut().take() {
            // Cancellation is synchronous; release can proceed on the runtime.
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(allocation.close());
            } else {
                allocation.control.closed.cancel();
            }
        }
    }
}
struct RelaySocket {
    conn: Arc<dyn Conn + Send + Sync>,
    address: SocketAddr,
    send_timeout: Duration,
}
impl fmt::Debug for RelaySocket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RelaySocket([redacted])")
    }
}
impl AsyncUdpSocket for RelaySocket {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.address)
    }
    fn send_to<'a>(
        &'a self,
        buf: &'a [u8],
        target: SocketAddr,
    ) -> Pin<Box<dyn Future<Output = io::Result<usize>> + Send + 'a>> {
        Box::pin(async move {
            tokio::time::timeout(self.send_timeout, self.conn.send_to(buf, target))
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "TURN send timeout"))?
                .map_err(|_| io::Error::other("TURN send failed"))
        })
    }
    fn recv_from<'a>(
        &'a self,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = io::Result<(usize, SocketAddr)>> + Send + 'a>> {
        Box::pin(async move {
            self.conn
                .recv_from(buf)
                .await
                .map_err(|_| io::Error::other("TURN receive failed"))
        })
    }
}

pub(crate) async fn allocate(
    opts: &RTCIceGatherOptions,
    udp: &UdpSocketDriverOptions,
    runtime: &Arc<dyn Runtime>,
    sockets: &mut HashMap<SocketAddr, Arc<dyn AsyncUdpSocket>>,
) -> Result<TurnLease> {
    let relay_only = opts.ice_gather_policy == RTCIceTransportPolicy::Relay;
    let deadline = tokio::time::Instant::now() + udp.turn_options.allocation_timeout;
    let mut saw_turn = false;
    for server in &opts.ice_servers {
        for url in &server.urls {
            if !url.starts_with("turn:") && !url.starts_with("turns:") {
                continue;
            }
            saw_turn = true;
            // Only UDP TURN is supported here. Never reinterpret TCP/TLS as UDP.
            let Some((host, port)) = udp_endpoint(url) else {
                continue;
            };
            let attempt = async {
                let destinations = tokio::net::lookup_host((host.as_str(), port)).await?;
                for destination in destinations {
                    let Some((&base, existing)) = sockets
                        .iter()
                        .find(|(addr, _)| addr.is_ipv4() == destination.is_ipv4())
                    else {
                        continue;
                    };
                    let socket = if relay_only {
                        existing.clone()
                    } else {
                        let raw = if let Some((ip, start, end)) = udp.port_range {
                            bind_udp_from_range(ip, start, end)?
                        } else {
                            std::net::UdpSocket::bind(SocketAddr::new(base.ip(), 0))?
                        };
                        raw.set_nonblocking(true)?;
                        runtime.wrap_udp_socket(raw)?
                    };
                    let related = socket.local_addr()?;
                    let control = Arc::new(ServerConn {
                        socket,
                        server: destination,
                        closed: tokio_util::sync::CancellationToken::new(),
                    });
                    let client = Client::new(ClientConfig {
                        stun_serv_addr: String::new(),
                        turn_serv_addr: destination.to_string(),
                        username: server.username.clone(),
                        password: server.credential.clone(),
                        realm: String::new(),
                        software: String::new(),
                        rto_in_ms: 200,
                        conn: control.clone(),
                        vnet: None,
                    })
                    .await
                    .map_err(|_| Error::Other("TURN client initialization failed".into()))?;
                    let mut lease = TurnLease {
                        allocation: Mutex::new(Some(Allocation {
                            client: client.clone(),
                            control,
                            relay: None,
                            close_timeout: udp.turn_options.close_timeout,
                        })),
                        candidate: None,
                    };
                    client
                        .listen()
                        .await
                        .map_err(|_| Error::Other("TURN listener failed".into()))?;
                    let relay: Arc<dyn Conn + Send + Sync> = match tokio::time::timeout(
                        udp.turn_options.attempt_timeout,
                        client.allocate(),
                    )
                    .await
                    {
                        Ok(Ok(relay)) => Arc::new(relay),
                        _ => {
                            lease.close().await;
                            continue;
                        }
                    };
                    let address = relay
                        .local_addr()
                        .map_err(|_| Error::Other("TURN relay address unavailable".into()))?;
                    lease.allocation.get_mut().as_mut().unwrap().relay = Some(relay.clone());
                    let candidate = CandidateRelayConfig {
                        base_config: CandidateConfig {
                            network: "udp".into(),
                            address: address.ip().to_string(),
                            port: address.port(),
                            component: 1,
                            ..Default::default()
                        },
                        rel_addr: related.ip().to_string(),
                        rel_port: related.port(),
                        ..Default::default()
                    }
                    .new_candidate_relay()?;
                    lease.candidate = Some(RTCIceCandidate::from(&candidate).to_json()?);
                    if relay_only {
                        sockets.clear();
                    }
                    sockets.insert(
                        address,
                        Arc::new(RelaySocket {
                            conn: relay,
                            address,
                            send_timeout: udp.turn_options.send_timeout,
                        }),
                    );
                    log::info!("TURN UDP allocation established");
                    return Ok::<_, Error>(Some(lease));
                }
                Ok(None)
            };
            match tokio::time::timeout_at(deadline, attempt).await {
                Ok(Ok(Some(lease))) => return Ok(lease),
                Err(_) => break,
                _ => continue,
            }
        }
    }
    if relay_only {
        return Err(Error::Other(
            "TURN relay required but no UDP allocation succeeded".into(),
        ));
    }
    if saw_turn {
        log::warn!("TURN allocation unavailable; ICE policy permits direct candidates");
    }
    Ok(TurnLease::default())
}

fn udp_endpoint(value: &str) -> Option<(String, u16)> {
    let rest = value.strip_prefix("turn:")?;
    if rest.starts_with("//") {
        return None;
    }
    let url = url::Url::parse(&format!("turn://{rest}")).ok()?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
    {
        return None;
    }
    let query: Vec<_> = url.query_pairs().collect();
    if !(query.is_empty() || (query.len() == 1 && query[0].0 == "transport" && query[0].1 == "udp"))
    {
        return None;
    }
    let port = url.port().unwrap_or(3478);
    if port == 0 {
        return None;
    }
    Some((url.host_str()?.trim_matches(['[', ']']).to_owned(), port))
}

#[cfg(test)]
mod tests {
    use super::udp_endpoint;
    #[test]
    fn udp_uri_selection_is_explicit() {
        assert_eq!(
            udp_endpoint("turn:localhost:3479?transport=udp"),
            Some(("localhost".into(), 3479))
        );
        assert_eq!(udp_endpoint("turn:[::1]:3478"), Some(("::1".into(), 3478)));
        for invalid in [
            "turns:localhost",
            "turn:localhost?transport=tcp",
            "turn:user:secret@localhost",
            "turn://localhost",
            "turn:localhost/path",
            "turn:localhost?x=y",
            "turn:localhost:0",
        ] {
            assert!(udp_endpoint(invalid).is_none());
        }
    }
}
