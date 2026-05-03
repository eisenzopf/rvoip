//! Test caller for the CallbackPeer builder IVR.
//!
//! Run with the server:
//!
//!   ./examples/callback_peer/03_builder_ivr/run.sh

use std::time::Duration;

use rvoip_session_core::{Config, StreamPeer};

#[tokio::main]
async fn main() -> rvoip_session_core::Result<()> {
    let mut caller = StreamPeer::with_config(Config::local("caller", 5121)).await?;

    let call = caller.call("sip:ivr@127.0.0.1:5120").await?;
    caller.wait_for_answered(call.id()).await?;

    for digit in ['1', '2', '#'] {
        call.send_dtmf(digit).await?;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    call.hangup_and_wait(Some(Duration::from_secs(5))).await?;
    caller.shutdown().await
}
