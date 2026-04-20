//! PRACK caller (Alice) — two modes selected via `PRACK_MODE` env var.
//!
//! `PRACK_MODE=negative` (default): `use_100rel: NotSupported`. Alice's
//! INVITE advertises no `100rel` option tag. Bob is `Required`, so he
//! MUST reply 420 Bad Extension per RFC 3262 §4. Alice asserts she
//! receives `CallFailed { status_code: 420 }` and exits 0.
//!
//! `PRACK_MODE=positive`: `use_100rel: Supported`. Bob's `on_incoming_call`
//! sends a reliable 183 with SDP before accepting. Alice's dialog-core
//! auto-PRACKs (Phase C.1.2), then the call answers normally. Alice
//! asserts she receives `CallAnswered` and exits 0.

use rvoip_session_core::{Config, Event, RelUsage, StreamPeer};
use tokio::time::{timeout, Duration};

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

    let alice_port = env_port("ALICE_PORT", 35063);
    let bob_port = env_port("BOB_PORT", 35064);
    let positive = positive_mode();

    let mut config = Config::local("alice", alice_port);
    config.use_100rel = if positive {
        RelUsage::Supported
    } else {
        RelUsage::NotSupported
    };

    let mut alice = StreamPeer::with_config(config).await?;
    let mut events = alice.control().subscribe_events().await?;

    if positive {
        println!("[ALICE] Calling Bob (PRACK_MODE=positive; expecting reliable 183 → PRACK → 200 OK)…");
    } else {
        println!("[ALICE] Calling Bob (100rel=NotSupported; expecting 420)…");
    }
    let _handle = alice.call(&format!("sip:bob@127.0.0.1:{}", bob_port)).await?;

    // Positive path allows more time because Bob sleeps mid-flow before
    // accepting (to let the ringback play in a real deployment).
    let deadline = if positive { Duration::from_secs(12) } else { Duration::from_secs(8) };

    let outcome = timeout(deadline, async {
        loop {
            match events.next().await {
                Some(Event::CallAnswered { .. }) => return Outcome::Answered,
                Some(Event::CallFailed { status_code, .. }) => return Outcome::Failed(status_code),
                Some(_) => continue,
                None => return Outcome::StreamClosed,
            }
        }
    })
    .await;

    let result = match outcome {
        Ok(o) => o,
        Err(_) => Outcome::Timeout,
    };

    if positive {
        match result {
            Outcome::Answered => {
                println!("[ALICE] Call answered after reliable 183.");
                std::process::exit(0);
            }
            other => {
                eprintln!("[ALICE] positive mode: expected Answered, got {:?}", other);
                std::process::exit(1);
            }
        }
    } else {
        match result {
            Outcome::Failed(420) => {
                println!("[ALICE] Received expected 420 Bad Extension.");
                std::process::exit(0);
            }
            other => {
                eprintln!("[ALICE] negative mode: expected Failed(420), got {:?}", other);
                std::process::exit(1);
            }
        }
    }
}

#[derive(Debug)]
enum Outcome {
    Answered,
    Failed(u16),
    StreamClosed,
    Timeout,
}
