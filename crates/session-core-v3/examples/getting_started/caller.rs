//! Minimal example — make a SIP call.
//!
//! Start the receiver first (`hello_receiver`), then run this:
//!   cargo run --example hello_caller

use rvoip_session_core_v3::{StreamPeer, Config};
use tokio::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut peer = StreamPeer::with_config(Config::local("alice", 5060)).await?;

    // Give the receiver time to start
    tokio::time::sleep(Duration::from_secs(1)).await;

    println!("Calling bob...");
    let handle = peer.call("sip:bob@127.0.0.1:5061").await?;
    peer.wait_for_answered(handle.id()).await?;
    println!("Connected! Hanging up in 5 seconds...");

    tokio::time::sleep(Duration::from_secs(5)).await;
    handle.hangup().await?;
    peer.wait_for_ended(handle.id()).await?;
    println!("Done.");
    Ok(())
}
