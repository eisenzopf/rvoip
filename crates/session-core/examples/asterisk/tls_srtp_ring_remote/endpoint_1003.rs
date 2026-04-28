//! Asterisk TLS/SRTP ring/cancel target: 1003 registers and lets 1001 ring
//! without answering so 1001 can validate CANCEL through Asterisk.

#[path = "../common.rs"]
mod common;

use common::{endpoint_config, init_tracing, load_env, register_endpoint, ExampleResult};
use rvoip_session_core::StreamPeer;
use tokio::time::{sleep, timeout, Duration};

#[tokio::main]
async fn main() -> ExampleResult<()> {
    load_env();
    init_tracing();

    let cfg = endpoint_config("1003", 5074, 16240, 16340)?;
    let mut peer = StreamPeer::with_config(cfg.tls_srtp_stream_config()?).await?;
    let registration = register_endpoint(&mut peer, &cfg).await?;
    println!("[1003] Registered; waiting for ring/cancel call.");

    let incoming = timeout(Duration::from_secs(60), peer.wait_for_incoming())
        .await
        .map_err(|_| "timed out waiting for ring/cancel call")??;
    println!(
        "[1003] Incoming call from {}; holding without answering.",
        incoming.from
    );
    let guard = incoming.defer(Duration::from_secs(30));
    sleep(Duration::from_secs(12)).await;
    drop(guard);

    peer.unregister(&registration).await.ok();
    peer.shutdown().await.ok();
    println!("[1003] Done.");
    Ok(())
}
