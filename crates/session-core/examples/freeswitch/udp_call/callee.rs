#[path = "../common.rs"]
mod common;

use common::{endpoint_config, init_tracing, load_env, register_endpoint, ExampleResult};
use rvoip_session_core::StreamPeer;

#[tokio::main]
async fn main() -> ExampleResult<()> {
    load_env();
    init_tracing();

    let cfg = endpoint_config("2002", 15082, 17120, 17220)?;
    let mut peer = StreamPeer::with_config(cfg.stream_config()).await?;
    let registration = register_endpoint(&mut peer, &cfg).await?;
    println!("[2002] Registered; waiting for basic UDP call.");

    let timeout = common::remote_test_timeout()?;
    let incoming = tokio::time::timeout(timeout, peer.wait_for_incoming())
        .await
        .map_err(|_| "timed out waiting for FreeSWITCH basic UDP call")??;
    println!("[2002] Incoming call from {}", incoming.from);
    let call = incoming.accept().await?;
    let _ = call.wait_for_end(Some(timeout)).await?;

    peer.unregister(&registration).await.ok();
    peer.shutdown().await.ok();
    Ok(())
}
