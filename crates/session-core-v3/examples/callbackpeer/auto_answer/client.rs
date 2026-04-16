//! Test caller for the auto-answer server.
//!
//! Run standalone:  cargo run -p rvoip-session-core-v3 --example callbackpeer_auto_answer_client
//! Or with server:  ./examples/callbackpeer/auto_answer/run.sh

use rvoip_session_core_v3::{Config, StreamPeer};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()))
        .init();

    let mut caller = StreamPeer::with_config(Config::local("caller", 5061)).await?;

    println!("Calling auto-answer server...");
    let handle = caller.call("sip:server@127.0.0.1:5060").await?;
    caller.wait_for_answered(handle.id()).await?;
    println!("Connected! Hanging up in 3 seconds...");

    sleep(Duration::from_secs(3)).await;
    handle.hangup().await?;
    caller.wait_for_ended(handle.id()).await?;
    println!("Done.");

    std::process::exit(0);
}
