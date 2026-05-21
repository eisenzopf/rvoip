//! Long-running registration-storm scenario for CPU + lock-contention
//! profiling.
//!
//! Spawns one registrar and `RVOIP_PROFILE_FANOUT` client peers that
//! REGISTER → unregister continuously for `RVOIP_PROFILE_DURATION`
//! seconds. Pair with `samply` for CPU flamegraphs and `tokio-console`
//! for task wait-time analysis (requires `--features tokio-console` and
//! `RUSTFLAGS="--cfg tokio_unstable"`).
//!
//! See `crates/rvoip-sip/docs/PROFILING.md`.

use rvoip_sip::{Config, StreamPeer, UnifiedCoordinator};
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const DEFAULT_DURATION_SECS: u64 = 30;
const DEFAULT_FANOUT: usize = 16;
const REGISTRAR_PORT: u16 = 45500;
const REALM: &str = "profiling.local";
const USER: &str = "alice";
const PASS: &str = "password123";

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
    let fanout: usize = env::var("RVOIP_PROFILE_FANOUT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_FANOUT);

    let coordinator =
        UnifiedCoordinator::new(Config::local("profiling-registrar", REGISTRAR_PORT)).await?;
    let mut users = HashMap::new();
    users.insert(USER.to_string(), PASS.to_string());
    let _registrar = coordinator
        .start_registration_server(REALM, users)
        .await?;

    let target = Arc::new(format!("sip:127.0.0.1:{}", REGISTRAR_PORT));
    let completed = Arc::new(AtomicU64::new(0));
    let stop_at = Instant::now() + duration;

    println!(
        "[registration_storm] fanout={} duration={}s",
        fanout,
        duration.as_secs()
    );

    let mut handles = Vec::with_capacity(fanout);
    for id in 0..fanout {
        let target = Arc::clone(&target);
        let completed = Arc::clone(&completed);
        handles.push(tokio::spawn(async move {
            let port = 45600 + id as u16;
            let media_start = 46000 + (id * 50) as u16;
            let cfg = Config {
                media_port_start: media_start,
                media_port_end: media_start + 49,
                ..Config::local(&format!("profiling-reg-{}", id), port)
            };
            let peer = match StreamPeer::with_config(cfg).await {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("[reg-{}] build failed: {}", id, e);
                    return;
                }
            };
            while Instant::now() < stop_at {
                let handle = match peer.register(target.as_str(), USER, PASS).send().await {
                    Ok(h) => h,
                    Err(e) => {
                        eprintln!("[reg-{}] register failed: {}", id, e);
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                };
                for _ in 0..20 {
                    if peer.is_registered(&handle).await.unwrap_or(false) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                let _ = peer.unregister(&handle).await;
                completed.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }

    // Reporter task
    let completed_report = Arc::clone(&completed);
    let reporter = tokio::spawn(async move {
        let start = Instant::now();
        let mut last = 0u64;
        let mut last_at = start;
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.tick().await;
        while Instant::now() < stop_at {
            interval.tick().await;
            let now = Instant::now();
            let total = completed_report.load(Ordering::Relaxed);
            let delta = total - last;
            let secs = now.duration_since(last_at).as_secs_f64();
            println!(
                "[registration_storm] {:>8} regs total ({:>7.0} regs/sec)",
                total,
                delta as f64 / secs
            );
            last = total;
            last_at = now;
        }
    });

    for h in handles {
        let _ = h.await;
    }
    let _ = reporter.await;

    let total = completed.load(Ordering::Relaxed);
    println!(
        "[registration_storm] done: {} regs in {}s ({:.0} regs/sec)",
        total,
        duration.as_secs(),
        total as f64 / duration.as_secs_f64()
    );
    Ok(())
}
