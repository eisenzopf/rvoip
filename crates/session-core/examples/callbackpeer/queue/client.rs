//! Test callers for the queue server.
//!
//! Run standalone:  cargo run -p rvoip-session-core --example callbackpeer_queue_client
//! Or with server:  ./examples/callbackpeer/queue/run.sh

use rvoip_session_core::{Config, StreamPeer};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()),
        )
        .init();

    for i in 0..3 {
        let port = 5061 + i as u16;
        let mut caller =
            StreamPeer::with_config(Config::local(&format!("caller{}", i), port)).await?;
        println!("Caller {} dialing in...", i);
        let h = caller.call("sip:queue@127.0.0.1:5060").await?;
        caller.wait_for_answered(h.id()).await.ok();
        sleep(Duration::from_secs(3)).await;
        h.hangup().await.ok();
        caller.wait_for_ended(h.id()).await.ok();
        sleep(Duration::from_millis(500)).await;
    }

    println!("Done.");

    std::process::exit(0);
}
