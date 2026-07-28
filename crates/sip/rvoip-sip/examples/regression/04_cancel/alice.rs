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
//! replies 487 Request Terminated to the INVITE, and rvoip-sip
//! surfaces that as a distinct `CallCancelled` event (not `CallFailed`)
//! so UIs can render "missed call" differently.

use rvoip_sip::{Config, Event, StreamPeer};
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
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_sip_dialog=error".into()),
        )
        .init();

    let alice_port = env_port("ALICE_PORT", 35071);
    let bob_port = env_port("BOB_PORT", 35072);

    let config = Config::local("alice", alice_port);
    let alice = StreamPeer::with_config(config).await?;

    println!("[ALICE] Calling Bob on port {}…", bob_port);
    let call_id = alice
        .invite(format!("sip:bob@127.0.0.1:{}", bob_port))
        .send()
        .await?;
    let handle = alice.coordinator().session(&call_id);

    // No public "ringing started" event — dialog-core's 180 is consumed
    // internally but the session state machine transitions Initiating →
    // Ringing on 180. Poll until we're in Ringing before hangup; the
    // `UAC/Ringing/HangupCall` transition is the one that dispatches
    // CANCEL. (The Initiating variant is intentionally absent for
    // reasons described in `state_tables/default.yaml`.)
    let ringing_reached = {
        use rvoip_sip::CallState;
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
    println!("[ALICE] Reached Ringing state — sending CANCEL via handle.hangup_and_wait()…");

    let mut events = alice.coordinator().events_for_session(&call_id).await?;

    // `hangup_and_wait()` from Ringing routes to the CANCEL wire path and
    // waits for the terminal CallCancelled event.
    match handle.hangup_and_wait(Some(Duration::from_secs(8))).await {
        Ok(reason) if reason == "Cancelled" => {}
        Ok(other) => {
            eprintln!("[ALICE] expected CallCancelled, got terminal reason {other:?}");
            std::process::exit(1);
        }
        Err(err) => {
            eprintln!("[ALICE] hangup_and_wait failed: {err}");
            std::process::exit(1);
        }
    }

    let first_terminal = timeout(Duration::from_secs(5), async {
        loop {
            match events.next().await {
                Some(event @ Event::CallEnded { .. })
                | Some(event @ Event::CallFailed { .. })
                | Some(event @ Event::CallCancelled { .. }) => return event,
                Some(_) => {}
                None => panic!("terminal event stream closed"),
            }
        }
    })
    .await?;
    if !matches!(first_terminal, Event::CallCancelled { .. }) {
        eprintln!("[ALICE] expected CallCancelled, got {first_terminal:?}");
        std::process::exit(1);
    }

    let duplicate = timeout(Duration::from_millis(500), async {
        loop {
            match events.next().await {
                Some(event @ Event::CallEnded { .. })
                | Some(event @ Event::CallFailed { .. })
                | Some(event @ Event::CallCancelled { .. }) => return event,
                Some(_) => {}
                None => panic!("terminal event stream closed while checking duplicates"),
            }
        }
    })
    .await;
    if let Ok(event) = duplicate {
        eprintln!("[ALICE] duplicate terminal event: {event:?}");
        std::process::exit(1);
    }

    println!("[ALICE] Got exactly one CallCancelled — expected outcome.");
    Ok(())
}
