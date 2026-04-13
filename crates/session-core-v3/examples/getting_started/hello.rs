//! Minimal example — make and receive a SIP call.
//!
//!   cargo run --example hello

use rvoip_session_core_v3::{Config, StreamPeer};
use tokio::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // --- Bob: receive the call ---
    tokio::spawn(async {
        let mut bob = StreamPeer::with_config(Config::local("bob", 5061)).await.unwrap();
        println!("[BOB] Waiting for call...");
        let incoming = bob.wait_for_incoming().await.unwrap();
        println!("[BOB] Call from {}", incoming.from);
        let handle = incoming.accept().await.unwrap();
        println!("[BOB] Answered!");
        handle.wait_for_end(None).await.ok();
        println!("[BOB] Call ended.");
    });
    tokio::time::sleep(Duration::from_secs(1)).await;

    // --- Alice: make the call ---
    let mut alice = StreamPeer::with_config(Config::local("alice", 5060)).await?;
    println!("[ALICE] Calling bob...");
    let handle = alice.call("sip:bob@127.0.0.1:5061").await?;
    alice.wait_for_answered(handle.id()).await?;
    println!("[ALICE] Connected! Hanging up in 3 seconds...");

    tokio::time::sleep(Duration::from_secs(3)).await;
    handle.hangup().await?;
    alice.wait_for_ended(handle.id()).await?;
    println!("[ALICE] Done.");
    Ok(())
}
