#[path = "../common.rs"]
mod common;

use rvoip_session_core::{Result, StreamPeer};

#[tokio::main]
async fn main() -> Result<()> {
    let user = common::env_or("FREESWITCH_CALLER_USER", "1000");
    let password = common::env_or("FREESWITCH_CALLER_PASSWORD", "1234");
    let target = common::env_or("FREESWITCH_TARGET_USER", "1001");
    let timeout = common::env_duration_secs("FREESWITCH_TEST_TIMEOUT_SECS", 30);

    let mut peer = StreamPeer::with_config(common::config(&user, 15061)).await?;
    let reg = peer
        .register_with(common::registration(&user, &password))
        .await?;
    let call = peer.call(&common::call_uri(&target)).await?;

    let answered = tokio::time::timeout(timeout, peer.wait_for_answered(call.id()))
        .await
        .map_err(|_| {
            rvoip_session_core::SessionError::Timeout("FreeSWITCH call timed out".into())
        })??;

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    answered.hangup_and_wait(Some(timeout)).await?;

    let _ = peer
        .control()
        .coordinator()
        .unregister_and_wait(&reg, Some(timeout))
        .await;

    peer.shutdown().await?;
    Ok(())
}
