//! Call-center **customer**.
//!
//! Dials the support line (the B2BUA), which bridges the call to an agent.
//! From the customer's perspective it's an ordinary call — the bridging and
//! routing are invisible.
//!
//! Run the whole call center with `./run_demo.sh`.

use std::time::Duration;

use clap::Parser;
use rvoip_sip::{Config, StreamPeer};

#[derive(Parser, Debug)]
#[command(about = "Call-center customer — dials the support line")]
struct Args {
    /// Local SIP port.
    #[arg(long, default_value_t = 5080)]
    port: u16,
    /// Support line URI (the B2BUA).
    #[arg(long, default_value = "sip:support@127.0.0.1:5070")]
    support: String,
    /// How long to stay connected before hanging up.
    #[arg(long, default_value_t = 2)]
    talk_secs: u64,
}

#[tokio::main]
async fn main() -> rvoip_sip::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .init();
    let args = Args::parse();

    let mut customer = StreamPeer::with_config(Config::local("customer", args.port)).await?;

    println!("[customer] calling support at {}", args.support);
    let call_id = customer.invite(args.support.clone()).send().await?;
    let call = customer.coordinator().session(&call_id);
    customer.wait_for_answered(call.id()).await?;
    println!("[customer] ✅ connected to an agent via the call center");

    tokio::time::sleep(Duration::from_secs(args.talk_secs)).await;

    call.hangup_and_wait(Some(Duration::from_secs(5))).await?;
    println!("[customer] ✅ done");
    customer.shutdown().await
}
