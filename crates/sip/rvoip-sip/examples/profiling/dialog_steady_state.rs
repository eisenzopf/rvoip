//! Steady-state dialog count scenario for CPU + lock-contention
//! profiling.
//!
//! Establishes `RVOIP_PROFILE_BACKLOG` persistent calls, then loops
//! setting up + tearing down additional calls against the backlog for
//! `RVOIP_PROFILE_DURATION` seconds. Pair with `tokio-console` to see
//! whether `Mutex` wait time grows with the backlog. See
//! `crates/sip/rvoip-sip/docs/PROFILING.md`.

use rvoip_sip::{Config, StreamPeer};
use std::env;
use std::time::{Duration, Instant};

const DEFAULT_DURATION_SECS: u64 = 60;
const DEFAULT_BACKLOG: usize = 250;
const SERVER_PORT: u16 = 47000;
const CLIENT_PORT: u16 = 47001;

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .init();

    #[cfg(feature = "tokio-console")]
    console_subscriber::init();

    let duration = Duration::from_secs(
        env::var("RVOIP_PROFILE_DURATION")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_DURATION_SECS),
    );
    let backlog: usize = env::var("RVOIP_PROFILE_BACKLOG")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_BACKLOG);

    let mut server = StreamPeer::with_config(Config {
        media_port_start: 47100,
        media_port_end: 47999,
        ..Config::local("profiling-steady-server", SERVER_PORT)
    })
    .await?;
    let server_task = tokio::spawn(async move {
        loop {
            match tokio::time::timeout(Duration::from_secs(300), server.wait_for_incoming()).await {
                Ok(Ok(incoming)) => {
                    if let Ok(h) = incoming.accept().await {
                        tokio::spawn(async move {
                            let _ = h.wait_for_end(Some(Duration::from_secs(600))).await;
                        });
                    }
                }
                _ => break,
            }
        }
    });

    let mut client = StreamPeer::with_config(Config {
        media_port_start: 48000,
        media_port_end: 48999,
        ..Config::local("profiling-steady-client", CLIENT_PORT)
    })
    .await?;
    let target = format!("sip:profiling-steady-server@127.0.0.1:{}", SERVER_PORT);

    println!(
        "[dialog_steady_state] establishing backlog of {} calls...",
        backlog
    );
    let mut backlog_handles = Vec::with_capacity(backlog);
    for i in 0..backlog {
        let call_id = client.invite(&target).send().await?;
        let handle = client.coordinator().session(&call_id);
        client.wait_for_answered(handle.id()).await?;
        backlog_handles.push(handle);
        if (i + 1) % 50 == 0 {
            println!("[dialog_steady_state]   backlog at {}/{}", i + 1, backlog);
        }
    }
    println!(
        "[dialog_steady_state] backlog ready; running churn loop for {}s",
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
                "[dialog_steady_state] backlog={} churn={} ({:.1} calls/sec)",
                backlog,
                completed,
                delta as f64 / secs
            );
            last_report = Instant::now();
            last_count = completed;
        }
    }

    println!(
        "[dialog_steady_state] done: {} churn calls over {} backlog dialogs in {}s",
        completed,
        backlog,
        duration.as_secs()
    );

    for h in &backlog_handles {
        let _ = h.hangup().await;
    }
    drop(backlog_handles);
    client.shutdown().await?;
    server_task.abort();
    let _ = server_task.await;
    Ok(())
}
