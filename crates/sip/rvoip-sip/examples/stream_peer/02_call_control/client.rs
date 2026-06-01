//! StreamPeer call-control client.
//!
//! Run with the server:
//!
//!   ./examples/stream_peer/02_call_control/run.sh

use std::time::Duration;

use rvoip_sip::{Config, StreamPeer};

#[tokio::main]
async fn main() -> rvoip_sip::Result<()> {
    let mut peer = StreamPeer::with_config(Config::local("controller", 5111)).await?;

    let call_id = peer.invite("sip:server@127.0.0.1:5110").send().await?;
    let call = peer.coordinator().session(&call_id);
    peer.wait_for_answered(call.id()).await?;
    println!("[client] connected as {}", call.id());

    call.hold().await?;
    println!("[client] placed call on hold");
    tokio::time::sleep(Duration::from_millis(500)).await;

    call.resume().await?;
    println!("[client] resumed call");
    tokio::time::sleep(Duration::from_millis(500)).await;

    for digit in ['1', '2', '#'] {
        call.send_dtmf(digit).await?;
        println!("[client] sent DTMF {digit}");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    call.hangup().await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    peer.shutdown().await
}
