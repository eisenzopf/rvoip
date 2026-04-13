//! Send DTMF digits during a call.
//!
//!   cargo run --example streampeer_dtmf

use rvoip_session_core_v3::{CallbackPeer, Config, StreamPeer};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("rvoip_session_core_v3=info")
        .init();

    // --- Background: auto-answer server ---
    tokio::spawn(async {
        let peer = CallbackPeer::with_auto_answer(Config::local("server", 5060)).await.unwrap();
        peer.run().await.ok();
    });
    sleep(Duration::from_secs(1)).await;

    // --- Demo: send DTMF digits ---
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
    Ok(())
}
