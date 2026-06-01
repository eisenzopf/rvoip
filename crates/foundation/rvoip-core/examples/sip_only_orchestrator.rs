//! Cross-transport `Orchestrator` example with one SIP adapter registered.
//!
//! This is the consumer pattern for cross-transport callers (Thelve, future
//! CPaaS, anyone planning to add WebRTC/QUIC adapters later): build a
//! `rvoip_core::Orchestrator`, register the SIP adapter, and dispatch
//! commands through the cross-transport seam. When `rvoip-webrtc` and
//! `rvoip-quic` adapters land, they register against the same Orchestrator
//! handle without reshaping consumer code.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p rvoip-core --example sip_only_orchestrator -- \
//!     --bind 127.0.0.1:5072
//! ```
//!
//! The example builds a SIP UnifiedCoordinator, wraps it in a `SipAdapter`,
//! registers it with `rvoip_core::Orchestrator`, and prints normalized
//! `Event`s as they arrive (translated from SIP's `api::Event`).

use rvoip_core::{Config, Orchestrator, Transport};
use rvoip_sip::api::unified::{Config as SipConfig, UnifiedCoordinator};
use rvoip_sip::SipAdapter;
use std::net::SocketAddr;

#[derive(Debug)]
struct Args {
    bind: SocketAddr,
}

impl Args {
    fn parse() -> Self {
        let mut bind = "127.0.0.1:5072".parse().unwrap();
        let mut iter = std::env::args().skip(1);
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--bind" => bind = iter.next().unwrap().parse().unwrap(),
                "--help" | "-h" => {
                    eprintln!("usage: sip_only_orchestrator [--bind ADDR]");
                    std::process::exit(0);
                }
                other => {
                    eprintln!("unknown arg: {other}");
                    std::process::exit(2);
                }
            }
        }
        Self { bind }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,rvoip_sip_dialog=warn".into()),
        )
        .init();

    let args = Args::parse();
    println!("[orchestrator] bind={}", args.bind);

    // 1. Build the SIP coordinator (proven api::UnifiedCoordinator).
    let coordinator = UnifiedCoordinator::new(SipConfig::on(
        "rvoip-orchestrator",
        args.bind.ip(),
        args.bind.port(),
    ))
    .await?;

    // 2. Wrap it in the cross-transport adapter.
    let adapter = SipAdapter::new(coordinator).await?;

    // 3. Build the rvoip-core Orchestrator and register the adapter. Future
    //    rvoip-webrtc / rvoip-quic adapters register against this same
    //    handle — consumers see no reshape when transports are added.
    let orchestrator = Orchestrator::new(Config::default());
    orchestrator.register(adapter)?;

    // Confirm the adapter is reachable.
    let registered = orchestrator
        .adapter(Transport::Sip)
        .expect("SipAdapter just registered");
    println!(
        "[orchestrator] registered adapter: transport={:?} kind={:?}",
        registered.transport(),
        registered.kind()
    );

    // 4. Subscribe to the cross-transport event bus. Every adapter event
    //    (incoming connection, connected, ended, failed) lands here as a
    //    normalized `rvoip_core::Event`.
    let mut events = orchestrator.subscribe_events();
    println!("[orchestrator] ready — waiting for events (Ctrl-C to quit)");

    while let Ok(event) = events.recv().await {
        println!("[orchestrator] {event:?}");
    }

    Ok(())
}
