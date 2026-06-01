//! URI-based call routing server.
//!
//! Run standalone:  cargo run -p rvoip-sip --example callback_peer_routing_handler_server
//! Or with client:  ./examples/callback_peer/04_routing_handler/run.sh

use rvoip_sip::api::handlers::{RoutingAction, RoutingHandler};
use rvoip_sip::{CallbackPeer, Config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_sip_dialog=error".into()),
        )
        .init();

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

    let peer = CallbackPeer::new(handler, Config::local("router", 5060)).await?;

    println!("Listening on port 5060 (routing server)...");
    println!("  support@ -> Accept");
    println!("  sales@   -> Accept");
    println!("  spam@    -> 403 Forbidden");
    println!("  *        -> 404 Not Found");
    println!("Press Ctrl+C to stop.");

    tokio::select! {
        res = peer.run() => res?,
        _ = tokio::signal::ctrl_c() => println!("\nShutting down."),
    }

    std::process::exit(0);
}
