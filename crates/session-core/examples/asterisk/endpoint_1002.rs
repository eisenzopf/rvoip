//! Asterisk endpoint 1002: register, answer 1001, send 880 Hz, record received audio.

#[path = "common.rs"]
mod common;

use common::{
    endpoint_config, exchange_tone_and_record, init_tracing, load_env, register_endpoint,
    ENDPOINT_1002_TONE_HZ,
};
use rvoip_session_core::StreamPeer;

#[tokio::main]
async fn main() -> common::ExampleResult<()> {
    load_env();
    init_tracing();

    let cfg = endpoint_config("1002", 5072, 16120, 16220)?;
    let mut peer = StreamPeer::with_config(cfg.stream_config()).await?;
    let registration = register_endpoint(&mut peer, &cfg).await?;
    println!("[1002] Registered; waiting for call.");

    let incoming = peer.wait_for_incoming().await?;
    println!("[1002] Incoming call from {}", incoming.from);
    let handle = incoming.accept().await?;
    println!("[1002] Call answered.");

    let wav = exchange_tone_and_record(
        &handle,
        ENDPOINT_1002_TONE_HZ,
        &cfg.output_dir,
        "1002_received.wav",
        false,
    )
    .await?;
    println!("[1002] Received audio saved to {}", wav.display());

    peer.unregister(&registration).await.ok();
    println!("[1002] Done.");
    Ok(())
}
