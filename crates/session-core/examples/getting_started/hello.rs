//! Minimal example — make and receive a SIP call with Endpoint.
//!
//!   cargo run --example hello

use rvoip_session_core::{Config, Endpoint, EndpointProfile};
use tokio::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // --- Bob: receive the call ---
    tokio::spawn(async {
        let mut bob = Endpoint::builder()
            .name("bob")
            .profile(EndpointProfile::Custom(Config::local("bob", 5061)))
            .build()
            .await
            .unwrap();
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
    let alice = Endpoint::builder()
        .name("alice")
        .profile(EndpointProfile::Custom(Config::local("alice", 5060)))
        .build()
        .await?;
    println!("[ALICE] Calling bob...");
    let handle = alice.call("sip:bob@127.0.0.1:5061").await?;
    handle
        .wait_for_answered(Some(Duration::from_secs(10)))
        .await?;
    println!("[ALICE] Connected! Hanging up in 3 seconds...");

    tokio::time::sleep(Duration::from_secs(3)).await;
    handle.hangup().await?;
    handle.wait_for_end(Some(Duration::from_secs(5))).await?;
    println!("[ALICE] Done.");
    Ok(())
}
