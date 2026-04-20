//! Send DTMF digits during a call.
//!
//! Run standalone:  cargo run -p rvoip-session-core --example streampeer_dtmf_client
//! Or with server:  ./examples/streampeer/dtmf/run.sh

use rvoip_session_core::{Config, StreamPeer};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()))
        .init();

    let mut peer = StreamPeer::with_config(Config::local("dtmf_sender", 5061)).await?;

    println!("Calling server...");
    let handle = peer.call("sip:server@127.0.0.1:5060").await?;
    peer.wait_for_answered(handle.id()).await?;
    println!("Connected!");

    for digit in ['1', '2', '3', '4', '#'] {
        sleep(Duration::from_millis(500)).await;
        println!("Sending DTMF: {}", digit);
        handle.send_dtmf(digit).await?;
    }

    sleep(Duration::from_secs(1)).await;
    handle.hangup().await?;
    peer.wait_for_ended(handle.id()).await?;
    println!("Done.");

    std::process::exit(0);
}
