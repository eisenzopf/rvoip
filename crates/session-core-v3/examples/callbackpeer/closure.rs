//! CallbackPeer with a closure — no trait boilerplate needed.
//!
//!   cargo run --example callbackpeer_closure
//!
//! Accepts calls from URIs containing "friend@", rejects everything else.

use rvoip_session_core_v3::{CallbackPeer, CallHandlerDecision, Config, StreamPeer};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("rvoip_session_core_v3=info")
        .init();

    // --- Background: two test callers ---
    tokio::spawn(async {
        sleep(Duration::from_secs(1)).await;

        // Caller 1: "friend@" URI — should be accepted
        let mut friend = StreamPeer::with_config(Config::local("friend", 5061)).await.unwrap();
        println!("[TEST] friend@ calling (should be accepted)...");
        let h = friend.call("sip:gatekeeper@127.0.0.1:5060").await.unwrap();
        friend.wait_for_answered(h.id()).await.ok();
        sleep(Duration::from_secs(2)).await;
        h.hangup().await.ok();
        friend.wait_for_ended(h.id()).await.ok();

        sleep(Duration::from_secs(1)).await;

        // Caller 2: "stranger@" URI — should be rejected
        let mut stranger = StreamPeer::with_config(Config::local("stranger", 5062)).await.unwrap();
        println!("[TEST] stranger@ calling (should be rejected)...");
        let h = stranger.call("sip:gatekeeper@127.0.0.1:5060").await.unwrap();
        sleep(Duration::from_secs(2)).await;
        h.hangup().await.ok();

        println!("[TEST] Done.");
        sleep(Duration::from_secs(1)).await;
        std::process::exit(0);
    });

    // --- Demo: closure-based handler ---
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
