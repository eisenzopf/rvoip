//! PRACK callee (Bob) — configured with `use_100rel: Required`.
//!
//! Any INVITE that doesn't carry `Supported: 100rel` (or `Require: 100rel`)
//! is rejected with 420 Bad Extension + `Unsupported: 100rel` per RFC 3262
//! §4. Bob just waits long enough for Alice to observe the failure, then
//! exits.

use rvoip_session_core_v3::{Config, RelUsage, StreamPeer};
use tokio::time::{sleep, Duration};

fn env_port(key: &str, default: u16) -> u16 {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()))
        .init();

    let bob_port = env_port("BOB_PORT", 35064);

    let mut config = Config::local("bob", bob_port);
    config.use_100rel = RelUsage::Required;

    let _bob = StreamPeer::with_config(config).await?;
    println!("[BOB] Listening on {} (100rel=Required)", bob_port);

    // Give Alice time to send her INVITE, receive the 420, and exit.
    sleep(Duration::from_secs(8)).await;

    println!("[BOB] Done.");
    std::process::exit(0);
}
