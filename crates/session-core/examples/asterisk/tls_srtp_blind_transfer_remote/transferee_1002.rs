//! Asterisk TLS/SRTP blind transfer test, transferee side.
//!
//! 1002 answers the call from 1001 and stays available while Asterisk performs
//! the server-side transfer to rvoip endpoint 1003.

#[path = "../common.rs"]
mod common;

use common::{
    endpoint_config, init_tracing, load_env, register_endpoint, remote_test_timeout,
    start_tone_recorder, ExampleResult, ENDPOINT_1002_TONE_HZ,
};
use rvoip_session_core::StreamPeer;
use tokio::time::{sleep, timeout, Duration};

#[tokio::main]
async fn main() -> ExampleResult<()> {
    load_env();
    init_tracing();

    let cfg = endpoint_config("1002", 5072, 16120, 16220)?;
    let mut peer = StreamPeer::with_config(cfg.tls_srtp_stream_config()?).await?;
    let registration = register_endpoint(&mut peer, &cfg).await?;
    println!("[1002] Waiting for transferor call from 1001.");

    let wait = remote_test_timeout()?;
    let incoming = timeout(wait, peer.wait_for_incoming())
        .await
        .map_err(|_| format!("timed out after {:?} waiting for transferor call", wait))??;
    println!("[1002] Incoming call from {}", incoming.from);
    let handle = incoming.accept().await?;
    println!("[1002] Call answered; staying up while Asterisk completes the transfer.");
    let recorder = start_tone_recorder(&handle, ENDPOINT_1002_TONE_HZ).await?;
    println!(
        "[1002] Sending anchor/transferee {:.0}Hz tone.",
        ENDPOINT_1002_TONE_HZ
    );
    sleep(Duration::from_secs(12)).await;
    println!("[1002] Transfer window elapsed; hanging up anchor call.");
    handle.hangup().await.ok();
    handle.wait_for_end(Some(Duration::from_secs(8))).await.ok();
    let wav = recorder
        .stop_and_save(&cfg.output_dir, "tls_srtp_blind_transfer_1002_received.wav")
        .await?;
    println!("[1002] Received audio saved to {}", wav.display());

    peer.unregister(&registration).await.ok();
    peer.shutdown().await.ok();
    println!("[1002] Done.");
    Ok(())
}
