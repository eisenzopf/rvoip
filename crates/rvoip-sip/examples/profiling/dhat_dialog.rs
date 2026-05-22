//! Heap profile for a steady-state dialog workload.
//!
//! Establishes a small backlog of concurrent dialogs and then churns
//! through additional call setups under dhat. Smaller backlog than the
//! CPU-side `profiling_dialog_steady_state` because dhat slows things
//! down considerably. See `crates/rvoip-sip/docs/PROFILING.md`.
//!
//! ```bash
//! cargo run --release --features dhat -p rvoip-sip --example profiling_dhat_dialog
//! ```

#![cfg(feature = "dhat")]

use rvoip_sip::{Config, StreamPeer};
use std::time::Duration;

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

const SERVER_PORT: u16 = 50000;
const CLIENT_PORT: u16 = 50001;
const BACKLOG: usize = 25;
const CHURN_CALLS: usize = 50;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _profiler = dhat::Profiler::new_heap();

    let mut server = StreamPeer::with_config(Config {
        media_port_start: 50100,
        media_port_end: 50499,
        ..Config::local("dhat-steady-server", SERVER_PORT)
    })
    .await?;
    let server_task = tokio::spawn(async move {
        loop {
            match tokio::time::timeout(Duration::from_secs(120), server.wait_for_incoming()).await {
                Ok(Ok(incoming)) => {
                    if let Ok(h) = incoming.accept().await {
                        tokio::spawn(async move {
                            let _ = h.wait_for_end(Some(Duration::from_secs(300))).await;
                        });
                    }
                }
                _ => break,
            }
        }
    });

    let mut client = StreamPeer::with_config(Config {
        media_port_start: 50500,
        media_port_end: 50999,
        ..Config::local("dhat-steady-client", CLIENT_PORT)
    })
    .await?;
    let target = format!("sip:dhat-steady-server@127.0.0.1:{}", SERVER_PORT);

    let mut backlog_handles = Vec::with_capacity(BACKLOG);
    for _ in 0..BACKLOG {
        let call_id = client.invite(&target).send().await?;
        let handle = client.coordinator().session(&call_id);
        client.wait_for_answered(handle.id()).await?;
        backlog_handles.push(handle);
    }

    for _ in 0..CHURN_CALLS {
        let call_id = client.invite(&target).send().await?;
        let handle = client.coordinator().session(&call_id);
        client.wait_for_answered(handle.id()).await?;
        handle.hangup().await?;
        client.wait_for_ended(handle.id()).await?;
    }

    for h in &backlog_handles {
        let _ = h.hangup().await;
    }
    drop(backlog_handles);
    client.shutdown().await?;
    server_task.abort();
    let _ = server_task.await;

    println!(
        "[dhat_dialog] done — backlog={} churn={}; dhat-heap.json written",
        BACKLOG, CHURN_CALLS
    );
    Ok(())
}
