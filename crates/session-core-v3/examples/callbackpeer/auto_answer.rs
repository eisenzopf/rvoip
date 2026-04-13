//! Simplest possible SIP server — auto-answers every incoming call.
//!
//!   cargo run --example callbackpeer_auto_answer
//!
//! Then call it from another peer (e.g. `hello_caller` pointed at this port).

use rvoip_session_core_v3::{CallbackPeer, Config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("rvoip_session_core_v3=info")
        .init();

    println!("Auto-answer server listening on port 5060...");
    let peer = CallbackPeer::with_auto_answer(Config::local("server", 5060)).await?;
    peer.run().await?;
    Ok(())
}
