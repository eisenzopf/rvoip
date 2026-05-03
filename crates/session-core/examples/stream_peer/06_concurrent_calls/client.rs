//! Spawns 5 concurrent callers to the answerer.
//!
//! Run standalone:  cargo run -p rvoip-session-core --example advanced_concurrent_calls_client
//! Or with server:  ./examples/advanced/concurrent_calls/run.sh

use rvoip_session_core::{Config, StreamPeer};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()),
        )
        .init();

    const NUM_CALLERS: usize = 5;

    let mut caller_tasks = Vec::new();
    for id in 0..NUM_CALLERS {
        let task = tokio::spawn(async move {
            let port = 6001 + id as u16;
            let mut peer = StreamPeer::with_config(Config {
                media_port_start: 21000 + (id * 100) as u16,
                media_port_end: 21100 + (id * 100) as u16,
                ..Config::local(&format!("caller{}", id), port)
            })
            .await?;

            println!("[CALLER-{}] Calling answerer...", id);
            let handle = peer.call("sip:answerer@127.0.0.1:6000").await?;
            peer.wait_for_answered(handle.id()).await?;
            println!("[CALLER-{}] Connected!", id);

            sleep(Duration::from_secs(3)).await;

            handle.hangup().await?;
            peer.wait_for_ended(handle.id()).await?;
            println!("[CALLER-{}] Done.", id);
            Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
        });
        caller_tasks.push(task);
        // Stagger callers slightly
        sleep(Duration::from_millis(200)).await;
    }

    // Wait for all callers
    for (i, task) in caller_tasks.into_iter().enumerate() {
        match task.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => println!("[CALLER-{}] Error: {}", i, e),
            Err(e) => println!("[CALLER-{}] Task panicked: {}", i, e),
        }
    }

    println!("All {} callers finished.", NUM_CALLERS);

    std::process::exit(0);
}
