//! Test callers for the routing server.
//!
//! Run standalone:  cargo run -p rvoip-session-core --example callbackpeer_routing_client
//! Or with server:  ./examples/callbackpeer/routing/run.sh

use rvoip_session_core::{Config, StreamPeer};
use tokio::time::{sleep, timeout, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()),
        )
        .init();

    for (i, target) in ["support", "spam", "unknown"].iter().enumerate() {
        let port = 5061 + i as u16;
        let mut caller =
            StreamPeer::with_config(Config::local(&format!("caller{}", i), port)).await?;
        println!("Calling {}@...", target);
        let h = caller
            .call(&format!("sip:{}@127.0.0.1:5060", target))
            .await?;

        // Wait up to 2s for the call to be answered (or rejected).
        let _ = timeout(Duration::from_secs(2), caller.wait_for_answered(h.id())).await;

        // Short dwell, then hang up. For rejected calls, hangup is a no-op.
        sleep(Duration::from_millis(500)).await;
        let _ = h.hangup().await;
        // Don't block on wait_for_ended — some calls may already be Failed.
        let _ = timeout(Duration::from_secs(2), caller.wait_for_ended(h.id())).await;

        sleep(Duration::from_millis(200)).await;
    }

    println!("Done.");

    std::process::exit(0);
}
