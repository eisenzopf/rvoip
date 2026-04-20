//! Auto-answer SIP server for the hold/resume example.
//!
//! Run standalone:  cargo run -p rvoip-session-core --example streampeer_hold_resume_server
//! Or with client:  ./examples/streampeer/hold_resume/run.sh

use rvoip_session_core::{CallbackPeer, Config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()))
        .init();

    let peer = CallbackPeer::with_auto_answer(Config::local("server", 5060)).await?;

    println!("Listening on port 5060 (auto-answer)...");
    println!("Press Ctrl+C to stop.");

    tokio::select! {
        res = peer.run() => res?,
        _ = tokio::signal::ctrl_c() => println!("\nShutting down."),
    }

    std::process::exit(0);
}
