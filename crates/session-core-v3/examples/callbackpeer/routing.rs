//! URI-based call routing with RoutingHandler.
//!
//!   cargo run --example callbackpeer_routing
//!
//! Routes incoming calls based on the To URI:
//! - "support@" -> Accept
//! - "sales@"   -> Accept
//! - "spam@"    -> Reject 403
//! - anything else -> Reject 404

use rvoip_session_core_v3::api::handlers::{RoutingAction, RoutingHandler};
use rvoip_session_core_v3::{CallbackPeer, Config, StreamPeer};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("rvoip_session_core_v3=info")
        .init();

    // --- Background: test callers for each route ---
    tokio::spawn(async {
        sleep(Duration::from_secs(1)).await;

        for (i, target) in ["support", "spam", "unknown"].iter().enumerate() {
            let port = 5061 + i as u16;
            let mut caller =
                StreamPeer::with_config(Config::local(&format!("caller{}", i), port))
                    .await
                    .unwrap();
            println!("[TEST] Calling {}@... ", target);
            let h = caller
                .call(&format!("sip:{}@127.0.0.1:5060", target))
                .await
                .unwrap();
            sleep(Duration::from_secs(2)).await;
            h.hangup().await.ok();
            caller.wait_for_ended(h.id()).await.ok();
            sleep(Duration::from_millis(500)).await;
        }

        println!("[TEST] Done.");
        sleep(Duration::from_secs(1)).await;
        std::process::exit(0);
    });

    // --- Demo: routing handler ---
    let handler = RoutingHandler::new()
        .with_rule("support@", RoutingAction::Accept)
        .with_rule("sales@", RoutingAction::Accept)
        .with_rule(
            "spam@",
            RoutingAction::Reject {
                status: 403,
                reason: "Forbidden".into(),
            },
        )
        .with_default(RoutingAction::Reject {
            status: 404,
            reason: "Not Found".into(),
        });

    println!("Routing server on port 5060...");
    println!("  support@ -> Accept");
    println!("  sales@   -> Accept");
    println!("  spam@    -> 403 Forbidden");
    println!("  *        -> 404 Not Found");

    let peer = CallbackPeer::new(handler, Config::local("router", 5060)).await?;
    peer.run().await?;
    Ok(())
}
