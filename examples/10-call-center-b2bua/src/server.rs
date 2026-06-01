//! Mini call-center **B2BUA server**.
//!
//! Listens for inbound customer calls and, for each one, originates a second
//! call leg to an available agent and bridges the two through media-core. This
//! is the back-to-back user agent (B2BUA) pattern — the server is a full UA on
//! both legs, so it can route, hide topology, and bridge media.
//!
//! Built on [`UnifiedCoordinator`] (raw call control) plus the
//! [`server::b2bua::SipB2bua`] helper, which wires the canonical
//! inbound → originate → bridge flow in one `handle_inbound` call. Agents are
//! chosen round-robin to demonstrate routing.
//!
//! Run the whole call center with `./run_demo.sh`.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use rvoip_sip::server::b2bua::SipB2bua;
use rvoip_sip::{Config, Event, UnifiedCoordinator};

#[derive(Parser, Debug)]
#[command(about = "Call-center B2BUA — bridges each customer to an agent")]
struct Args {
    /// Address the support line binds to.
    #[arg(long, default_value = "127.0.0.1:5070")]
    bind: SocketAddr,
    /// `From` identity the B2BUA presents on the outbound (agent) leg.
    #[arg(long, default_value = "sip:b2bua@127.0.0.1:5070")]
    from: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_sip_dialog=error".into()),
        )
        .init();
    let args = Args::parse();

    // The agent pool. A real deployment would resolve these from a registrar
    // (presence/availability); here they are fixed endpoints chosen round-robin.
    let agents = Arc::new(vec![
        "sip:agent@127.0.0.1:5071".to_string(),
        "sip:agent@127.0.0.1:5072".to_string(),
    ]);
    let next = Arc::new(AtomicUsize::new(0));

    let coordinator =
        UnifiedCoordinator::new(Config::on("call-center", args.bind.ip(), args.bind.port())).await?;
    let b2bua = SipB2bua::new(coordinator.clone());
    let mut events = coordinator.events().await?;

    println!(
        "[call-center] support line on {} — {} agents in the pool",
        args.bind,
        agents.len()
    );

    while let Some(event) = events.next().await {
        match event {
            Event::IncomingCall {
                call_id, from, to, ..
            } => {
                let idx = next.fetch_add(1, Ordering::Relaxed) % agents.len();
                let agent_uri = agents[idx].clone();
                println!(
                    "[call-center] customer {from} → {to}: routing to {agent_uri} (session={call_id})"
                );
                let from_uri = args.from.clone();
                let b2bua = b2bua.clone();
                tokio::spawn(async move {
                    match b2bua.handle_inbound(&from_uri, &call_id, &agent_uri).await {
                        Ok(_handle) => {
                            println!("[call-center] ✅ bridged {call_id} ↔ {agent_uri}");
                            // Hold the BridgeHandle so the bridge stays up until
                            // a leg drops. A production B2BUA would watch both
                            // legs for CallEnded and tear down explicitly.
                            tokio::time::sleep(Duration::from_secs(3600)).await;
                        }
                        Err(err) => {
                            eprintln!("[call-center] bridge failed for {call_id}: {err}");
                        }
                    }
                });
            }
            Event::CallEnded { call_id, reason } => {
                println!("[call-center] session {call_id} ended: {reason}");
            }
            Event::CallFailed {
                call_id,
                status_code,
                reason,
            } => {
                println!("[call-center] session {call_id} failed: {status_code} {reason}");
            }
            _ => {}
        }
    }

    Ok(())
}
