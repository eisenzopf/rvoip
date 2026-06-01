//! Softphone audio — the **callee**.
//!
//! Builds an `Endpoint`, answers the inbound call, then sends an 880 Hz PCMU
//! tone while verifying it receives the caller's 440 Hz tone.
//!
//! Run with `./run_demo.sh`, or pair manually with the `caller` binary.

use std::time::Duration;

use clap::Parser;
use softphone_audio::{build_endpoint, exchange_audio, print_report, AudioPlan};

#[derive(Parser, Debug)]
#[command(about = "Softphone callee — answers, sends 880 Hz, expects 440 Hz")]
struct Args {
    /// Local SIP port.
    #[arg(long, default_value_t = 5073)]
    port: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .init();
    let args = Args::parse();

    let mut bob = build_endpoint("callee", args.port, 17_250, 17_299).await?;
    println!("[callee] waiting on sip:callee@127.0.0.1:{}", args.port);

    let incoming = bob.wait_for_incoming().await?;
    println!("[callee] incoming call from {}", incoming.from());
    let call = incoming.answer().await?;
    println!("[callee] answered as {}", call.id());

    let report = exchange_audio(
        call.clone(),
        AudioPlan {
            role: "callee",
            remote: "caller",
            send_hz: 880.0,
            expect_hz: 440.0,
            reject_hz: 880.0,
        },
    )
    .await?;
    print_report(&report);

    call.wait_for_end(Some(Duration::from_secs(10))).await?;
    bob.shutdown().await?;
    Ok(())
}
