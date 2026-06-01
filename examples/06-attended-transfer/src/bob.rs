//! Attended transfer — **Bob** (the transferor).
//!
//! Answers Alice, then places a *consultation* call to Charlie. Once the
//! consultation is up, Bob reads the consultation dialog's identity, formats it
//! as an RFC 3891 `Replaces` value, and issues an attended transfer on the
//! original (Alice) leg with `transfer_attended` — sending Alice a REFER whose
//! Refer-To embeds the Replaces. Alice then replaces the consultation leg.

use rvoip_sip::{Config, StreamPeer};
use tokio::time::{sleep, Duration};

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

    let bob_port = env_port("BOB_PORT", 5061);
    let charlie_port = env_port("CHARLIE_PORT", 5062);

    let mut bob = StreamPeer::with_config(Config::local("bob", bob_port)).await?;
    println!("[BOB] Waiting for call...");

    let incoming = bob.wait_for_incoming().await?;
    println!("[BOB] Call from {}", incoming.from);
    let alice_leg = incoming.accept().await?;

    sleep(Duration::from_secs(1)).await;
    println!("[BOB] Consulting Charlie...");
    let consult_id = bob
        .invite(format!("sip:charlie@127.0.0.1:{charlie_port}"))
        .send()
        .await?;
    let consult = bob.coordinator().session(&consult_id);
    bob.wait_for_answered(consult.id()).await?;
    println!("[BOB] Consultation with Charlie established.");

    sleep(Duration::from_secs(1)).await;
    let replaces = consult
        .dialog_identity()
        .await?
        .and_then(|id| id.to_replaces_value())
        .ok_or("consultation dialog identity not confirmed yet")?;
    println!("[BOB] Attended-transferring Alice to Charlie (Replaces={replaces})");
    alice_leg
        .transfer_attended(&format!("sip:charlie@127.0.0.1:{charlie_port}"), &replaces)
        .await?;

    // Give Alice time to place the replacing INVITE, then drop both legs.
    sleep(Duration::from_secs(3)).await;
    let _ = alice_leg.hangup_and_wait(Some(Duration::from_secs(5))).await;
    let _ = consult.hangup_and_wait(Some(Duration::from_secs(5))).await;
    println!("[BOB] Done.");
    std::process::exit(0);
}
