//! Endpoint incoming redirect example.
//!
//! Run in one terminal:
//!
//!   cargo run --example endpoint_incoming_redirect
//!
//! Then send an INVITE to `sip:frontdesk@127.0.0.1:5088`.

use rvoip_session_core::{Config, Endpoint, EndpointProfile};

#[tokio::main]
async fn main() -> rvoip_session_core::Result<()> {
    let mut endpoint = Endpoint::builder()
        .name("frontdesk")
        .profile(EndpointProfile::Custom(Config::local("frontdesk", 5088)))
        .build()
        .await?;

    println!("waiting for call to sip:frontdesk@127.0.0.1:5088");
    let incoming = endpoint.wait_for_incoming().await?;
    println!("redirecting {} to voicemail", incoming.from);
    incoming.redirect_to("sip:voicemail@127.0.0.1:5099").await?;
    endpoint.shutdown().await
}
