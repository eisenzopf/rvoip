//! Phase 4 demo — orchestrator that bridges UCTP (QUIC + WebTransport) to SIP.
//!
//! Brings up:
//! - Shared `quinn::Endpoint` with both ALPNs (`uctp/1`, `h3`) on a self-signed cert
//! - `UctpQuicAdapter` + `UctpWtAdapter` on that endpoint
//! - `SipAdapter` on a separate UDP port
//!
//! Subscribes to the orchestrator's cross-transport event bus and logs
//! every event. The actual frame-pump bridging between SIP and UCTP is
//! a v0.x follow-up (per design doc §6.2) — `Orchestrator::bridge_connections`
//! is still stubbed in rvoip-core. v0 demo exercises:
//!   - adapter wiring (`register`)
//!   - cross-adapter event normalization (`subscribe_events`)
//!   - the dual-ALPN shared-endpoint deployment pattern
//!
//! Run:
//! ```bash
//! cargo run -p rvoip-uctp --example orchestrator_bridge -- \
//!     --uctp-bind 127.0.0.1:4433 \
//!     --sip-bind  127.0.0.1:5072
//! ```
//!
//! Writes the self-signed cert PEM to `/tmp/uctp_demo_cert.pem` so the
//! `uctp_agent_quic` / `uctp_agent_wt` agent binaries can trust it.

use std::net::SocketAddr;
use std::sync::Arc;

use rvoip_core::{Config, Orchestrator, Transport};
use rvoip_quic::{UctpQuicAdapter, UctpQuicConfig};
use rvoip_sip::api::unified::{Config as SipConfig, UnifiedCoordinator};
use rvoip_sip::SipAdapter;
use rvoip_auth_core::bearer_stub;
use rvoip_uctp::substrate::{dispatch_by_alpn, make_server_endpoint, self_signed_for_dev};
use rvoip_webtransport::{UctpWtAdapter, UctpWtConfig};
use rvoip_websocket::{UctpWsAdapter, UctpWsConfig};
use tokio::net::TcpListener;

const ALPN_UCTP: &[u8] = b"uctp/1";
const ALPN_H3: &[u8] = b"h3";
const CERT_DER_PATH: &str = "/tmp/uctp_demo_cert.der";

#[derive(Debug)]
struct Args {
    uctp_bind: SocketAddr,
    ws_bind: SocketAddr,
    sip_bind: SocketAddr,
}

impl Args {
    fn parse() -> Self {
        let mut uctp_bind: SocketAddr = "127.0.0.1:4433".parse().unwrap();
        let mut ws_bind: SocketAddr = "127.0.0.1:7777".parse().unwrap();
        let mut sip_bind: SocketAddr = "127.0.0.1:5072".parse().unwrap();
        let mut iter = std::env::args().skip(1);
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--uctp-bind" => uctp_bind = iter.next().unwrap().parse().unwrap(),
                "--ws-bind" => ws_bind = iter.next().unwrap().parse().unwrap(),
                "--sip-bind" => sip_bind = iter.next().unwrap().parse().unwrap(),
                "--help" | "-h" => {
                    eprintln!(
                        "usage: orchestrator_bridge [--uctp-bind ADDR] [--ws-bind ADDR] [--sip-bind ADDR]"
                    );
                    std::process::exit(0);
                }
                other => {
                    eprintln!("unknown arg: {other}");
                    std::process::exit(2);
                }
            }
        }
        Self {
            uctp_bind,
            ws_bind,
            sip_bind,
        }
    }
}

fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,rvoip_sip_dialog=warn".into()),
        )
        .init();
    install_crypto_provider();

    let args = Args::parse();
    println!(
        "[orchestrator_bridge] uctp_bind={}  ws_bind={}  sip_bind={}",
        args.uctp_bind, args.ws_bind, args.sip_bind
    );

    // --- 1. Shared quinn::Endpoint with both ALPNs ---
    let (cert_der, key_der) = self_signed_for_dev(&["localhost".into()])?;
    let mut tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)?;
    tls.alpn_protocols = vec![ALPN_UCTP.to_vec(), ALPN_H3.to_vec()];

    let quinn_ep = Arc::new(make_server_endpoint(
        args.uctp_bind,
        Arc::new(tls),
        quinn::TransportConfig::default(),
    )?);

    // Persist the cert (DER form — agents read with std::fs::read +
    // CertificateDer::from). PEM would be nicer for human inspection but
    // the rustls 0.23 CertificateDer doesn't ship a `to_pem` method and
    // adding the `pem` crate just for the demo is overkill.
    std::fs::write(CERT_DER_PATH, cert_der.as_ref())?;
    println!("[orchestrator_bridge] wrote demo cert (DER) to {CERT_DER_PATH}");

    let mut routes = dispatch_by_alpn(Arc::clone(&quinn_ep), &[ALPN_UCTP, ALPN_H3])?;
    let uctp_accept_rx = routes.take(ALPN_UCTP).expect("uctp/1 channel");
    let wt_accept_rx = routes.take(ALPN_H3).expect("h3 channel");

    // --- 2. Build the three adapters ---
    let quic_adapter = UctpQuicAdapter::new(UctpQuicConfig::new(
        Arc::clone(&quinn_ep),
        uctp_accept_rx,
        bearer_stub(),
    ))
    .await?;
    let wt_adapter = UctpWtAdapter::new(UctpWtConfig::new(
        Arc::clone(&quinn_ep),
        wt_accept_rx,
        bearer_stub(),
    ))
    .await?;

    // WebSocket listener (TCP, no TLS for v0 demo).
    let ws_listener = TcpListener::bind(args.ws_bind).await?;
    let ws_adapter = UctpWsAdapter::new(UctpWsConfig::new(ws_listener, bearer_stub())).await?;

    let sip_coordinator = UnifiedCoordinator::new(SipConfig::on(
        "rvoip-orchestrator-bridge",
        args.sip_bind.ip(),
        args.sip_bind.port(),
    ))
    .await?;
    let sip_adapter = SipAdapter::new(sip_coordinator).await?;

    // --- 3. Register all four with the cross-transport orchestrator ---
    let orchestrator = Orchestrator::new(Config::default());
    orchestrator.register(quic_adapter)?;
    orchestrator.register(wt_adapter)?;
    orchestrator.register(ws_adapter)?;
    orchestrator.register(sip_adapter)?;

    for transport in [
        Transport::Quic,
        Transport::WebTransport,
        Transport::WebSocket,
        Transport::Sip,
    ] {
        let a = orchestrator
            .adapter(transport)
            .expect("adapter just registered");
        println!(
            "[orchestrator_bridge] registered: transport={:?} kind={:?}",
            a.transport(),
            a.kind()
        );
    }

    // --- 4. Subscribe + auto-bridge policy ---
    // Bridge the first two non-SIP `ConnectionInbound`s that arrive.
    // The orchestrator's `bridge_connections` (un-stubbed in FP-B)
    // resolves each connection's audio stream via the matching
    // adapter, spawns a bidirectional frame pump, and emits
    // `Event::ConnectionsBridged`. Audio frames pushed from one
    // peer's `frames_out` arrive on the other peer's `frames_in` —
    // proven by `cross_transport_bridge.rs` for QUIC↔WT and WT↔WT.
    //
    // SIP-side bridging is deferred: SIP connections need their own
    // streams populated (the SipAdapter's `streams()` impl is still
    // returning the legacy SIP-bridge shape, not a MediaStream).
    // Tracked as a follow-up in the deferred-items list.
    let mut events = orchestrator.subscribe_events();
    let orch_for_bridge = Arc::clone(&orchestrator);
    let mut pending: Vec<rvoip_core::ids::ConnectionId> = Vec::new();
    println!("[orchestrator_bridge] ready — waiting for events (Ctrl-C to quit)");
    while let Ok(event) = events.recv().await {
        println!("[orchestrator_bridge] {event:?}");
        if let rvoip_core::events::Event::ConnectionInbound { connection_id, .. } = &event {
            pending.push(connection_id.clone());
            if pending.len() == 2 {
                let a = pending.remove(0);
                let b = pending.remove(0);
                match orch_for_bridge.bridge_connections(a.clone(), b.clone()).await {
                    Ok(bid) => println!(
                        "[orchestrator_bridge] auto-bridged {} <-> {} as {}",
                        a, b, bid
                    ),
                    Err(e) => println!(
                        "[orchestrator_bridge] auto-bridge failed for {} <-> {}: {}",
                        a, b, e
                    ),
                }
            }
        }
    }

    Ok(())
}
