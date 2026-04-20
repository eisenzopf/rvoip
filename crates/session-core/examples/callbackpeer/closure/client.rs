//! Test callers for the closure gatekeeper server.
//!
//! Run standalone:  cargo run -p rvoip-session-core --example callbackpeer_closure_client
//! Or with server:  ./examples/callbackpeer/closure/run.sh

use rvoip_session_core::{Config, StreamPeer};
use tokio::time::{sleep, timeout, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()))
        .init();

    // Caller 1: "friend@" URI — should be accepted
    let mut friend = StreamPeer::with_config(Config::local("friend", 5061)).await?;
    println!("friend@ calling (should be accepted)...");
    let h = friend.call("sip:gatekeeper@127.0.0.1:5060").await?;
    let _ = timeout(Duration::from_secs(3), friend.wait_for_answered(h.id())).await;
    sleep(Duration::from_secs(2)).await;
    let _ = h.hangup().await;
    let _ = timeout(Duration::from_secs(2), friend.wait_for_ended(h.id())).await;

    sleep(Duration::from_secs(1)).await;

    // Caller 2: "stranger@" URI — should be rejected
    let mut stranger = StreamPeer::with_config(Config::local("stranger", 5062)).await?;
    println!("stranger@ calling (should be rejected)...");
    let h = stranger.call("sip:gatekeeper@127.0.0.1:5060").await?;
    // Rejection usually arrives in <1s
    let _ = timeout(Duration::from_secs(2), stranger.wait_for_ended(h.id())).await;

    println!("Done.");

    std::process::exit(0);
}
