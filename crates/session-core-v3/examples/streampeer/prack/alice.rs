//! PRACK caller (Alice) — configured with `use_100rel: NotSupported`.
//!
//! Alice's INVITE therefore advertises no `100rel` option tag. Bob is
//! configured with `Required`, so he MUST reply 420 Bad Extension per
//! RFC 3262 §4. Alice asserts she received `CallFailed { status_code: 420 }`
//! and exits 0 on success.

use rvoip_session_core_v3::{Config, Event, RelUsage, StreamPeer};
use tokio::time::{timeout, Duration};

fn env_port(key: &str, default: u16) -> u16 {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()))
        .init();

    let alice_port = env_port("ALICE_PORT", 35063);
    let bob_port = env_port("BOB_PORT", 35064);

    let mut config = Config::local("alice", alice_port);
    config.use_100rel = RelUsage::NotSupported;

    let mut alice = StreamPeer::with_config(config).await?;
    let mut events = alice.control().subscribe_events().await?;

    println!("[ALICE] Calling Bob (100rel=NotSupported; expecting 420)…");
    let _handle = alice.call(&format!("sip:bob@127.0.0.1:{}", bob_port)).await?;

    let outcome = timeout(Duration::from_secs(8), async {
        loop {
            match events.next().await {
                Some(Event::CallFailed { status_code, .. }) => return Some(status_code),
                Some(Event::CallAnswered { .. }) => return None,
                Some(_) => continue,
                None => return None,
            }
        }
    })
    .await;

    match outcome {
        Ok(Some(420)) => {
            println!("[ALICE] Received expected 420 Bad Extension.");
            std::process::exit(0);
        }
        Ok(Some(code)) => {
            eprintln!("[ALICE] Got CallFailed with unexpected status {}", code);
            std::process::exit(1);
        }
        Ok(None) => {
            eprintln!("[ALICE] Call answered unexpectedly or event stream ended");
            std::process::exit(1);
        }
        Err(_) => {
            eprintln!("[ALICE] Timed out waiting for CallFailed");
            std::process::exit(1);
        }
    }
}
