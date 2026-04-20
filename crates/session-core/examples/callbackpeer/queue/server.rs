//! Call center queue server with agent task.
//!
//! Run standalone:  cargo run -p rvoip-session-core --example callbackpeer_queue_server
//! Or with client:  ./examples/callbackpeer/queue/run.sh

use std::time::Duration;

use rvoip_session_core::api::handlers::QueueHandler;
use rvoip_session_core::{CallbackPeer, Config};
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()))
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

    let peer = CallbackPeer::new(handler, Config::local("queue", 5060)).await?;

    println!("Listening on port 5060 (queue server)...");
    println!("  Calls ring for up to 30 seconds");
    println!("  Agent picks up after ~2 second delay");
    println!("Press Ctrl+C to stop.");

    tokio::select! {
        res = peer.run() => res?,
        _ = tokio::signal::ctrl_c() => println!("\nShutting down."),
    }

    std::process::exit(0);
}
