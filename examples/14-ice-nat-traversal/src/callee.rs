//! ICE NAT traversal — the **callee** (Bob).
//!
//! Listens for one inbound call with `Config::enable_ice = true`, accepts
//! it, waits for a real RFC 8445 ICE connectivity check to complete, prints
//! the selected candidate pair, then waits for the caller to hang up.
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
#[command(about = "ICE NAT traversal callee — waits for a call, answers, waits for ICE")]
struct Args {
    /// SIP port this callee binds to.
    #[arg(long, default_value_t = 5061)]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .init();
    let args = Args::parse();

    let mut config = Config::local("callee", args.port);
    config.enable_ice = true;
    let mut bob = StreamPeer::with_config(config).await?;
    println!("[callee] listening on sip:callee@127.0.0.1:{}", args.port);

    // Subscribe before accepting so an IceConnected fired right after the
    // 200 OK isn't missed.
    let mut events = bob.coordinator().events().await?;

    let incoming = bob.wait_for_incoming().await?;
    println!("[callee] incoming call from {}", incoming.from);
    let call = incoming.accept().await?;
    println!("[callee] answered {}", call.id());

    println!("[callee] waiting for ICE connectivity check…");
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
        Some(addr) => println!("[callee] ICE connected — selected pair remote address: {addr}"),
        None => println!("[callee] no IceConnected event observed"),
    }

    call.wait_for_end(None).await?;
    println!("[callee] call ended");
    bob.shutdown().await
}
