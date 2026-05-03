//! Basic UnifiedCoordinator call.
//!
//! Run with:
//!
//!   cargo run -p rvoip-session-core --example unified_basic_call

use std::time::Duration;

use rvoip_session_core::{Config, Event, Result, SessionError, UnifiedCoordinator};

#[tokio::main]
async fn main() -> Result<()> {
    let bob = UnifiedCoordinator::new(Config::local("bob", 5131)).await?;
    let mut bob_events = bob.events().await?;
    let bob_task = {
        let bob = bob.clone();
        tokio::spawn(async move {
            while let Some(event) = bob_events.next().await {
                if let Event::IncomingCall { call_id, from, .. } = event {
                    println!("[bob] accepting call from {from}");
                    bob.accept_call(&call_id).await?;
                    return Ok::<_, rvoip_session_core::SessionError>(());
                }
            }
            Err(rvoip_session_core::SessionError::Other(
                "bob event stream closed".to_string(),
            ))
        })
    };

    tokio::time::sleep(Duration::from_millis(300)).await;

    let alice = UnifiedCoordinator::new(Config::local("alice", 5130)).await?;
    let call_id = alice
        .make_call(
            "sip:alice@127.0.0.1:5130",
            "sip:bob@127.0.0.1:5131",
        )
        .await?;

    let mut call_events = alice.events_for_session(&call_id).await?;
    loop {
        match call_events.next().await {
            Some(Event::CallAnswered { .. }) => break,
            Some(Event::CallFailed { reason, .. }) => {
                return Err(SessionError::Other(format!("call failed: {reason}")));
            }
            Some(_) => {}
            None => return Err(SessionError::Other("alice event stream closed".to_string())),
        }
    }

    println!("[alice] connected as {}", call_id);
    alice.hangup(&call_id).await?;
    alice.shutdown_gracefully(Some(Duration::from_secs(2))).await?;
    bob.shutdown_gracefully(Some(Duration::from_secs(2))).await?;

    bob_task
        .await
        .map_err(|err| SessionError::Other(err.to_string()))??;
    Ok(())
}
