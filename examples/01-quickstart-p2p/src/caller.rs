//! Quickstart P2P — the **caller** (Alice).
//!
//! Dials the callee over loopback, waits for the call to be answered, holds
//! the media path open for ~1 second, then hangs up cleanly.
//!
//! This is the smallest possible end-to-end SIP call with rvoip-sip. It uses
//! the [`StreamPeer`] surface — a sequential client API where each helper
//! blocks until the next matching event — which keeps simple clients direct.
//!
//! Run both sides together with `./run_demo.sh`, or manually:
//!
//! ```text
//! cargo run --bin callee -- --port 5061      # terminal 1
//! cargo run --bin caller -- --peer-port 5061 # terminal 2
//! ```

use std::time::Duration;

use clap::Parser;
use rvoip_sip::{Config, Result, StreamPeer};

#[derive(Parser, Debug)]
#[command(about = "Quickstart P2P caller — dials the callee, talks ~1s, hangs up")]
struct Args {
    /// SIP port this caller binds to.
    #[arg(long, default_value_t = 5060)]
    port: u16,
    /// SIP port of the callee to dial.
    #[arg(long, default_value_t = 5061)]
    peer_port: u16,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .init();
    let args = Args::parse();

    // Bind a peer on a local UDP port. `Config::local` defaults to PCMU/PCMA
    // media over UDP on 127.0.0.1 — the beta full-media profile.
    let mut alice = StreamPeer::with_config(Config::local("caller", args.port)).await?;

    let target = format!("sip:callee@127.0.0.1:{}", args.peer_port);
    println!("[caller] inviting {target}");

    // INVITE → 100/180 → 200 OK → ACK is driven by the stack. `send()`
    // returns once the request is on the wire; `wait_for_answered` blocks
    // until the 200 OK arrives (or the timeout / failure).
    let call_id = alice.invite(target).send().await?;
    let call = alice.coordinator().session(&call_id);
    alice.wait_for_answered(call.id()).await?;
    println!("[caller] ✅ call connected as {}", call.id());

    // Media (PCMU RTP) is flowing in both directions here. A real softphone
    // would pump audio frames in; see example 02-softphone-audio.
    tokio::time::sleep(Duration::from_secs(1)).await;

    // BYE → 200 OK. `hangup_and_wait` blocks until the dialog is fully torn
    // down so the process can exit deterministically.
    call.hangup_and_wait(Some(Duration::from_secs(5))).await?;
    println!("[caller] ✅ call completed, hung up cleanly");
    alice.shutdown().await
}
