//! INTERFACE_DESIGN.md §16.4 — Full Thelve-shape gateway sketch.
//!
//! Cargo feature: `full`. Workers connect via UCTP; customers call
//! in via SIP/PSTN or via WebRTC widgets; AI agents attach
//! in-process via the harness; vCons emit per Session; identity
//! is OAuth + DPoP for human workers and SIP Digest for legacy
//! devices.
//!
//! Run with:
//!
//! ```sh
//! cargo run --example full_thelve_shape \
//!   --features full --manifest-path crates/rvoip/Cargo.toml
//! ```
//!
//! Sketch: the real gateway wires substrate + interop adapters from
//! every feature-gated re-export, plus an IdentityProviderChain,
//! plus the rvoip-harness AI runtime, plus a real `ConversationStore`
//! and `VconStore`. This binary just demonstrates the import shape
//! and event loop.

use rvoip::{Config, Orchestrator};
use tracing::Level;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    let orchestrator = Orchestrator::new(Config::default());
    let mut events = orchestrator.subscribe_events();

    tracing::info!("rvoip full Thelve-shape sketch — all features compiled in");
    tracing::info!(
        "wire SIP / WebRTC / UCTP adapters + harness providers + identity \
         chain + conversation/vcon stores here"
    );

    // Skeleton command-handler loop matching the §16.4 sketch:
    // workforce-orchestration commands (AttachAi, BridgeWorker,
    // TransferToHuman) come in from `my_thelve` and the orchestrator
    // dispatches. Here we just pump events for ~2s.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(200), events.recv()).await {
            Ok(Ok(event)) => {
                tracing::info!(?event, "thelve gateway event");
            }
            _ => continue,
        }
    }

    tracing::info!("exiting (sketch only)");
    Ok(())
}
