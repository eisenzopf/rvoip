//! CANCEL caller (Alice).
//!
//! Alice INVITEs Bob. Bob has an incoming-call handler that sleeps without
//! ever accepting, so Alice stays in `Ringing` indefinitely. Once Alice
//! sees `CallStateChanged(Ringing)`, she calls `handle.hangup()`. Because
//! the call is not yet answered, the state machine dispatches CANCEL
//! under the hood (per the `UAC/Ringing/HangupCall` transition) and
//! Alice should observe `Event::CallCancelled`.
//!
//! RFC 3261 §9 — CANCEL cancels a pending INVITE. The responding UAS
//! replies 487 Request Terminated to the INVITE, and session-core
//! surfaces that as a distinct `CallCancelled` event (not `CallFailed`)
//! so UIs can render "missed call" differently.

use rvoip_session_core::{Config, Event, StreamPeer};
use tokio::time::{sleep, timeout, Duration};

fn env_port(key: &str, default: u16) -> u16 {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()),
        )
        .init();

    let alice_port = env_port("ALICE_PORT", 35071);
    let bob_port = env_port("BOB_PORT", 35072);

    let config = Config::local("alice", alice_port);
    let mut alice = StreamPeer::with_config(config).await?;
    let mut events = alice.control().subscribe_events().await?;

    println!("[ALICE] Calling Bob on port {}…", bob_port);
    let handle = alice.call(&format!("sip:bob@127.0.0.1:{}", bob_port)).await?;

    // No public "ringing started" event — dialog-core's 180 is consumed
    // internally but the session state machine transitions Initiating →
    // Ringing on 180. Poll until we're in Ringing before hangup; the
    // `UAC/Ringing/HangupCall` transition is the one that dispatches
    // CANCEL. (The Initiating variant is intentionally absent for
    // reasons described in `state_tables/default.yaml`.)
    let ringing_reached = {
        use rvoip_session_core::CallState;
        let mut reached = false;
        for _ in 0..40 {
            if matches!(handle.state().await, Ok(CallState::Ringing)) {
                reached = true;
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        reached
    };
    if !ringing_reached {
        eprintln!("[ALICE] call never reached Ringing state within 4s");
        std::process::exit(1);
    }
    println!("[ALICE] Reached Ringing state — sending CANCEL via handle.hangup()…");

    // `hangup()` from Ringing routes to the CANCEL wire path.
    handle.hangup().await?;

    let outcome = timeout(Duration::from_secs(8), async {
        loop {
            match events.next().await {
                Some(Event::CallCancelled { .. }) => return Outcome::Cancelled,
                Some(Event::CallFailed { status_code, .. }) => {
                    return Outcome::Failed(status_code);
                }
                Some(Event::CallEnded { .. }) => return Outcome::Ended,
                Some(_) => continue,
                None => return Outcome::StreamClosed,
            }
        }
    })
    .await;

    match outcome {
        Ok(Outcome::Cancelled) => {
            println!("[ALICE] Got CallCancelled — expected outcome.");
            std::process::exit(0);
        }
        Ok(other) => {
            eprintln!("[ALICE] expected CallCancelled, got {:?}", other);
            std::process::exit(1);
        }
        Err(_) => {
            eprintln!("[ALICE] timed out waiting for CallCancelled");
            std::process::exit(1);
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
enum Outcome {
    Cancelled,
    Failed(u16),
    Ended,
    StreamClosed,
}
