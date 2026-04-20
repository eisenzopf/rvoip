//! Test caller for the IVR server.
//!
//! Run standalone:  cargo run -p rvoip-session-core --example callbackpeer_ivr_client
//! Or with server:  ./examples/callbackpeer/ivr/run.sh

use rvoip_session_core::{Config, StreamPeer};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()))
        .init();

    let mut caller = StreamPeer::with_config(Config::local("caller", 5061)).await?;

    println!("Calling IVR...");
    let handle = caller.call("sip:ivr@127.0.0.1:5060").await?;
    caller.wait_for_answered(handle.id()).await?;

    // Navigate the menu
    for digit in ['1', '2', '9'] {
        sleep(Duration::from_secs(1)).await;
        println!("Pressing '{}'", digit);
        handle.send_dtmf(digit).await.ok();
    }

    sleep(Duration::from_secs(2)).await;
    handle.hangup().await.ok();
    println!("Done.");

    std::process::exit(0);
}
