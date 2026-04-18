//! PRACK callee (Bob) — two modes selected via `PRACK_MODE` env var.
//!
//! `PRACK_MODE=negative` (default): `use_100rel: Required`. Any INVITE
//! without `Supported: 100rel` is rejected with 420 Bad Extension per
//! RFC 3262 §4. Bob waits long enough for Alice to observe the failure.
//!
//! `PRACK_MODE=positive`: `use_100rel: Supported`. On `IncomingCall`, Bob
//! calls `send_early_media(None)` (reliable 183 with auto-negotiated SDP),
//! pauses briefly, then `accept()`s. Exercises the full UAS reliable-18x
//! path end-to-end.

use rvoip_session_core_v3::{Config, Event, RelUsage, StreamPeer};
use tokio::time::{sleep, timeout, Duration};

fn env_port(key: &str, default: u16) -> u16 {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

fn positive_mode() -> bool {
    std::env::var("PRACK_MODE")
        .map(|s| s.eq_ignore_ascii_case("positive"))
        .unwrap_or(false)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()))
        .init();

    let bob_port = env_port("BOB_PORT", 35064);
    let positive = positive_mode();

    let mut config = Config::local("bob", bob_port);
    config.use_100rel = if positive {
        RelUsage::Supported
    } else {
        RelUsage::Required
    };

    let mut bob = StreamPeer::with_config(config).await?;
    let control = bob.control().clone();

    if positive {
        println!("[BOB] Listening on {} (100rel=Supported, positive path)", bob_port);

        // Drive one call through reliable-183 → accept.
        match timeout(Duration::from_secs(10), bob.wait_for_incoming()).await {
            Ok(Ok(incoming)) => {
                println!("[BOB] Incoming call from {}", incoming.from);

                if let Err(e) = incoming.send_early_media(None).await {
                    eprintln!("[BOB] send_early_media failed: {}", e);
                    std::process::exit(1);
                }
                println!("[BOB] Sent reliable 183 with SDP; pausing before 200 OK…");

                // Simulated ringback playback window. Also gives Alice's
                // auto-PRACK time to round-trip before the 200 OK overtakes.
                sleep(Duration::from_millis(400)).await;

                if let Err(e) = incoming.accept().await {
                    eprintln!("[BOB] accept failed: {}", e);
                    std::process::exit(1);
                }
                println!("[BOB] Accepted. Holding the call briefly…");

                // Let the CallAnswered / ACK propagate on Alice's side.
                sleep(Duration::from_secs(2)).await;

                // Best-effort hangup via event subscription is overkill for
                // the test; dropping both peers via process exit is fine.
                let _ = control.subscribe_events().await.map(|mut rx| async move {
                    while let Some(ev) = rx.next().await {
                        if matches!(ev, Event::CallEnded { .. }) {
                            break;
                        }
                    }
                });
            }
            Ok(Err(e)) => {
                eprintln!("[BOB] wait_for_incoming error: {}", e);
                std::process::exit(1);
            }
            Err(_) => {
                eprintln!("[BOB] timed out waiting for incoming call");
                std::process::exit(1);
            }
        }
    } else {
        println!("[BOB] Listening on {} (100rel=Required, negative path)", bob_port);
        // Give Alice time to send her INVITE, receive the 420, and exit.
        sleep(Duration::from_secs(8)).await;
    }

    println!("[BOB] Done.");
    std::process::exit(0);
}
