//! URI-based call routing server.
//!
//! Run standalone:  cargo run -p rvoip-session-core --example callbackpeer_routing_server
//! Or with client:  ./examples/callbackpeer/routing/run.sh

use rvoip_session_core::api::handlers::{RoutingAction, RoutingHandler};
use rvoip_session_core::{CallbackPeer, Config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()),
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
