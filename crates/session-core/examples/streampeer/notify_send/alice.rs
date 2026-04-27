//! NOTIFY send — Alice.
//!
//! Alice calls Bob, waits for the call to be answered, sends a NOTIFY on
//! the established dialog, and exits cleanly. The NOTIFY round-trip is
//! asserted from Bob's side (see `bob.rs`).

use rvoip_session_core::{Config, Event, StreamPeer};
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

    let alice_port = env_port("ALICE_PORT", 35091);
    let bob_port = env_port("BOB_PORT", 35092);

    let config = Config::local("alice", alice_port);
    let mut alice = StreamPeer::with_config(config).await?;
    let mut events = alice.control().subscribe_events().await?;

    println!("[ALICE] Calling Bob on port {}…", bob_port);
    let handle = alice
        .call(&format!("sip:bob@127.0.0.1:{}", bob_port))
        .await?;

    let answered = timeout(Duration::from_secs(8), async {
        loop {
            match events.next().await {
                Some(Event::CallAnswered { .. }) => return true,
                Some(Event::CallFailed { .. }) | Some(Event::CallEnded { .. }) => return false,
                Some(_) => continue,
                None => return false,
            }
        }
    })
    .await
    .unwrap_or(false);
    if !answered {
        eprintln!("[ALICE] did not see CallAnswered within 8s");
        std::process::exit(1);
    }

    // Let ACK land before sending NOTIFY so the dialog is fully confirmed
    // on both sides.
    sleep(Duration::from_millis(300)).await;

    println!("[ALICE] Sending NOTIFY (event=dialog, subscription-state=active;expires=3600)…");
    handle
        .send_notify(
            "dialog",
            Some("<dialog-info/>".to_string()),
            Some("active;expires=3600".to_string()),
        )
        .await?;

    // Brief delay so Bob's cross-crate dispatch observes the NOTIFY
    // before we tear the call down.
    sleep(Duration::from_millis(500)).await;
    handle.hangup().await?;

    println!("[ALICE] Done.");
    std::process::exit(0);
}
