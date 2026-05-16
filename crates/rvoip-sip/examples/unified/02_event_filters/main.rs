//! UnifiedCoordinator event filtering.
//!
//! Run with:
//!
//!   cargo run -p rvoip-sip --example unified_event_filters

use std::time::Duration;

use rvoip_sip::{Config, Event, Result, SessionError, UnifiedCoordinator};

#[tokio::main]
async fn main() -> Result<()> {
    let bob = UnifiedCoordinator::new(Config::local("bob", 5141)).await?;
    let mut global_events = bob.events().await?;
    let bob_task = {
        let bob = bob.clone();
        tokio::spawn(async move {
            while let Some(event) = global_events.next().await {
                if let Event::IncomingCall { call_id, from, .. } = event {
                    println!("[bob/global] incoming {call_id} from {from}");
                    bob.accept_call(&call_id).await?;
                    return Ok::<_, rvoip_sip::SessionError>(());
                }
            }
            Err(rvoip_sip::SessionError::Other(
                "global event stream closed".to_string(),
            ))
        })
    };

    tokio::time::sleep(Duration::from_millis(300)).await;

    let alice = UnifiedCoordinator::new(Config::local("alice", 5140)).await?;
    let call_id = alice
        .invite(
            Some("sip:alice@127.0.0.1:5140".to_string()),
            "sip:bob@127.0.0.1:5141",
        )
        .send()
        .await?;
    let mut call_events = alice.events_for_session(&call_id).await?;

    loop {
        match call_events.next().await {
            Some(Event::CallAnswered { call_id, .. }) => {
                println!("[alice/filtered] {call_id} answered");
                break;
            }
            Some(Event::CallFailed { reason, .. }) => {
                return Err(SessionError::Other(format!("call failed: {reason}")));
            }
            Some(_) => {}
            None => return Err(SessionError::Other("filtered event stream closed".to_string())),
        }
    }

    alice.hangup(&call_id).await?;
    alice.shutdown_gracefully(Some(Duration::from_secs(2))).await?;
    bob.shutdown_gracefully(Some(Duration::from_secs(2))).await?;
    bob_task
        .await
        .map_err(|err| SessionError::Other(err.to_string()))??;
    Ok(())
}
