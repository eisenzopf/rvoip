//! Minimal example — receive a SIP call.
//!
//! Start this first, then run `hello_caller`:
//!   cargo run --example hello_receiver

use rvoip_session_core_v3::{StreamPeer, Config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = StreamPeer::with_config(Config::local("bob", 5061)).await?;

    println!("Waiting for incoming call...");
    let incoming = peer.wait_for_incoming().await?;
    println!("Call from {}", incoming.from);

    let handle = incoming.accept().await?;
    println!("Answered! Waiting for caller to hang up...");

    handle.wait_for_end(None).await?;
    println!("Call ended.");
    Ok(())
}
