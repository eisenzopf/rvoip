//! Asterisk endpoint 1001: register, call 1002, send 440 Hz, record received audio.

#[path = "common.rs"]
mod common;

use common::{
    endpoint_config, exchange_tone_and_record, init_tracing, load_env,
    post_register_settle_duration, register_endpoint, ENDPOINT_1001_TONE_HZ,
};
use rvoip_session_core::StreamPeer;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> common::ExampleResult<()> {
    load_env();
    init_tracing();

    let cfg = endpoint_config("1001", 5070, 16000, 16100)?;
    let mut peer = StreamPeer::with_config(cfg.stream_config()).await?;
    let registration = register_endpoint(&mut peer, &cfg).await?;

    let settle = post_register_settle_duration()?;
    if !settle.is_zero() {
        println!(
            "[1001] Waiting {}s for Asterisk OPTIONS qualify before calling...",
            settle.as_secs()
        );
        sleep(settle).await;
    }

    let target = cfg.outbound_call_uri("1002");
    println!("[1001] Calling {}...", target);
    let handle = peer.call(&target).await?;
    peer.wait_for_answered(handle.id()).await?;
    println!("[1001] Call established.");

    let wav = exchange_tone_and_record(
        &handle,
        ENDPOINT_1001_TONE_HZ,
        &cfg.output_dir,
        "1001_received.wav",
        true,
    )
    .await?;
    println!("[1001] Received audio saved to {}", wav.display());

    peer.unregister(&registration).await.ok();
    println!("[1001] Done.");
    Ok(())
}
