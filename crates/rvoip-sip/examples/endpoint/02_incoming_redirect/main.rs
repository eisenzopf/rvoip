//! Endpoint incoming redirect example.
//!
//! Run with:
//!
//!   cargo run --example endpoint_incoming_redirect

use std::time::Duration;

use rvoip_sip::{Config, Endpoint, EndpointProfile, Result, SessionError};

#[tokio::main]
async fn main() -> Result<()> {
    let frontdesk_task = tokio::spawn(async {
        let mut frontdesk = Endpoint::builder()
            .name("frontdesk")
            .profile(EndpointProfile::Custom(Config::local("frontdesk", 5088)))
            .build()
            .await?;

        let incoming = frontdesk.wait_for_incoming().await?;
        println!("[frontdesk] redirecting {} to voicemail", incoming.from());
        incoming.redirect_to("sip:voicemail@127.0.0.1:5099").await?;
        frontdesk.shutdown().await
    });

    tokio::time::sleep(Duration::from_millis(300)).await;

    let caller = Endpoint::builder()
        .name("alice")
        .profile(EndpointProfile::Custom(Config::local("alice", 5087)))
        .build()
        .await?;

    let call_id = caller.invite("sip:frontdesk@127.0.0.1:5088")?.send().await?;
    let call = caller.wrap_call(call_id);
    frontdesk_task
        .await
        .map_err(|err| SessionError::Other(err.to_string()))??;

    println!("[alice] frontdesk sent redirect for {}", call.id());
    call.hangup().await.ok();
    caller.shutdown().await?;
    Ok(())
}
