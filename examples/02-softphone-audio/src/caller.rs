//! Softphone audio — the **caller**.
//!
//! Builds an `Endpoint`, dials the callee, then sends a 440 Hz PCMU tone while
//! verifying it receives the callee's 880 Hz tone. Proves a real bidirectional
//! media path without any audio hardware.
//!
//! For a full hardware softphone (real mic/speaker via CPAL, plus a terminal
//! UI), see the in-crate example: `cargo run -p rvoip-sip --example sip_client`.
//!
//! Run with `./run_demo.sh`, or pair manually with the `callee` binary.

use std::time::Duration;

use clap::Parser;
use softphone_audio::{build_endpoint, exchange_audio, print_report, AudioPlan};

#[derive(Parser, Debug)]
#[command(about = "Softphone caller — sends a 440 Hz PCMU tone, expects 880 Hz back")]
struct Args {
    /// Local SIP port.
    #[arg(long, default_value_t = 5072)]
    port: u16,
    /// Callee SIP port to dial.
    #[arg(long, default_value_t = 5073)]
    peer_port: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .init();
    let args = Args::parse();

    let alice = build_endpoint("caller", args.port, 17_200, 17_249).await?;

    println!("[caller] calling sip:callee@127.0.0.1:{}", args.peer_port);
    let call = alice
        .call_and_wait(
            &format!("sip:callee@127.0.0.1:{}", args.peer_port),
            Some(Duration::from_secs(10)),
        )
        .await?;
    println!("[caller] call answered as {}", call.id());

    let report = exchange_audio(
        call.clone(),
        AudioPlan {
            role: "caller",
            remote: "callee",
            send_hz: 440.0,
            expect_hz: 880.0,
            reject_hz: 440.0,
        },
    )
    .await?;
    print_report(&report);

    call.hangup_and_wait(Some(Duration::from_secs(5))).await?;
    alice.shutdown().await?;
    Ok(())
}
