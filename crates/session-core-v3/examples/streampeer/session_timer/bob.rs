//! RFC 4028 session-timer callee (Bob) — accepts the call, stays alive
//! long enough for Alice to observe a refresh cycle, then exits.

use rvoip_session_core_v3::{Config, StreamPeer};
use tokio::time::{sleep, Duration};

fn env_port(key: &str, default: u16) -> u16 {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()))
        .init();

    let bob_port = env_port("BOB_PORT", 35066);

    let mut config = Config::local("bob", bob_port);
    config.session_timer_secs = Some(10);
    config.session_timer_min_se = 5;

    let mut bob = StreamPeer::with_config(config).await?;
    println!("[BOB] Listening on {} (session timer = 10s)", bob_port);

    let incoming = bob.wait_for_incoming().await?;
    println!("[BOB] Call from {}", incoming.from);
    let handle = incoming.accept().await?;

    // Stay alive long enough for one refresh cycle. Alice hangs up on her
    // side — we just sit and answer any UPDATE refreshes that arrive.
    let _ = tokio::time::timeout(Duration::from_secs(20), bob.wait_for_ended(handle.id())).await;
    sleep(Duration::from_millis(100)).await;

    println!("[BOB] Done.");
    std::process::exit(0);
}
