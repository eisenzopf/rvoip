//! Shared quinn endpoint helpers.
//!
//! Both `rvoip-quic` (ALPN `uctp/1`) and `rvoip-webtransport` (ALPN `h3`)
//! can deploy onto a single shared `quinn::Endpoint`. [`dispatch_by_alpn`]
//! is the single-consumer accept loop that fans handshook
//! `quinn::Connection`s out to per-adapter channels by their negotiated
//! ALPN. See design doc §5.4 ("Dual-ALPN single-endpoint deployment").

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use quinn::{Endpoint, EndpointConfig, ServerConfig, TransportConfig};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::errors::SubstrateError;

/// Default per-adapter accept-channel depth. Plenty for non-pathological
/// connection rates and small enough that an unconsumed channel
/// signals a stuck adapter quickly.
pub const ALPN_ACCEPT_CAP: usize = 64;

/// Build a quinn server endpoint bound to `addr`. `tls` MUST already
/// include all desired ALPNs in `alpn_protocols`. The returned endpoint
/// listens for incoming UDP packets but does not accept anything until
/// a consumer drives [`Endpoint::accept`] (typically via
/// [`dispatch_by_alpn`]).
pub fn make_server_endpoint(
    addr: SocketAddr,
    tls: Arc<rustls::ServerConfig>,
    transport_cfg: TransportConfig,
) -> Result<Endpoint, SubstrateError> {
    let crypto = Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from((*tls).clone())
            .map_err(|e| SubstrateError::Tls(rustls::Error::General(e.to_string())))?,
    );
    let mut server_cfg = ServerConfig::with_crypto(crypto);
    server_cfg.transport_config(Arc::new(transport_cfg));
    Endpoint::server(server_cfg, addr).map_err(SubstrateError::from)
}

/// Build a quinn client endpoint bound to `bind`. The returned endpoint
/// can dial via [`Endpoint::connect_with`].
pub fn make_client_endpoint(
    bind: SocketAddr,
    _client_cfg: Arc<rustls::ClientConfig>,
) -> Result<Endpoint, SubstrateError> {
    // We don't install the client config on the endpoint here because
    // quinn expects a `quinn::ClientConfig`, not `rustls::ClientConfig`.
    // The caller passes the per-connection config when dialing via
    // `Endpoint::connect_with` instead. Keeping the signature symmetric
    // with `make_server_endpoint` lets callers shape their setup the
    // same way for both sides.
    let endpoint = Endpoint::new(
        EndpointConfig::default(),
        None,
        std::net::UdpSocket::bind(bind)?,
        Arc::new(quinn::TokioRuntime),
    )
    .map_err(SubstrateError::from)?;
    Ok(endpoint)
}

/// Per-ALPN receiver handles handed back from [`dispatch_by_alpn`].
pub struct AlpnRoutes {
    routes: HashMap<Vec<u8>, mpsc::Receiver<quinn::Connection>>,
}

impl AlpnRoutes {
    /// Take ownership of the receiver for a specific ALPN. Returns
    /// `None` if the ALPN wasn't passed to [`dispatch_by_alpn`] or has
    /// already been taken.
    pub fn take(&mut self, alpn: &[u8]) -> Option<mpsc::Receiver<quinn::Connection>> {
        self.routes.remove(alpn)
    }
}

