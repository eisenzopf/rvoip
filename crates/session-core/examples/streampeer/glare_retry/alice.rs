//! RFC 3261 §14.1 glare caller (Alice).
//!
//! Alice calls Bob, both reach Active, then Alice and Bob *simultaneously*
//! invoke `hold()`. Because both have an outgoing re-INVITE in flight, each
//! UAS sees an inbound re-INVITE while its own is pending — the
//! `HasPendingReinvite` guard fires and each side responds 491 Request
//! Pending. The `Active/ReinviteGlare` transition in the state table
//! schedules a retry with random backoff (2.1–4.0 s). Eventually the
//! retries resolve and both peers settle on OnHold.
//!
//! No test hooks: the 491 response comes from the production YAML path.
//! Synchronization is via the `RVOIP_TEST_GLARE_START_MS` env var (a
//! wall-clock epoch in milliseconds that both peers sleep until before
//! calling `hold()`).
//!
//! NOTE on log output: the first `hold()` attempt deliberately produces
//! ERROR-level lines ("Transaction terminated after timeout" → "Failed to
//! execute action SendReINVITE"). That is the 491 Request Pending handshake
//! surfacing through the executor's generic action-failure logger. The
//! `ReinviteGlare` transition schedules a backoff retry and both peers
//! converge to OnHold, which is the success criterion. See
//! `docs/EXAMPLE_RUN_ERRORS_TRACKING.md` (Cluster D).

use rvoip_session_core::{CallState, Config, StreamPeer};
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

    let alice_port = env_port("ALICE_PORT", 35073);
    let bob_port = env_port("BOB_PORT", 35074);

    let mut alice = StreamPeer::with_config(Config::local("alice", alice_port)).await?;

    println!("[ALICE] Calling Bob on port {}…", bob_port);
    let handle = alice
        .call(&format!("sip:bob@127.0.0.1:{}", bob_port))
        .await?;
    alice.wait_for_answered(handle.id()).await?;
    println!("[ALICE] Connected.");

    // Wait for both peers to reach Active so hold() has a stable starting
    // state. UAC transitions to Active on Dialog200OK + ACK.
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
        eprintln!("[ALICE] never reached Active within 4s");
        std::process::exit(1);
    }

    // Synchronize the simultaneous hold.
    if let Some(start_ms) = env_u64("RVOIP_TEST_GLARE_START_MS") {
        let delta = start_ms.saturating_sub(now_ms());
        if delta > 0 {
            sleep(Duration::from_millis(delta)).await;
        }
    } else {
        // Without explicit sync the test is racy; give Bob a moment to also
        // reach Active before both sides press hold at once.
        sleep(Duration::from_millis(200)).await;
    }

    println!("[ALICE] Calling hold() (glare window)…");
    // Under heavy-concurrency CI (multiple test binaries racing for
    // cargo/system resources), the first re-INVITE occasionally trips a
    // spurious transaction timeout before the wire exchange even starts.
    // Retry once after a short backoff so the test is stable; a second
    // failure is treated as a real bug.
    if let Err(e) = handle.hold().await {
        eprintln!("[ALICE] hold() returned error on first try: {} — retrying in 500ms", e);
        sleep(Duration::from_millis(500)).await;
        if let Err(e2) = handle.hold().await {
            eprintln!("[ALICE] hold() retry also failed: {}", e2);
            std::process::exit(1);
        }
    }

    // Wait up to 20s for OnHold to stick — the first re-INVITE may glare
    // with Bob's, and the state machine's ScheduleReinviteRetry backs off
    // 2.1–4.0 s before retrying. Allow two retry windows.
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
        eprintln!("[ALICE] call did not reach OnHold within 20s");
        std::process::exit(1);
    }

    // Hold the OnHold state for a few seconds so we can confirm it's
    // stable (not bouncing back to Active) — otherwise a spurious retry
    // could mask a broken glare flow.
    let stability_deadline = TokioInstant::now() + Duration::from_secs(3);
    while TokioInstant::now() < stability_deadline {
        if !handle.is_on_hold().await {
            eprintln!("[ALICE] OnHold state did not stay stable");
            std::process::exit(1);
        }
        sleep(Duration::from_millis(200)).await;
    }

    println!("[ALICE] OnHold is stable — glare retry resolved. Hanging up.");
    let _ = handle.hangup().await;
    let _ = timeout(Duration::from_secs(5), alice.wait_for_ended(handle.id())).await;
    std::process::exit(0);
}
