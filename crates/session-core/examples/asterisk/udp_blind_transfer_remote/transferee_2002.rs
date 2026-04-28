//! Asterisk UDP blind transfer test, transferee side.
//!
//! 2002 answers the call from 2001 and stays available while Asterisk performs
//! the server-side transfer to rvoip endpoint 2003.

#[path = "../common.rs"]
mod common;

use common::{
    endpoint_config, init_tracing, load_env, register_endpoint, remote_test_timeout, ExampleResult,
};
use rvoip_session_core::StreamPeer;
use tokio::time::{sleep, timeout, Duration};

#[tokio::main]
async fn main() -> ExampleResult<()> {
    load_env();
    init_tracing();

    let cfg = endpoint_config("2002", 5082, 17120, 17220)?;
    let mut peer = StreamPeer::with_config(cfg.stream_config()).await?;
    let registration = register_endpoint(&mut peer, &cfg).await?;
    println!("[2002] Waiting for transferor call from 2001.");

    let wait = remote_test_timeout()?;
    let incoming = timeout(wait, peer.wait_for_incoming())
        .await
        .map_err(|_| format!("timed out after {:?} waiting for transferor call", wait))??;
    println!("[2002] Incoming call from {}", incoming.from);
    let handle = incoming.accept().await?;
    println!("[2002] Call answered; staying up while Asterisk completes the transfer.");
    sleep(Duration::from_secs(10)).await;
    println!("[2002] Transfer window elapsed; hanging up anchor call.");
    handle.hangup().await.ok();
    handle.wait_for_end(Some(Duration::from_secs(8))).await.ok();

    peer.unregister(&registration).await.ok();
    peer.shutdown().await.ok();
    println!("[2002] Done.");
    Ok(())
}
