//! Attended transfer — **Alice** (the transferee).
//!
//! Calls Bob, then waits for the attended-transfer REFER. The REFER's Refer-To
//! carries an embedded `Replaces` pointing at Bob's consultation dialog with
//! Charlie, so Alice completes the transfer by placing a fresh INVITE to the
//! Refer-To target — which replaces that consultation leg.

use rvoip_sip::{Config, Event, StreamPeer};
use tokio::time::{timeout, Duration};

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

    // Defaults avoid the combined examples-smoke ports used by quickstart-p2p
    // (5060/5061) and secure-call-srtp (5060).
    let alice_port = env_port("ALICE_PORT", 5080);
    let bob_port = env_port("BOB_PORT", 5081);

    let mut alice = StreamPeer::with_config(Config::local("alice", alice_port)).await?;

    println!("[ALICE] Calling Bob...");
    let call_id = alice
        .invite(format!("sip:bob@127.0.0.1:{bob_port}"))
        .send()
        .await?;
    // Prefer SessionHandle::wait_for_answered: it observes authoritative
    // lifecycle evidence and does not miss CallAnswered if the event already
    // fired. StreamPeer::wait_for_answered only watches the event stream.
    let bob_leg = alice.coordinator().session(&call_id);
    bob_leg
        .wait_for_answered(Some(Duration::from_secs(20)))
        .await?;
    println!("[ALICE] Connected to Bob.");

    // Bound the REFER wait: the combined examples-smoke hang was Alice waiting
    // here forever after dialing a stale peer on recycled 5060/5061 ports.
    println!("[ALICE] Waiting for attended transfer...");
    let mut events = alice.control().subscribe_events().await?;
    let (refer_to, replaces) = timeout(Duration::from_secs(45), async {
        loop {
            match events.next().await {
                Some(Event::ReferReceived {
                    refer_to, replaces, ..
                }) => return Ok::<_, String>((refer_to, replaces)),
                Some(Event::CallEnded { .. }) => {
                    return Err("call ended before attended-transfer REFER".into());
                }
                None => return Err("event stream closed before attended-transfer REFER".into()),
                _ => {}
            }
        }
    })
    .await
    .map_err(|_| "timed out waiting for attended-transfer REFER".to_string())??;
    println!("[ALICE] Got REFER to {refer_to}");
    println!("[ALICE] Replaces = {replaces:?}");

    // Place the replacing INVITE while the original Bob leg is still up. Hanging
    // up first races Bob's consultation teardown and can leave Charlie answered
    // without Alice observing lifecycle answer evidence.
    println!("[ALICE] Completing transfer — calling the Refer-To target...");
    let charlie_id = alice.invite(refer_to.clone()).send().await?;
    let charlie = alice.coordinator().session(&charlie_id);
    charlie
        .wait_for_answered(Some(Duration::from_secs(30)))
        .await?;
    println!("[ALICE] ✅ Connected to Charlie (attended transfer complete).");

    let _ = bob_leg.hangup().await;
    let _ = bob_leg.wait_for_end(Some(Duration::from_secs(5))).await;

    tokio::time::sleep(Duration::from_secs(1)).await;
    let _ = charlie.hangup().await;
    let _ = charlie.wait_for_end(Some(Duration::from_secs(10))).await;

    println!("[ALICE] Done.");
    Ok(())
}
