//! Asterisk TLS/SRTP DTMF receiver: 1002 answers 1001 and validates digits.

#[path = "../common.rs"]
mod common;

use common::{
    endpoint_config, init_tracing, load_env, register_endpoint, remote_test_digits,
    remote_test_timeout, wait_for_dtmf_sequence_on_events, ExampleResult,
};
use rvoip_session_core::StreamPeer;
use tokio::time::Duration;

#[tokio::main]
async fn main() -> ExampleResult<()> {
    load_env();
    init_tracing();

    let cfg = endpoint_config("1002", 5072, 16120, 16220)?;
    let mut peer = StreamPeer::with_config(cfg.tls_srtp_stream_config()?).await?;
    let registration = register_endpoint(&mut peer, &cfg).await?;
    println!("[1002] Registered; waiting for TLS/SRTP DTMF call.");

    let incoming = peer.wait_for_incoming().await?;
    println!("[1002] Incoming call from {}", incoming.from);
    let handle = incoming.accept().await?;
    let mut events = handle.events().await?;
    let digits = remote_test_digits();
    println!(
        "[1002] Call answered; waiting for DTMF digits {}.",
        digits.iter().collect::<String>()
    );

    wait_for_dtmf_sequence_on_events(&mut events, &digits, remote_test_timeout()?).await?;
    handle
        .wait_for_end(Some(Duration::from_secs(10)))
        .await
        .ok();

    peer.unregister(&registration).await.ok();
    peer.shutdown().await.ok();
    println!("[1002] DTMF sequence received. Done.");
    Ok(())
}
