//! CallbackPeer with a closure — no trait boilerplate needed.
//!
//!   cargo run --example callbackpeer_closure
//!
//! Accepts calls from URIs containing "friend@", rejects everything else.

use rvoip_session_core_v3::{CallbackPeer, CallHandlerDecision, Config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("rvoip_session_core_v3=info")
        .init();

    println!("Closure-based server on port 5060...");
    println!("  Accepts calls from URIs containing 'friend@'");
    println!("  Rejects everything else with 403 Forbidden");

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

    peer.run().await?;
    Ok(())
}
