//! Basic StreamPeer call.
//!
//! Run with:
//!
//!   cargo run -p rvoip-session-core --example stream_peer_basic_call

use std::time::Duration;

use rvoip_session_core::{Config, Result, SessionError, StreamPeer};

#[tokio::main]
async fn main() -> Result<()> {
    let bob_task = tokio::spawn(async {
        let mut bob = StreamPeer::with_config(Config::local("bob", 5101)).await?;
        let incoming = bob.wait_for_incoming().await?;
        println!("[bob] incoming from {}", incoming.from);
        let call = incoming.accept().await?;
        call.wait_for_end(None).await?;
        bob.shutdown().await
    });

    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut alice = StreamPeer::with_config(Config::local("alice", 5100)).await?;
    let call = alice.call("sip:bob@127.0.0.1:5101").await?;
    alice.wait_for_answered(call.id()).await?;
    println!("[alice] connected as {}", call.id());

    tokio::time::sleep(Duration::from_secs(1)).await;
    call.hangup_and_wait(Some(Duration::from_secs(5))).await?;
    alice.shutdown().await?;

    bob_task
        .await
        .map_err(|err| SessionError::Other(err.to_string()))??;
    Ok(())
}
