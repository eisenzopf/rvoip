//! Multiple concurrent calls in a single process.
//!
//!   cargo run --example advanced_concurrent_calls
//!
//! One answerer accepts calls from 5 callers running in parallel.
//! Each call lasts 3 seconds before hanging up.

use rvoip_session_core_v3::{Config, StreamPeer};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("rvoip_session_core_v3=info")
        .init();

    const NUM_CALLERS: usize = 5;

    // Answerer task
    let answerer = tokio::spawn(async move {
        let mut peer = StreamPeer::with_config(Config {
            media_port_start: 20000,
            media_port_end: 20200,
            ..Config::local("answerer", 6000)
        })
        .await?;

        println!("[ANSWERER] Ready on port 6000");
        let mut handles = Vec::new();

        for _ in 0..NUM_CALLERS {
            match tokio::time::timeout(Duration::from_secs(10), peer.wait_for_incoming()).await {
                Ok(Ok(incoming)) => {
                    println!("[ANSWERER] Accepting call from {}", incoming.from);
                    match incoming.accept().await {
                        Ok(h) => handles.push(h),
                        Err(e) => println!("[ANSWERER] Accept failed: {}", e),
                    }
                }
                _ => break,
            }
        }

        println!("[ANSWERER] {} calls active, waiting for them to end...", handles.len());
        for h in &handles {
            h.wait_for_end(Some(Duration::from_secs(10))).await.ok();
        }
        println!("[ANSWERER] All calls ended.");
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    });

    // Give answerer time to start
    sleep(Duration::from_secs(1)).await;

    // Spawn callers
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

    answerer.await.unwrap().unwrap();
    println!("All done. {} calls completed.", NUM_CALLERS);
    Ok(())
}
