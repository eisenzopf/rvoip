//! Endpoint local-call example.
//!
//! Run with:
//!
//!   cargo run --example endpoint_local_call

use std::time::Duration;

use rvoip_session_core::{Config, Endpoint, EndpointProfile};

#[tokio::main]
async fn main() -> rvoip_session_core::Result<()> {
    let bob_task = tokio::spawn(async {
        let mut bob = Endpoint::builder()
            .name("bob")
            .profile(EndpointProfile::Custom(Config::local("bob", 5071)))
            .build()
            .await?;

        let incoming = bob.wait_for_incoming().await?;
        println!("[bob] incoming from {}", incoming.from);
        let call = incoming.accept().await?;
        call.wait_for_end(None).await?;
        bob.shutdown().await
    });

    tokio::time::sleep(Duration::from_millis(300)).await;

    let alice = Endpoint::builder()
        .name("alice")
        .profile(EndpointProfile::Custom(Config::local("alice", 5070)))
        .build()
        .await?;

    let call = alice
        .call_and_wait("sip:bob@127.0.0.1:5071", Some(Duration::from_secs(10)))
        .await?;
    println!("[alice] connected as {}", call.id());
    tokio::time::sleep(Duration::from_secs(1)).await;
    call.hangup_and_wait(Some(Duration::from_secs(5))).await?;
    alice.shutdown().await?;

    bob_task
        .await
        .map_err(|err| rvoip_session_core::SessionError::Other(err.to_string()))??;
    Ok(())
}
