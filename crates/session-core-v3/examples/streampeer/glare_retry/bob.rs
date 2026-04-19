//! RFC 3261 §14.1 glare callee (Bob).
//!
//! Bob accepts Alice's INVITE, reaches Active, then at the synchronized
//! `RVOIP_TEST_GLARE_START_MS` instant invokes `hold()` at the same time
//! as Alice. Each side's UAS sees an inbound re-INVITE while its own is
//! pending; the production HasPendingReinvite-guarded transition fires
//! 491 Request Pending, the peer retries after backoff, and both land on
//! OnHold.

use rvoip_session_core_v3::{CallState, Config, StreamPeer};
use tokio::time::{sleep, timeout, Duration, Instant as TokioInstant};

fn env_port(key: &str, default: u16) -> u16 {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

fn env_u64(key: &str) -> Option<u64> {
    std::env::var(key).ok().and_then(|s| s.parse().ok())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()),
        )
        .init();

    let bob_port = env_port("BOB_PORT", 35074);
    let mut bob = StreamPeer::with_config(Config::local("bob", bob_port)).await?;
    println!("[BOB] Listening on {}", bob_port);

    let incoming = match timeout(Duration::from_secs(8), bob.wait_for_incoming()).await {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => {
            eprintln!("[BOB] wait_for_incoming error: {}", e);
            std::process::exit(1);
        }
        Err(_) => {
            eprintln!("[BOB] timeout waiting for incoming INVITE");
            std::process::exit(1);
        }
    };
    println!("[BOB] Incoming call from {}", incoming.from);
    let handle = incoming.accept().await?;

    let active_reached = {
        let mut ok = false;
        for _ in 0..40 {
            if matches!(handle.state().await, Ok(CallState::Active)) {
                ok = true;
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        ok
    };
    if !active_reached {
        eprintln!("[BOB] never reached Active within 4s");
        std::process::exit(1);
    }

    if let Some(start_ms) = env_u64("RVOIP_TEST_GLARE_START_MS") {
        let delta = start_ms.saturating_sub(now_ms());
        if delta > 0 {
            sleep(Duration::from_millis(delta)).await;
        }
    } else {
        sleep(Duration::from_millis(200)).await;
    }

    println!("[BOB] Calling hold() (glare window)…");
    // See alice.rs — same retry rationale for heavy-concurrency CI.
    if let Err(e) = handle.hold().await {
        eprintln!("[BOB] hold() returned error on first try: {} — retrying in 500ms", e);
        sleep(Duration::from_millis(500)).await;
        if let Err(e2) = handle.hold().await {
            eprintln!("[BOB] hold() retry also failed: {}", e2);
            std::process::exit(1);
        }
    }

    let deadline = TokioInstant::now() + Duration::from_secs(20);
    let mut observed_on_hold = false;
    while TokioInstant::now() < deadline {
        if handle.is_on_hold().await {
            observed_on_hold = true;
            break;
        }
        sleep(Duration::from_millis(200)).await;
    }

    if !observed_on_hold {
        eprintln!("[BOB] call did not reach OnHold within 20s");
        std::process::exit(1);
    }

    println!("[BOB] OnHold — waiting for Alice to hang up.");
    // Let Alice drive the hangup — Bob just keeps the call alive past the
    // glare/retry window and exits when Alice tears down.
    let _ = timeout(Duration::from_secs(15), bob.wait_for_ended(handle.id())).await;
    std::process::exit(0);
}
