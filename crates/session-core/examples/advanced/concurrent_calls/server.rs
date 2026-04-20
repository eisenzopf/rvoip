//! Concurrent calls answerer (StreamPeer).
//!
//! Run standalone:  cargo run -p rvoip-session-core --example advanced_concurrent_calls_server
//! Or with client:  ./examples/advanced/concurrent_calls/run.sh

use rvoip_session_core::{Config, StreamPeer};
use tokio::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()))
        .init();

    const NUM_CALLERS: usize = 5;

    let mut peer = StreamPeer::with_config(Config {
        media_port_start: 20000,
        media_port_end: 20200,
        ..Config::local("answerer", 6000)
    })
    .await?;

    println!("Listening on port 6000...");
    println!("Press Ctrl+C to stop.");

    let mut handles = Vec::new();

    for _ in 0..NUM_CALLERS {
        tokio::select! {
            result = async {
                match tokio::time::timeout(Duration::from_secs(10), peer.wait_for_incoming()).await {
                    Ok(Ok(incoming)) => {
                        println!("[ANSWERER] Accepting call from {}", incoming.from);
                        match incoming.accept().await {
                            Ok(h) => Some(h),
                            Err(e) => {
                                println!("[ANSWERER] Accept failed: {}", e);
                                None
                            }
                        }
                    }
                    _ => None,
                }
            } => {
                if let Some(h) = result {
                    handles.push(h);
                } else {
                    break;
                }
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\nShutting down.");
                std::process::exit(0);
            }
        }
    }

    println!("[ANSWERER] {} calls active, waiting for them to end...", handles.len());
    for h in &handles {
        h.wait_for_end(Some(Duration::from_secs(10))).await.ok();
    }
    println!("[ANSWERER] All calls ended. {} total calls handled.", handles.len());

    std::process::exit(0);
}
