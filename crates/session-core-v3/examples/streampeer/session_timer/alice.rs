//! RFC 4028 session-timer caller (Alice) — configured with a 10-second
//! Session-Expires. After the call is answered Alice subscribes to events
//! and asserts at least one `SessionRefreshed` event fires within the
//! expected window (half-expiry = 5 s), then hangs up.

use rvoip_session_core_v3::{Config, Event, StreamPeer};
use tokio::time::{timeout, Duration};

fn env_port(key: &str, default: u16) -> u16 {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()))
        .init();

    let alice_port = env_port("ALICE_PORT", 35065);
    let bob_port = env_port("BOB_PORT", 35066);

    let mut config = Config::local("alice", alice_port);
    config.session_timer_secs = Some(10);
    config.session_timer_min_se = 5;

    let mut alice = StreamPeer::with_config(config).await?;
    let mut events = alice.control().subscribe_events().await?;

    println!("[ALICE] Calling Bob (session timer = 10s)…");
    let handle = alice.call(&format!("sip:bob@127.0.0.1:{}", bob_port)).await?;
    alice.wait_for_answered(handle.id()).await?;
    println!("[ALICE] Connected.");

    // Expect a refresh event within ~6 seconds (half-expiry + slack).
    let outcome = timeout(Duration::from_secs(12), async {
        loop {
            match events.next().await {
                Some(Event::SessionRefreshed { expires_secs, .. }) => return Some(expires_secs),
                Some(Event::SessionRefreshFailed { reason, .. }) => {
                    eprintln!("[ALICE] Unexpected SessionRefreshFailed: {}", reason);
                    return None;
                }
                Some(Event::CallEnded { .. }) => {
                    eprintln!("[ALICE] Call ended before refresh observed");
                    return None;
                }
                Some(_) => continue,
                None => return None,
            }
        }
    })
    .await;

    match outcome {
        Ok(Some(secs)) => {
            println!("[ALICE] Session refreshed (expires={}s). Hanging up.", secs);
            handle.hangup().await?;
            let _ = alice.wait_for_ended(handle.id()).await;
            std::process::exit(0);
        }
        _ => {
            eprintln!("[ALICE] Did not receive SessionRefreshed within 12s");
            std::process::exit(1);
        }
    }
}
