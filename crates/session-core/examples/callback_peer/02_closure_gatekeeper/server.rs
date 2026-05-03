//! Closure-based SIP gatekeeper server.
//!
//! Run standalone:  cargo run -p rvoip-session-core --example callbackpeer_closure_server
//! Or with client:  ./examples/callbackpeer/closure/run.sh

use rvoip_session_core::{CallHandlerDecision, CallbackPeer, Config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()),
        )
        .init();

    let peer = CallbackPeer::from_fn(Config::local("gatekeeper", 5060), |call| {
        if call.from.contains("friend@") {
            println!("Accepting call from {}", call.from);
            CallHandlerDecision::Accept
        } else {
            println!("Rejecting call from {}", call.from);
            CallHandlerDecision::Reject {
                status: 403,
                reason: "Forbidden".into(),
            }
        }
    })
    .await?;

    println!("Listening on port 5060 (closure gatekeeper)...");
    println!("  Accepts calls from URIs containing 'friend@'");
    println!("  Rejects everything else with 403 Forbidden");
    println!("Press Ctrl+C to stop.");

    tokio::select! {
        res = peer.run() => res?,
        _ = tokio::signal::ctrl_c() => println!("\nShutting down."),
    }

    std::process::exit(0);
}
