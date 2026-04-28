#[path = "../common.rs"]
mod common;

use rvoip_session_core::{Result, StreamPeer};

#[tokio::main]
async fn main() -> Result<()> {
    let user = common::env_or("FREESWITCH_USER", "1000");
    let password = common::env_or("FREESWITCH_PASSWORD", "1234");
    let timeout = common::env_duration_secs("FREESWITCH_TEST_TIMEOUT_SECS", 10);

    let mut peer = StreamPeer::with_config(common::config(&user, 15060)).await?;
    let handle = peer
        .register_with(common::registration(&user, &password))
        .await?;

    tokio::time::timeout(timeout, async {
        loop {
            if peer.is_registered(&handle).await? {
                break Ok::<(), rvoip_session_core::SessionError>(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|_| {
        rvoip_session_core::SessionError::Timeout("FreeSWITCH REGISTER timed out".into())
    })??;

    peer.control()
        .coordinator()
        .unregister_and_wait(&handle, Some(timeout))
        .await?;
    peer.shutdown().await?;
    Ok(())
}
