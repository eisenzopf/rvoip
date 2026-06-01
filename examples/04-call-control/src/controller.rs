//! Call-control demo — the **controller**.
//!
//! Connects a call, then drives mid-call control over the established dialog:
//! hold (re-INVITE with the media direction changed), resume (re-INVITE back
//! to sendrecv), and a short RFC 4733 DTMF burst. All of this is on the
//! [`SessionHandle`] returned for the call, which is the per-call control
//! object shared by every rvoip-sip API surface.
//!
//! Run with `./run_demo.sh`, or pair manually with the `peer` binary.

use std::time::Duration;

use clap::Parser;
use rvoip_sip::{Config, StreamPeer};

#[derive(Parser, Debug)]
#[command(about = "Connects a call, then exercises hold / resume / DTMF")]
struct Args {
    /// Local SIP port.
    #[arg(long, default_value_t = 5060)]
    port: u16,
    /// Peer SIP port to dial.
    #[arg(long, default_value_t = 5061)]
    peer_port: u16,
}

#[tokio::main]
async fn main() -> rvoip_sip::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .init();
    let args = Args::parse();

    let mut peer = StreamPeer::with_config(Config::local("controller", args.port)).await?;

    let call_id = peer
        .invite(format!("sip:peer@127.0.0.1:{}", args.peer_port))
        .send()
        .await?;
    let call = peer.coordinator().session(&call_id);
    peer.wait_for_answered(call.id()).await?;
    println!("[controller] connected as {}", call.id());

    call.hold().await?;
    println!("[controller] ⏸  placed call on hold (re-INVITE)");
    tokio::time::sleep(Duration::from_millis(500)).await;

    call.resume().await?;
    println!("[controller] ▶  resumed call (re-INVITE)");
    tokio::time::sleep(Duration::from_millis(500)).await;

    for digit in ['1', '2', '#'] {
        call.send_dtmf(digit).await?;
        println!("[controller] sent DTMF {digit} (RFC 4733)");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    call.hangup_and_wait(Some(Duration::from_secs(5))).await?;
    println!("[controller] ✅ done");
    peer.shutdown().await
}
