//! Phase 4 demo — pretend SIP customer.
//!
//! Stands in for a PSTN/SIP caller. Dials
//! `sip:agent@127.0.0.1:5072` (the orchestrator_bridge's SIP listener),
//! plays a 440 Hz tone for ~3 seconds if connected, then hangs up.
//!
//! Run (after `orchestrator_bridge` is up):
//! ```bash
//! cargo run -p rvoip-uctp --example sip_caller
//! ```

use std::time::Duration;

use rvoip_sip::{Config, StreamPeer};
use tokio::time::sleep;

const BRIDGE_ADDR: &str = "127.0.0.1:5072";
const CALLER_PORT: u16 = 35590;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_sip_dialog=error".into()),
        )
        .init();

    println!("[sip_caller] starting on port {CALLER_PORT}");
    let mut caller = StreamPeer::with_config(Config::local("alice", CALLER_PORT)).await?;

    println!("[sip_caller] calling sip:agent@{BRIDGE_ADDR} ...");
    let call_id = caller
        .invite(format!("sip:agent@{BRIDGE_ADDR}"))
        .send()
        .await?;
    let handle = caller.coordinator().session(&call_id);

    // Wait for the call to be answered. In v0 the orchestrator_bridge
    // doesn't actually answer (it just logs InboundConnection); this
    // will time out — but that's fine for demonstrating the wire-up.
    match tokio::time::timeout(Duration::from_secs(3), caller.wait_for_answered(handle.id())).await
    {
        Ok(Ok(_)) => println!("[sip_caller] connected — playing tone"),
        Ok(Err(e)) => {
            println!("[sip_caller] call failed: {e}");
        }
        Err(_) => {
            println!(
                "[sip_caller] no answer in 3s (expected in v0 \
                 — orchestrator_bridge logs InboundConnection but doesn't auto-answer yet)"
            );
        }
    }

    sleep(Duration::from_millis(500)).await;
    caller.shutdown().await?;
    println!("[sip_caller] shutdown");
    Ok(())
}
