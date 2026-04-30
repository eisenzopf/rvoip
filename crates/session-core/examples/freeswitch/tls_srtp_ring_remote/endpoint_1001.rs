//! FreeSWITCH TLS/SRTP ring/cancel test: rvoip user 1001 calls rvoip endpoint
//! 1003, waits briefly for FreeSWITCH to deliver the INVITE, then cancels.

#[path = "../common.rs"]
mod common;

use common::{
    endpoint_config, env_duration_secs, init_tracing, load_env, post_register_settle_duration,
    register_endpoint, wait_for_cancel_cleanup, ExampleResult,
};
use rvoip_session_core::StreamPeer;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> ExampleResult<()> {
    load_env();
    init_tracing();

    let cfg = endpoint_config("1001", 15070, 16000, 16100)?;
    let mut peer = StreamPeer::with_config(cfg.tls_srtp_stream_config()?).await?;
    let registration = register_endpoint(&mut peer, &cfg).await?;

    let settle = post_register_settle_duration()?;
    if !settle.is_zero() {
        println!(
            "[1001] Waiting {}s for FreeSWITCH OPTIONS qualify before calling...",
            settle.as_secs()
        );
        sleep(settle).await;
    }

    let target = cfg.remote_call_uri();
    println!(
        "[1001] Calling rvoip TLS/SRTP target {}. It should ring; no answer required.",
        target
    );
    let handle = peer.call(&target).await?;
    let cancel_delay = env_duration_secs("FREESWITCH_RING_CANCEL_DELAY_SECS", 2);
    println!(
        "[1001] Call submitted; waiting {}s before cancelling.",
        cancel_delay.as_secs()
    );
    sleep(cancel_delay).await;

    println!("[1001] Cancelling call.");
    handle.hangup().await?;
    wait_for_cancel_cleanup(&handle, Duration::from_secs(12)).await?;

    peer.unregister(&registration).await.ok();
    peer.shutdown().await.ok();
    println!("[1001] Ring/cancel test passed.");
    Ok(())
}
