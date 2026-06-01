//! Call-center **agent** endpoint.
//!
//! A reactive [`CallbackPeer`] that auto-answers the leg the B2BUA originates
//! to it and stays up to take more calls. Start two of these (ports 5071 and
//! 5072) to see the server round-robin between them.
//!
//! Run the whole call center with `./run_demo.sh`.

use clap::Parser;
use rvoip_sip::{CallHandlerDecision, CallbackPeer, Config, Result};

#[derive(Parser, Debug)]
#[command(about = "Call-center agent — auto-answers bridged calls")]
struct Args {
    /// SIP port this agent binds to.
    #[arg(long, default_value_t = 5071)]
    port: u16,
    /// Display label for logs (e.g. \"alice\").
    #[arg(long, default_value = "agent")]
    name: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .init();
    let args = Args::parse();
    let label = args.name.clone();
    let established_label = label.clone();
    let ended_label = label.clone();

    let peer = CallbackPeer::builder(Config::local("agent", args.port))
        .on_incoming(move |call| {
            let label = label.clone();
            async move {
                println!("[{label}] incoming bridged call from {}", call.from);
                CallHandlerDecision::Accept
            }
        })
        .on_established(move |call| {
            let label = established_label.clone();
            async move {
                println!("[{label}] ✅ connected to customer ({})", call.id());
                Ok(())
            }
        })
        .on_ended(move |call_id, reason| {
            let label = ended_label.clone();
            async move {
                println!("[{label}] call {call_id} ended: {reason:?}");
                Ok(())
            }
        })
        .build()
        .await?;

    println!("[{}] agent ready on sip:agent@127.0.0.1:{}", args.name, args.port);
    tokio::select! {
        res = peer.run() => res,
        _ = tokio::signal::ctrl_c() => Ok(()),
    }
}
