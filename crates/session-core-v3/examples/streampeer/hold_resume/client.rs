//! Hold and resume a call.
//!
//! Run standalone:  cargo run -p rvoip-session-core-v3 --example streampeer_hold_resume_client
//! Or with server:  ./examples/streampeer/hold_resume/run.sh

use rvoip_session_core_v3::{Config, StreamPeer};
use tokio::time::{sleep, timeout, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()))
        .init();

    let mut peer = StreamPeer::with_config(Config::local("holder", 5061)).await?;

    println!("Calling server...");
    let handle = peer.call("sip:server@127.0.0.1:5060").await?;
    peer.wait_for_answered(handle.id()).await?;
    println!("Connected! Active: {}", handle.is_active().await);

    println!("Putting call on hold...");
    let _ = handle.hold().await;
    sleep(Duration::from_millis(500)).await;
    println!("On hold: {}", handle.is_on_hold().await);

    sleep(Duration::from_secs(2)).await;

    println!("Resuming call...");
    let _ = handle.resume().await;
    sleep(Duration::from_millis(500)).await;
    println!("Active again: {}", handle.is_active().await);

    sleep(Duration::from_secs(1)).await;
    let _ = handle.hangup().await;
    // wait_for_ended with timeout — session may already be cleaned up
    let _ = timeout(Duration::from_secs(3), peer.wait_for_ended(handle.id())).await;
    println!("Done.");

    std::process::exit(0);
}
