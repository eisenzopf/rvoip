//! Call-control demo — the **peer**.
//!
//! Answers the inbound call and consumes its per-call event stream, logging
//! the DTMF digits the controller sends. Stays up until the call ends.
//!
//! Run with `./run_demo.sh`, or pair manually with the `controller` binary.

use std::time::Duration;

use clap::Parser;
use rvoip_sip::{Config, Event, StreamPeer};

#[derive(Parser, Debug)]
#[command(about = "Answers a call and logs hold/resume + received DTMF")]
struct Args {
    /// Local SIP port.
    #[arg(long, default_value_t = 5061)]
    port: u16,
}

#[tokio::main]
async fn main() -> rvoip_sip::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .init();
    let args = Args::parse();

    let mut peer = StreamPeer::with_config(Config::local("peer", args.port)).await?;
    println!("[peer] listening on sip:peer@127.0.0.1:{}", args.port);

    let incoming = peer.wait_for_incoming().await?;
    println!("[peer] incoming from {}", incoming.from);
    let call = incoming.accept().await?;
    let mut events = call.events().await?;

    // Collect the three DTMF digits the controller sends, then wind down.
    let mut digits = Vec::new();
    while digits.len() < 3 {
        match tokio::time::timeout(Duration::from_secs(10), events.next()).await {
            Ok(Some(Event::DtmfReceived { digit, .. })) => {
                println!("[peer] received DTMF {digit}");
                digits.push(digit);
            }
            Ok(Some(Event::CallEnded { .. })) | Ok(Some(Event::CallFailed { .. })) => break,
            Ok(Some(_)) => {} // hold/resume re-INVITEs surface here too
            Ok(None) | Err(_) => break,
        }
    }

    call.wait_for_end(Some(Duration::from_secs(10))).await.ok();
    println!("[peer] ✅ done (received {} DTMF digits)", digits.len());
    peer.shutdown().await
}
