//! Call center queue using QueueHandler.
//!
//!   cargo run --example callbackpeer_queue
//!
//! Incoming calls are placed in a queue. A simulated "agent" task picks them
//! up after a short delay, demonstrating the deferred-accept pattern used in
//! call centers.

use std::time::Duration;

use rvoip_session_core_v3::api::handlers::QueueHandler;
use rvoip_session_core_v3::{CallbackPeer, Config, StreamPeer};
use tokio::time::sleep;

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
            sleep(Duration::from_secs(2)).await;

            match guard.accept().await {
                Ok(handle) => println!("[AGENT] Answered call {}", handle.id()),
                Err(e) => println!("[AGENT] Failed to accept: {}", e),
            }
        }
    });

    // --- Background: 3 test callers with staggered timing ---
    tokio::spawn(async {
        sleep(Duration::from_secs(1)).await;

        for i in 0..3 {
            let port = 5061 + i as u16;
            let mut caller =
                StreamPeer::with_config(Config::local(&format!("caller{}", i), port))
                    .await
                    .unwrap();
            println!("[TEST] Caller {} dialing in...", i);
            let h = caller.call("sip:queue@127.0.0.1:5060").await.unwrap();
            caller.wait_for_answered(h.id()).await.ok();
            sleep(Duration::from_secs(3)).await;
            h.hangup().await.ok();
            caller.wait_for_ended(h.id()).await.ok();
            sleep(Duration::from_millis(500)).await;
        }

        println!("[TEST] All callers done.");
        sleep(Duration::from_secs(1)).await;
        std::process::exit(0);
    });

    // --- Demo: queue server ---
    println!("Call queue server on port 5060...");
    println!("  Calls ring for up to 30 seconds");
    println!("  Agent picks up after ~2 second delay");

    let peer = CallbackPeer::new(handler, Config::local("queue", 5060)).await?;
    peer.run().await?;
    Ok(())
}
