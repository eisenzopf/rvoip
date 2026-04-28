#[path = "../common.rs"]
mod common;

use rvoip_session_core::{Result, StreamPeer};

#[tokio::main]
async fn main() -> Result<()> {
    let user = common::env_or("FREESWITCH_CALLEE_USER", "1001");
    let password = common::env_or("FREESWITCH_CALLEE_PASSWORD", "1234");
    let timeout = common::env_duration_secs("FREESWITCH_TEST_TIMEOUT_SECS", 30);

    let mut peer = StreamPeer::with_config(common::config(&user, 15062)).await?;
    let reg = peer
        .register_with(common::registration(&user, &password))
        .await?;

    let incoming = tokio::time::timeout(timeout, peer.wait_for_incoming())
        .await
        .map_err(|_| {
            rvoip_session_core::SessionError::Timeout(
                "Timed out waiting for FreeSWITCH inbound call".into(),
            )
        })??;
    let call = incoming.accept().await?;
    let _ = call.wait_for_end(Some(timeout)).await?;

    let _ = peer
        .control()
        .coordinator()
        .unregister_and_wait(&reg, Some(timeout))
        .await;
    peer.shutdown().await?;
    Ok(())
}
