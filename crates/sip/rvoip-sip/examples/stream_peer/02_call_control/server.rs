//! StreamPeer call-control server.
//!
//! Run with the client:
//!
//!   ./examples/stream_peer/02_call_control/run.sh

use std::time::Duration;

use rvoip_sip::{Config, Event, StreamPeer};

#[tokio::main]
async fn main() -> rvoip_sip::Result<()> {
    let mut peer = StreamPeer::with_config(Config::local("server", 5110)).await?;

    let incoming = peer.wait_for_incoming().await?;
    println!("[server] incoming from {}", incoming.from);
    let call = incoming.accept().await?;
    let mut events = call.events().await?;

    let mut digits = Vec::new();
    while digits.len() < 3 {
        let event = tokio::time::timeout(Duration::from_secs(10), events.next()).await;
        match event {
            Ok(Some(Event::DtmfReceived { digit, .. })) => {
                println!("[server] received DTMF {digit}");
                digits.push(digit);
            }
            Ok(Some(Event::CallEnded { .. })) | Ok(Some(Event::CallFailed { .. })) => break,
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(_) => break,
        }
    }

    call.wait_for_end(Some(Duration::from_secs(10))).await.ok();
    peer.shutdown().await
}
