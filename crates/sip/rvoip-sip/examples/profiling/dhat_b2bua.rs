//! Heap profile for a registration-storm workload.
//!
//! Runs a small fixed registration storm under dhat so per-REGISTER
//! allocation counts are visible. Smaller numbers than the CPU-side
//! `profiling_registration_storm` because dhat instruments every
//! allocation. See `crates/rvoip-sip/docs/PROFILING.md`.
//!
//! ```bash
//! cargo run --release --features dhat -p rvoip-sip --example profiling_dhat_b2bua
//! ```

#![cfg(feature = "dhat")]

use rvoip_sip::{Config, StreamPeer, UnifiedCoordinator};
use std::collections::HashMap;
use std::time::Duration;

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

const REGISTRAR_PORT: u16 = 49000;
const CLIENT_BASE_PORT: u16 = 49100;
const REGISTRATIONS_PER_CLIENT: usize = 8;
const FANOUT: usize = 4;
const REALM: &str = "dhat.local";
const USER: &str = "alice";
const PASS: &str = "password123";

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _profiler = dhat::Profiler::new_heap();

    let coordinator =
        UnifiedCoordinator::new(Config::local("dhat-registrar", REGISTRAR_PORT)).await?;
    let mut users = HashMap::new();
    users.insert(USER.to_string(), PASS.to_string());
    let _registrar = coordinator.start_registration_server(REALM, users).await?;

    let target = format!("sip:127.0.0.1:{}", REGISTRAR_PORT);

    let mut handles = Vec::with_capacity(FANOUT);
    for id in 0..FANOUT {
        let target = target.clone();
        handles.push(tokio::spawn(async move {
            let port = CLIENT_BASE_PORT + id as u16;
            let media_start = 49500 + (id * 50) as u16;
            let cfg = Config {
                media_port_start: media_start,
                media_port_end: media_start + 49,
                ..Config::local(&format!("dhat-reg-{}", id), port)
            };
            let peer = StreamPeer::with_config(cfg).await.expect("peer");
            for _ in 0..REGISTRATIONS_PER_CLIENT {
                let handle = peer
                    .register(&target, USER, PASS)
                    .send()
                    .await
                    .expect("register");
                for _ in 0..20 {
                    if peer.is_registered(&handle).await.unwrap_or(false) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                peer.unregister(&handle).await.ok();
            }
        }));
    }

    for h in handles {
        let _ = h.await;
    }

    println!(
        "[dhat_b2bua] done — {} regs total ({} clients × {} regs); dhat-heap.json written",
        FANOUT * REGISTRATIONS_PER_CLIENT,
        FANOUT,
        REGISTRATIONS_PER_CLIENT
    );
    Ok(())
}
