//! Hold and resume a call.
//!
//!   cargo run --example streampeer_hold_resume

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

    // --- Demo: hold and resume ---
    let mut peer = StreamPeer::with_config(Config::local("holder", 5061)).await?;

    println!("Calling server...");
    let handle = peer.call("sip:server@127.0.0.1:5060").await?;
    peer.wait_for_answered(handle.id()).await?;
    println!("Connected! Active: {}", handle.is_active().await);

    println!("Putting call on hold...");
    handle.hold().await?;
    sleep(Duration::from_millis(500)).await;
    println!("On hold: {}", handle.is_on_hold().await);

    sleep(Duration::from_secs(3)).await;

    println!("Resuming call...");
    handle.resume().await?;
    sleep(Duration::from_millis(500)).await;
    println!("Active again: {}", handle.is_active().await);

    sleep(Duration::from_secs(1)).await;
    handle.hangup().await?;
    peer.wait_for_ended(handle.id()).await?;
    println!("Done.");
    Ok(())
}
