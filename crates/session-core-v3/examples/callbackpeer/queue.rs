//! Call center queue using QueueHandler.
//!
//!   cargo run --example callbackpeer_queue
//!
//! Incoming calls are placed in a queue. A simulated "agent" task picks them
//! up after a short delay, demonstrating the deferred-accept pattern used in
//! call centers.

use std::time::Duration;

use rvoip_session_core_v3::api::handlers::QueueHandler;
use rvoip_session_core_v3::{CallbackPeer, Config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("rvoip_session_core_v3=info")
        .init();

    // Create queue with capacity 50, 30-second ringing timeout
    let (handler, mut rx) = QueueHandler::new(50, Duration::from_secs(30));

    // Simulated agent — picks up queued calls after a 2-second delay
    tokio::spawn(async move {
        while let Some(guard) = rx.recv().await {
            println!(
                "[AGENT] Call {} in queue, answering in 2 seconds...",
                guard.call_id()
            );
            tokio::time::sleep(Duration::from_secs(2)).await;

            match guard.accept().await {
                Ok(handle) => {
                    println!("[AGENT] Answered call {}", handle.id());
                    // In a real app, you'd handle the call here
                }
                Err(e) => {
                    println!("[AGENT] Failed to accept: {}", e);
                }
            }
        }
    });

    println!("Call queue server on port 5060...");
    println!("  Calls ring for up to 30 seconds");
    println!("  Agent picks up after ~2 second delay");

    let peer = CallbackPeer::new(handler, Config::local("queue", 5060)).await?;
    peer.run().await?;
    Ok(())
}
