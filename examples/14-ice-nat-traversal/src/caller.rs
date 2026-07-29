//! ICE NAT traversal — the **caller** (Alice).
//!
//! Dials the callee over loopback with `Config::enable_ice = true`, waits
//! for the call to be answered, then waits for a real RFC 8445 ICE
//! connectivity check to complete and prints the selected candidate pair
//! before hanging up.
//!
//! Uses the [`StreamPeer`] surface on both sides (like
//! [01-quickstart-p2p](../01-quickstart-p2p/)) so each side can subscribe to
//! the raw event stream and observe `Event::IceConnected` directly — the
//! callback (`CallHandler`) surface doesn't route this event to a dedicated
//! hook today.
//!
//! Run both sides together with `./run_demo.sh`, or manually:
//!
//! ```text
//! cargo run --bin callee -- --port 5061      # terminal 1
//! cargo run --bin caller -- --peer-port 5061 # terminal 2
//! ```

use std::time::Duration;

use clap::Parser;
use rvoip_sip::{Config, Event, Result, StreamPeer};

#[derive(Parser, Debug)]
#[command(about = "ICE NAT traversal caller — dials the callee, waits for ICE, hangs up")]
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

    // Config::enable_ice turns on real RFC 8445 candidate gathering and
    // connectivity checks (requires rvoip-sip's `ice` feature — see this
    // example's Cargo.toml). Orthogonal to SRTP keying: combine freely
    // with SDES or DTLS-SRTP, or leave media plaintext as here.
    let mut config = Config::local("caller", args.port);
    config.enable_ice = true;
    let mut alice = StreamPeer::with_config(config).await?;

    // Subscribe before placing the call so an IceConnected fired right
    // after the answer isn't missed.
    let mut events = alice.coordinator().events().await?;

    let target = format!("sip:callee@127.0.0.1:{}", args.peer_port);
    println!("[caller] inviting {target}");

    let call_id = alice.invite(target).send().await?;
    let call = alice.coordinator().session(&call_id);
    alice.wait_for_answered(call.id()).await?;
    println!("[caller] call connected as {}", call.id());

    println!("[caller] waiting for ICE connectivity check…");
    let selected = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match events.next().await {
                Some(Event::IceConnected { selected_addr, .. }) => return Some(selected_addr),
                Some(_) => continue,
                None => return None,
            }
        }
    })
    .await
    .ok()
    .flatten();

    match selected {
        Some(addr) => println!("[caller] ICE connected — selected pair remote address: {addr}"),
        None => println!("[caller] no IceConnected event observed"),
    }

    call.hangup_and_wait(Some(Duration::from_secs(5))).await?;
    println!("[caller] call completed, hung up cleanly");
    alice.shutdown().await
}
