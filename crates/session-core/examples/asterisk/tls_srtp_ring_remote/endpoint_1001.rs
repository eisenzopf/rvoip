//! Asterisk TLS/SRTP ring/cancel test: rvoip user 1001 calls rvoip endpoint
//! 1003, verifies the call reaches Ringing, then cancels.

#[path = "../common.rs"]
mod common;

use common::{
    call_with_ringing_retry, endpoint_config, init_tracing, load_env,
    post_register_settle_duration, register_endpoint, remote_test_timeout, wait_for_cancel_cleanup,
    ExampleResult,
};
use rvoip_session_core::StreamPeer;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> ExampleResult<()> {
    load_env();
    init_tracing();

    let cfg = endpoint_config("1001", 5070, 16000, 16100)?;
    let mut peer = StreamPeer::with_config(cfg.tls_srtp_stream_config()?).await?;
    let registration = register_endpoint(&mut peer, &cfg).await?;

    let settle = post_register_settle_duration()?;
    if !settle.is_zero() {
        println!(
            "[1001] Waiting {}s for Asterisk OPTIONS qualify before calling...",
            settle.as_secs()
        );
        sleep(settle).await;
    }

    let target = cfg.remote_call_uri();
    println!(
        "[1001] Calling rvoip TLS/SRTP target {}. It should ring; no answer required.",
        target
    );
    let handle = call_with_ringing_retry(&mut peer, &target, remote_test_timeout()?).await?;
    println!("[1001] Target is ringing; cancelling call.");
    handle.hangup().await?;
    wait_for_cancel_cleanup(&handle, Duration::from_secs(12)).await?;

    peer.unregister(&registration).await.ok();
    peer.shutdown().await.ok();
    println!("[1001] Ring/cancel test passed.");
    Ok(())
}
