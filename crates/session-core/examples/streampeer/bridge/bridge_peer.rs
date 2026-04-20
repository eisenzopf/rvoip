//! Bridge example — b2bua-style peer that terminates the inbound call from
//! Alice, originates a new outbound call to Carol, and relays RTP between
//! the two legs via `UnifiedCoordinator::bridge()`. Uses the per-call
//! event filter (`events_for_session`) to track each leg independently.
//!
//! Sequencing:
//! 1. Wait for Alice's INVITE (`Event::IncomingCall`).
//! 2. Originate outbound to Carol with `make_call`.
//! 3. Wait for Carol to answer (`CallAnswered` on the outbound leg).
//! 4. Accept Alice's INVITE.
//! 5. Poll both legs until `CallState::Active`, then call `bridge()`.
//! 6. Hold the bridge open long enough for the tones to flow through.
//! 7. Drop the bridge handle (closes the relay) and hang up both legs.

use rvoip_session_core::{CallState, Config, Event, SessionId, UnifiedCoordinator};
use std::time::Duration;
use tokio::time::{sleep, timeout};

fn env_u16(k: &str, default: u16) -> u16 {
    std::env::var(k).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

async fn wait_for_state(
    coord: &UnifiedCoordinator,
    session: &SessionId,
    target: CallState,
    deadline: Duration,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let end = tokio::time::Instant::now() + deadline;
    loop {
        let st = coord.get_state(session).await?;
        if st == target {
            return Ok(());
        }
        if tokio::time::Instant::now() >= end {
            return Err(format!(
                "session {} never reached {:?} (stuck at {:?})",
                session.0, target, st
            )
            .into());
        }
        sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()))
        .init();

    let bridge_port = env_u16("BRIDGE_SIP_PORT", 35592);
    let carol_port = env_u16("CAROL_SIP_PORT", 35591);
    let media_start = env_u16("BRIDGE_MEDIA_PORT_START", 35760);
    let media_end = env_u16("BRIDGE_MEDIA_PORT_END", 35810);
    let call_duration_secs = env_u16("BRIDGE_CALL_DURATION_SECS", 4) as u64;

    let coord = UnifiedCoordinator::new(Config {
        media_port_start: media_start,
        media_port_end: media_end,
        ..Config::local("bridge", bridge_port)
    })
    .await?;

    // Unfiltered receiver used only to catch the first IncomingCall —
    // per-leg streams take over once the session IDs are known.
    let mut events = coord.events().await?;

    println!("[BRIDGE] Listening on 127.0.0.1:{}...", bridge_port);

    let inbound_id = loop {
        match events.next().await {
            Some(Event::IncomingCall { call_id, from, .. }) => {
                println!("[BRIDGE] Incoming call from {} (leg A = {})", from, call_id.0);
                break call_id;
            }
            Some(_) => continue,
            None => return Err("event stream closed before incoming".into()),
        }
    };

    // Open per-session streams so inbound and outbound can be observed
    // independently — this is exactly the Item 1 use case.
    let mut inbound_events = coord.events_for_session(&inbound_id).await?;

    // Originate outbound to Carol.
    let outbound_target = format!("sip:carol@127.0.0.1:{}", carol_port);
    println!("[BRIDGE] Calling Carol at {}", outbound_target);
    let outbound_id = coord
        .make_call(
            &format!("sip:bridge@127.0.0.1:{}", bridge_port),
            &outbound_target,
        )
        .await?;
    println!("[BRIDGE] Outbound leg B = {}", outbound_id.0);

    let mut outbound_events = coord.events_for_session(&outbound_id).await?;

    // Wait for Carol to answer. Bound the wait so a hung outbound leg
    // doesn't park the bridge forever.
    println!("[BRIDGE] Waiting for Carol to answer...");
    let answer_deadline = Duration::from_secs(15);
    let answered = timeout(answer_deadline, async {
        loop {
            match outbound_events.next().await? {
                Event::CallAnswered { .. } => return Some(()),
                Event::CallEnded { .. } | Event::CallFailed { .. } => return None,
                _ => continue,
            }
        }
    })
    .await;
    match answered {
        Ok(Some(())) => println!("[BRIDGE] Carol answered"),
        Ok(None) => return Err("outbound leg terminated before answering".into()),
        Err(_) => return Err("outbound leg answer timeout".into()),
    }

    // Now accept Alice's INVITE.
    println!("[BRIDGE] Accepting Alice's call");
    coord.accept_call(&inbound_id).await?;

    // Both legs must be Active before we bridge — inbound transitions via
    // the 200-OK → ACK round-trip, outbound via the answer above.
    wait_for_state(&coord, &inbound_id, CallState::Active, Duration::from_secs(5)).await?;
    wait_for_state(&coord, &outbound_id, CallState::Active, Duration::from_secs(5)).await?;

    println!("[BRIDGE] Both legs active — bridging RTP streams");
    let bridge = coord.bridge(&inbound_id, &outbound_id).await?;
    println!(
        "[BRIDGE] Bridge established ({:?} <-> {:?})",
        bridge.sessions().0, bridge.sessions().1
    );

    // Let audio flow end-to-end for the configured duration.
    sleep(Duration::from_secs(call_duration_secs)).await;

    // Drop the bridge first so the relay cancel gate flips before we
    // tear the legs down. Hanging up while the bridge is still active
    // would leave forwarder tasks pointing at a dead session until they
    // notice the broadcast closing.
    drop(bridge);

    println!("[BRIDGE] Tearing down both legs");
    let _ = coord.hangup(&inbound_id).await;
    let _ = coord.hangup(&outbound_id).await;

    // Drain a few events so the BYE/200 OK exchanges complete before
    // the coordinator shuts down. `_` bindings silence unused-result
    // warnings for timeouts that are expected to fire.
    let _ = timeout(Duration::from_secs(2), inbound_events.next()).await;
    let _ = timeout(Duration::from_secs(2), outbound_events.next()).await;
    sleep(Duration::from_millis(500)).await;

    println!("[BRIDGE] Done.");
    std::process::exit(0);
}
