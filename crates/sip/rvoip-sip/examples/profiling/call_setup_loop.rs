//! Long-running INVITE → 200 OK → ACK → BYE loop, for CPU profiling.
//!
//! Same loopback shape as `benches/call_setup.rs` but runs continuously
//! for `RVOIP_PROFILE_DURATION` seconds (default 30) so `samply` /
//! `cargo-flamegraph` have enough samples. See
//! `crates/rvoip-sip/docs/PROFILING.md`.
//!
//! ```bash
//! cargo build --profile flamegraph -p rvoip-sip --example profiling_call_setup_loop
//! samply record target/flamegraph/examples/profiling_call_setup_loop
//! ```

use rvoip_sip::{Config, StreamPeer};
use std::env;
use std::time::{Duration, Instant};

const DEFAULT_DURATION_SECS: u64 = 30;
const SERVER_PORT: u16 = 45000;
const CLIENT_PORT: u16 = 45001;

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .init();

    let duration = Duration::from_secs(
        env::var("RVOIP_PROFILE_DURATION")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_DURATION_SECS),
    );

    let mut server = StreamPeer::with_config(Config {
        media_port_start: 45100,
        media_port_end: 45299,
        ..Config::local("profiling-server", SERVER_PORT)
    })
    .await?;

    let server_task = tokio::spawn(async move {
        loop {
            match tokio::time::timeout(Duration::from_secs(60), server.wait_for_incoming()).await {
                Ok(Ok(incoming)) => {
                    if let Ok(h) = incoming.accept().await {
                        tokio::spawn(async move {
                            let _ = h.wait_for_end(Some(Duration::from_secs(60))).await;
                        });
                    }
                }
                _ => break,
            }
        }
    });

    let mut client = StreamPeer::with_config(Config {
        media_port_start: 45300,
        media_port_end: 45499,
        ..Config::local("profiling-client", CLIENT_PORT)
    })
    .await?;
    let target = format!("sip:profiling-server@127.0.0.1:{}", SERVER_PORT);

    println!(
        "[call_setup_loop] running for {}s (override with RVOIP_PROFILE_DURATION)",
        duration.as_secs()
    );

    let start = Instant::now();
    let mut completed = 0u64;
    let mut last_report = start;
    let mut last_count = 0u64;
    let report_every = Duration::from_secs(5);

    while start.elapsed() < duration {
        let call_id = client.invite(&target).send().await?;
        let handle = client.coordinator().session(&call_id);
        client.wait_for_answered(handle.id()).await?;
        handle.hangup().await?;
        client.wait_for_ended(handle.id()).await?;
        completed += 1;

        if last_report.elapsed() >= report_every {
            let delta = completed - last_count;
            let secs = last_report.elapsed().as_secs_f64();
            println!(
                "[call_setup_loop] {:>6} calls total ({:>6.1} calls/sec)",
                completed,
                delta as f64 / secs
            );
            last_report = Instant::now();
            last_count = completed;
        }
    }

    let total_secs = start.elapsed().as_secs_f64();
    println!(
        "[call_setup_loop] done: {} calls in {:.2}s ({:.1} calls/sec)",
        completed,
        total_secs,
        completed as f64 / total_secs
    );

    client.shutdown().await?;
    server_task.abort();
    let _ = server_task.await;
    Ok(())
}
