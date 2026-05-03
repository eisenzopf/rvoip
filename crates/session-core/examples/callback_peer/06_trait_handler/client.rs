//! Test caller for the custom handler server.
//!
//! Run standalone:  cargo run -p rvoip-session-core --example callbackpeer_custom_client
//! Or with server:  ./examples/callbackpeer/custom/run.sh

use rvoip_session_core::{Config, StreamPeer};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()),
        )
        .init();

    let mut caller = StreamPeer::with_config(Config::local("caller", 5061)).await?;

    println!("Calling custom handler...");
    let handle = caller.call("sip:custom@127.0.0.1:5060").await?;
    caller.wait_for_answered(handle.id()).await?;

    // Send some DTMF to trigger on_dtmf
    for digit in ['5', '#'] {
        sleep(Duration::from_secs(1)).await;
        println!("Sending DTMF '{}'", digit);
        handle.send_dtmf(digit).await.ok();
    }

    sleep(Duration::from_secs(2)).await;
    println!("Hanging up...");
    handle.hangup().await.ok();
    caller.wait_for_ended(handle.id()).await.ok();

    println!("Done.");

    std::process::exit(0);
}
