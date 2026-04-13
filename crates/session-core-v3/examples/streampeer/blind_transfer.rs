//! Blind transfer (REFER) between three peers in a single process.
//!
//!   cargo run --example streampeer_blind_transfer
//!
//! Flow:
//!   1. Alice calls Bob
//!   2. They talk for 2 seconds
//!   3. Bob transfers Alice to Charlie (sends REFER)
//!   4. Alice calls Charlie, they talk for 2 seconds
//!   5. Alice hangs up

use rvoip_session_core_v3::{Config, Event, StreamPeer};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("rvoip_session_core_v3=info")
        .init();

    // --- Charlie (transfer target) ---
    let charlie_task = tokio::spawn(async {
        let mut charlie = StreamPeer::with_config(Config::local("charlie", 5062)).await?;
        println!("[CHARLIE] Waiting for transferred call...");

        let incoming = charlie.wait_for_incoming().await?;
        println!("[CHARLIE] Incoming call from {}", incoming.from);
        let handle = incoming.accept().await?;
        println!("[CHARLIE] Answered!");

        handle.wait_for_end(Some(Duration::from_secs(30))).await.ok();
        println!("[CHARLIE] Call ended.");
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    });

    // --- Bob (transferor) ---
    let bob_task = tokio::spawn(async {
        let mut bob = StreamPeer::with_config(Config::local("bob", 5061)).await?;
        println!("[BOB] Waiting for call...");

        let incoming = bob.wait_for_incoming().await?;
        println!("[BOB] Call from {}", incoming.from);
        let handle = incoming.accept().await?;

        // Talk for 2 seconds, then transfer
        sleep(Duration::from_secs(2)).await;
        println!("[BOB] Transferring Alice to Charlie...");
        handle.transfer_blind("sip:charlie@127.0.0.1:5062").await?;

        sleep(Duration::from_secs(1)).await;
        handle.hangup().await?;
        bob.wait_for_ended(handle.id()).await?;
        println!("[BOB] Done.");
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    });

    // Give Bob and Charlie time to start
    sleep(Duration::from_secs(1)).await;

    // --- Alice (caller, gets transferred) ---
    let mut alice = StreamPeer::with_config(Config::local("alice", 5060)).await?;

    println!("[ALICE] Calling Bob...");
    let handle = alice.call("sip:bob@127.0.0.1:5061").await?;
    alice.wait_for_answered(handle.id()).await?;
    println!("[ALICE] Connected to Bob!");

    // Wait for REFER from Bob
    println!("[ALICE] Waiting for transfer...");
    let mut events = alice.control().subscribe_events().await?;
    loop {
        match events.next().await {
            Some(Event::ReferReceived { refer_to, .. }) => {
                println!("[ALICE] Got REFER to {}", refer_to);
                // Hang up with Bob and call Charlie
                handle.hangup().await?;
                alice.wait_for_ended(handle.id()).await?;

                println!("[ALICE] Calling Charlie...");
                let charlie_handle = alice.call(&refer_to).await?;
                alice.wait_for_answered(charlie_handle.id()).await?;
                println!("[ALICE] Connected to Charlie!");

                sleep(Duration::from_secs(2)).await;
                charlie_handle.hangup().await?;
                alice.wait_for_ended(charlie_handle.id()).await?;
                break;
            }
            Some(Event::CallEnded { .. }) => {
                println!("[ALICE] Call ended before transfer");
                break;
            }
            None => break,
            _ => {}
        }
    }

    println!("[ALICE] Done.");
    bob_task.await.unwrap().unwrap();
    charlie_task.await.unwrap().unwrap();
    println!("All peers finished.");
    Ok(())
}
