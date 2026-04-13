//! Simplest possible SIP server — auto-answers every incoming call.
//!
//!   cargo run --example callbackpeer_auto_answer

use rvoip_session_core_v3::{CallbackPeer, Config, StreamPeer};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("rvoip_session_core_v3=info")
        .init();

    // --- Background: test caller that dials in after 1s, talks 3s, hangs up ---
    tokio::spawn(async {
        sleep(Duration::from_secs(1)).await;
        let mut caller = StreamPeer::with_config(Config::local("caller", 5061)).await.unwrap();
        println!("[TEST] Calling auto-answer server...");
        let handle = caller.call("sip:server@127.0.0.1:5060").await.unwrap();
        caller.wait_for_answered(handle.id()).await.ok();
        println!("[TEST] Connected! Hanging up in 3 seconds...");
        sleep(Duration::from_secs(3)).await;
        handle.hangup().await.ok();
        println!("[TEST] Done.");
        // Small delay then exit the process
        sleep(Duration::from_secs(1)).await;
        std::process::exit(0);
    });

    // --- Demo: auto-answer server ---
    println!("Auto-answer server listening on port 5060...");
    let peer = CallbackPeer::with_auto_answer(Config::local("server", 5060)).await?;
    peer.run().await?;
    Ok(())
}
