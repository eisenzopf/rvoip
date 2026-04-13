//! Hold and resume a call.
//!
//!   cargo run --example streampeer_hold_resume
//!
//! Makes a call, puts it on hold for 3 seconds, then resumes.
//! Pair with `callbackpeer_auto_answer` or any SIP endpoint.

use rvoip_session_core_v3::{Config, StreamPeer};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("rvoip_session_core_v3=info")
        .init();

    let mut peer = StreamPeer::with_config(Config::local("holder", 5061)).await?;

    println!("Calling sip:server@127.0.0.1:5060...");
    let handle = peer.call("sip:server@127.0.0.1:5060").await?;
    peer.wait_for_answered(handle.id()).await?;
    println!("Connected! Active: {}", handle.is_active().await);

    // Put on hold
    println!("Putting call on hold...");
    handle.hold().await?;
    sleep(Duration::from_millis(500)).await;
    println!("On hold: {}", handle.is_on_hold().await);

    sleep(Duration::from_secs(3)).await;

    // Resume
    println!("Resuming call...");
    handle.resume().await?;
    sleep(Duration::from_millis(500)).await;
    println!("Active again: {}", handle.is_active().await);

    sleep(Duration::from_secs(2)).await;
    handle.hangup().await?;
    peer.wait_for_ended(handle.id()).await?;
    println!("Done.");
    Ok(())
}
