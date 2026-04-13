//! Send DTMF digits during a call.
//!
//!   cargo run --example streampeer_dtmf
//!
//! Makes a call and sends DTMF digits "1 2 3 4 #" with pauses between them.
//! Pair with `callbackpeer_ivr` or any SIP endpoint.

use rvoip_session_core_v3::{Config, StreamPeer};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("rvoip_session_core_v3=info")
        .init();

    let mut peer = StreamPeer::with_config(Config::local("dtmf_sender", 5061)).await?;

    println!("Calling sip:ivr@127.0.0.1:5060...");
    let handle = peer.call("sip:ivr@127.0.0.1:5060").await?;
    peer.wait_for_answered(handle.id()).await?;
    println!("Connected!");

    // Send DTMF digits with pauses
    for digit in ['1', '2', '3', '4', '#'] {
        sleep(Duration::from_millis(500)).await;
        println!("Sending DTMF: {}", digit);
        handle.send_dtmf(digit).await?;
    }

    sleep(Duration::from_secs(2)).await;
    handle.hangup().await?;
    peer.wait_for_ended(handle.id()).await?;
    println!("Done.");
    Ok(())
}
