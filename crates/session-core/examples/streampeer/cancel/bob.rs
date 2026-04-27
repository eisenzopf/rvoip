//! CANCEL callee (Bob).
//!
//! Bob listens and answers `wait_for_incoming()` — but never accepts the
//! call. This keeps Alice in the `Ringing` state long enough for her to
//! send CANCEL. Bob just needs to stay alive; dialog-core auto-replies
//! 180 Ringing on the incoming INVITE, and on receiving CANCEL it
//! replies 487 Request Terminated and tears down the transaction.

use rvoip_session_core::{Config, StreamPeer};
use tokio::time::{sleep, timeout, Duration};

fn env_port(key: &str, default: u16) -> u16 {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()),
        )
        .init();

    let bob_port = env_port("BOB_PORT", 35072);
    let config = Config::local("bob", bob_port);
    let mut bob = StreamPeer::with_config(config).await?;

    println!(
        "[BOB] Listening on {} (will not accept incoming call)",
        bob_port
    );

    // Wait for Alice's INVITE to land, then just drop the IncomingCall.
    // We deliberately do NOT call accept() or reject() — we let the 180
    // ringback carry and wait for Alice's CANCEL.
    match timeout(Duration::from_secs(6), bob.wait_for_incoming()).await {
        Ok(Ok(incoming)) => {
            println!(
                "[BOB] Incoming call from {} — holding without accepting",
                incoming.from
            );
            // Hold the IncomingCall guard so the session stays alive.
            // Sleep past the window in which Alice will send CANCEL + observe
            // the result. Bob exits normally afterwards.
            sleep(Duration::from_secs(10)).await;
            drop(incoming);
        }
        Ok(Err(e)) => {
            eprintln!("[BOB] wait_for_incoming error: {}", e);
            std::process::exit(1);
        }
        Err(_) => {
            eprintln!("[BOB] never saw an incoming call");
            std::process::exit(1);
        }
    }

    println!("[BOB] Done.");
    std::process::exit(0);
}
