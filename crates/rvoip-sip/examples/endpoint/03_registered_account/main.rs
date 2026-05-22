//! Endpoint registered-account example.
//!
//! Required environment:
//! - `SIP_REGISTRAR`, for example `sip:pbx.example.com` or `sips:pbx.example.com:5061`
//! - `SIP_USERNAME`
//! - `SIP_PASSWORD`
//! - `SIP_TARGET`, an extension such as `1002` or a full SIP URI
//!
//! Run with:
//!
//!   cargo run --example endpoint_registered_account

use std::time::Duration;

use rvoip_sip::{Endpoint, EndpointProfile};

#[tokio::main]
async fn main() -> rvoip_sip::Result<()> {
    let registrar = env("SIP_REGISTRAR")?;
    let username = env("SIP_USERNAME")?;
    let password = env("SIP_PASSWORD")?;
    let target = env("SIP_TARGET")?;

    let mut endpoint = Endpoint::builder()
        .name(&username)
        .account(&username)
        .password(password)
        .registrar(registrar)
        .profile(EndpointProfile::LanPbx)
        .advertised_addr(
            std::env::var("SIP_ADVERTISED_ADDR")
                .unwrap_or_else(|_| "127.0.0.1:5060".to_string())
                .parse::<std::net::SocketAddr>()
                .map_err(|err| rvoip_sip::SessionError::ConfigError(err.to_string()))?,
        )
        .build()
        .await?;

    endpoint.register().await?;
    let call = endpoint
        .call_and_wait(&target, Some(Duration::from_secs(30)))
        .await?;
    println!("connected call {}", call.id());
    call.hangup_and_wait(Some(Duration::from_secs(5))).await?;
    endpoint.unregister().await?;
    endpoint.shutdown().await
}

fn env(name: &str) -> rvoip_sip::Result<String> {
    std::env::var(name).map_err(|_| {
        rvoip_sip::SessionError::ConfigError(format!("{name} environment variable is required"))
    })
}
