//! Asterisk TLS/SRTP blind transfer target: 1003 registers, answers the
//! transferred call, and waits for teardown.

#[path = "../common.rs"]
mod common;

use common::{
    endpoint_config, init_tracing, load_env, register_endpoint, start_tone_recorder, ExampleResult,
    ENDPOINT_1003_TONE_HZ,
};
use rvoip_session_core::StreamPeer;
use tokio::time::{sleep, timeout, Duration};

#[tokio::main]
async fn main() -> ExampleResult<()> {
    load_env();
    init_tracing();

    let cfg = endpoint_config("1003", 5074, 16240, 16340)?;
    let mut peer = StreamPeer::with_config(cfg.tls_srtp_stream_config()?).await?;
    let registration = register_endpoint(&mut peer, &cfg).await?;
    println!("[1003] Registered; waiting for transferred TLS/SRTP call.");

    let incoming = timeout(Duration::from_secs(90), peer.wait_for_incoming())
        .await
        .map_err(|_| "timed out waiting for transferred call")??;
    println!("[1003] Incoming transferred call from {}", incoming.from);
    let handle = incoming.accept().await?;
    println!("[1003] Transferred call answered.");
    let recorder = start_tone_recorder(&handle, ENDPOINT_1003_TONE_HZ).await?;
    println!(
        "[1003] Sending transferred-leg {:.0}Hz tone.",
        ENDPOINT_1003_TONE_HZ
    );
    sleep(Duration::from_secs(4)).await;
    println!("[1003] Hanging up transferred call.");
    handle.hangup().await.ok();
    handle.wait_for_end(Some(Duration::from_secs(8))).await.ok();
    let wav = recorder
        .stop_and_save(&cfg.output_dir, "tls_srtp_blind_transfer_1003_received.wav")
        .await?;
    println!("[1003] Received audio saved to {}", wav.display());

    peer.unregister(&registration).await.ok();
    peer.shutdown().await.ok();
    println!("[1003] Done.");
    Ok(())
}
