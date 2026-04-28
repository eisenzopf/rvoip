//! Asterisk TLS/SRTP blind transfer test, transferor side.
//!
//! 1001 calls rvoip endpoint 1002 through Asterisk, then sends REFER to
//! transfer 1002 to rvoip endpoint 1003. 1001 waits for REFER NOTIFY completion.

#[path = "../common.rs"]
mod common;

use common::{
    call_with_answer_retry, endpoint_config, init_tracing, load_env, post_register_settle_duration,
    register_endpoint, remote_test_timeout, start_tone_recorder,
    wait_for_transfer_completion_on_events, ExampleResult, ENDPOINT_1001_TONE_HZ,
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

    let call_target = cfg.outbound_call_uri("1002");
    let transfer_target = cfg.remote_call_uri();
    println!("[1001] Calling {} before transfer.", call_target);
    let handle = call_with_answer_retry(&mut peer, &call_target, remote_test_timeout()?).await?;
    println!(
        "[1001] Call established; transferring peer to rvoip target {}.",
        transfer_target
    );
    let recorder = start_tone_recorder(&handle, ENDPOINT_1001_TONE_HZ).await?;
    println!(
        "[1001] Sending initial-leg {:.0}Hz tone.",
        ENDPOINT_1001_TONE_HZ
    );
    sleep(Duration::from_secs(3)).await;

    let mut events = handle.events().await?;
    handle.transfer_blind(&transfer_target).await?;
    wait_for_transfer_completion_on_events(&mut events, remote_test_timeout()?).await?;
    handle.wait_for_end(Some(Duration::from_secs(8))).await.ok();
    let wav = recorder
        .stop_and_save(&cfg.output_dir, "tls_srtp_blind_transfer_1001_received.wav")
        .await?;
    println!("[1001] Received audio saved to {}", wav.display());

    peer.unregister(&registration).await.ok();
    peer.shutdown().await.ok();
    println!("[1001] Transfer completed. Done.");
    Ok(())
}
