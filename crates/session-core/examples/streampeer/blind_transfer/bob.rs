//! Transferor (Bob) — accepts call from Alice, then transfers her to Charlie.
//!
//! Run standalone:  cargo run -p rvoip-session-core --example streampeer_blind_transfer_bob
//! Or with others:  ./examples/streampeer/blind_transfer/run.sh

use rvoip_session_core::{Config, Event, StreamPeer};
use tokio::time::{sleep, Duration};

fn env_port(key: &str, default: u16) -> u16 {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()),
        )
        .init();

    let bob_port = env_port("BOB_PORT", 5061);
    let charlie_port = env_port("CHARLIE_PORT", 5062);

    let mut bob = StreamPeer::with_config(Config::local("bob", bob_port)).await?;
    println!("[BOB] Waiting for call...");

    let incoming = bob.wait_for_incoming().await?;
    println!("[BOB] Call from {}", incoming.from);
    let handle = incoming.accept().await?;

    // Talk for 2 seconds, then transfer
    sleep(Duration::from_secs(2)).await;
    println!("[BOB] Transferring Alice to Charlie...");
    let transfer_outcome = handle
        .transfer_blind_and_wait(
            &format!("sip:charlie@127.0.0.1:{}", charlie_port),
            Some(Duration::from_secs(10)),
        )
        .await?;
    match transfer_outcome {
        Event::TransferCompleted { .. } => {}
        Event::TransferFailed {
            status_code,
            reason,
            ..
        } => {
            return Err(format!("transfer failed with {status_code}: {reason}").into());
        }
        other => return Err(format!("unexpected transfer outcome: {other:?}").into()),
    }

    sleep(Duration::from_secs(1)).await;
    handle.hangup_and_wait(Some(Duration::from_secs(8))).await?;
    println!("[BOB] Done.");

    std::process::exit(0);
}