/// Single-consumer ALPN dispatcher.
///
/// Spawns one accept task on the given Endpoint, drives the QUIC
/// handshake to completion, reads the negotiated ALPN from each fully
/// established `quinn::Connection`, and forwards it to the matching
/// adapter's mpsc channel. Unrecognized ALPNs are closed with
/// `error_code = 0x01, reason = "alpn-not-registered"`.
///
/// **Why a single accept task** (and not "each adapter calls
/// `endpoint.accept()` and filters"): `quinn::Endpoint::accept()` is
/// single-consumer; parallel loops race for each connection. The
/// dispatcher is the only correct shape for the dual-ALPN shared
/// endpoint that the design doc commits to.
///
/// **Why post-handshake `Connection`** (and not pre-handshake `Incoming`):
/// the ALPN can only be read after the handshake completes. Forwarding
/// `Incoming` through a channel would require the receiver to drive the
/// handshake itself, but then the receiver couldn't know which ALPN
/// channel to route to before driving. The "drive handshake centrally,
/// then route" shape is simpler and correct.
pub fn dispatch_by_alpn(
    endpoint: Arc<Endpoint>,
    alpns: &[&[u8]],
) -> Result<AlpnRoutes, SubstrateError> {
    let mut tx_map: HashMap<Vec<u8>, mpsc::Sender<quinn::Connection>> = HashMap::new();
    let mut routes: HashMap<Vec<u8>, mpsc::Receiver<quinn::Connection>> = HashMap::new();
    for alpn in alpns {
        let (tx, rx) = mpsc::channel(ALPN_ACCEPT_CAP);
        tx_map.insert(alpn.to_vec(), tx);
        routes.insert(alpn.to_vec(), rx);
    }

    let ep = endpoint.clone();
    tokio::spawn(async move {
        loop {
            let Some(incoming) = ep.accept().await else {
                debug!("substrate.quinn: endpoint closed; dispatcher exiting");
                return;
            };
            let connecting = match incoming.accept() {
                Ok(c) => c,
                Err(e) => {
                    warn!(error = %e, "substrate.quinn: accept() failed");
                    continue;
                }
            };
            let txs = tx_map.clone();
            tokio::spawn(async move {
                let conn = match connecting.await {
                    Ok(c) => c,
                    Err(e) => {
                        warn!(error = %e, "substrate.quinn: handshake failed");
                        return;
                    }
                };
                let alpn = conn
                    .handshake_data()
                    .and_then(|d| d.downcast::<quinn::crypto::rustls::HandshakeData>().ok())
                    .and_then(|d| d.protocol.clone())
                    .unwrap_or_default();
                match txs.get(&alpn) {
                    Some(tx) => {
                        if tx.send(conn).await.is_err() {
                            // Adapter dropped its receiver; just continue.
                        }
                    }
                    None => {
                        warn!(alpn = ?alpn, "substrate.quinn: ALPN not registered; closing");
                        conn.close(0x01u32.into(), b"alpn-not-registered");
                    }
                }
            });
        }
    });

    Ok(AlpnRoutes { routes })
}

/// Default interval for `quinn::Connection::stats()` sampling, per design
/// doc §3.9. Adapters override via their `quinn_stats_interval` config
/// field; setting that to zero disables the sampler entirely.
pub const DEFAULT_QUINN_STATS_INTERVAL: Duration = Duration::from_secs(5);

/// Spawn a per-connection task that polls [`quinn::Connection::stats`]
/// every `interval` and emits the gauges/counters listed in design doc
/// §3.9. The task exits when the connection closes (`close_reason()`
/// becomes `Some`). Returning the [`JoinHandle`] lets the caller abort
/// the sampler when its substrate connection task winds down.
///
/// `transport` is one of `"quic"` / `"webtransport"` — the same label
/// the coordinator uses, so per-transport comparison is direct.
///
/// Setting `interval` to zero disables the sampler (returns a task
/// that immediately exits) — supports the "sampler off" config knob
/// without forcing every adapter to branch.
pub fn spawn_stats_sampler(
    conn: quinn::Connection,
    transport: &'static str,
    interval: Duration,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if interval.is_zero() {
            return;
        }
        let mut ticker = tokio::time::interval(interval);
        // First tick fires immediately; skip so we don't emit
        // zero-valued metrics before any traffic.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if conn.close_reason().is_some() {
                return;
            }
            let stats = conn.stats();
            metrics::gauge!("uctp_quinn_rtt_seconds", "transport" => transport)
                .set(stats.path.rtt.as_secs_f64());
            metrics::gauge!("uctp_quinn_cwnd_bytes", "transport" => transport)
                .set(stats.path.cwnd as f64);
            metrics::counter!(
                "uctp_quinn_udp_datagrams_total",
                "transport" => transport,
                "direction" => "tx"
            )
            .absolute(stats.udp_tx.datagrams);
            metrics::counter!(
                "uctp_quinn_udp_datagrams_total",
                "transport" => transport,
                "direction" => "rx"
            )
            .absolute(stats.udp_rx.datagrams);
            metrics::counter!("uctp_quinn_lost_packets_total", "transport" => transport)
                .absolute(stats.path.lost_packets);
            metrics::counter!(
                "uctp_quinn_close_frames_total",
                "transport" => transport,
                "direction" => "tx"
            )
            .absolute(stats.frame_tx.connection_close);
            metrics::counter!(
                "uctp_quinn_close_frames_total",
                "transport" => transport,
                "direction" => "rx"
            )
            .absolute(stats.frame_rx.connection_close);
        }
    })
}
