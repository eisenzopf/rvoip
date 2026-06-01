//! Register to a registrar / PBX and place a call.
//!
//! Uses the [`Endpoint`] surface — the simplest account/profile API — to
//! REGISTER with credentials, dial an extension through the PBX, then
//! unregister and shut down. This is the everyday softphone-account flow.
//!
//! Driven entirely by environment variables so it works against any PBX:
//!
//! - `SIP_REGISTRAR` — e.g. `sip:pbx.example.com` or `sips:pbx.example.com:5061`
//! - `SIP_USERNAME`  — the account / extension to register as
//! - `SIP_PASSWORD`  — the account password (SIP digest auth)
//! - `SIP_TARGET`    — extension or full SIP URI to call
//! - `SIP_ADVERTISED_ADDR` — optional; the `host:port` to advertise in Contact
//!   (default `127.0.0.1:5060`)
//!
//! A copy-paste Asterisk-in-docker quickstart lives in this example's README.

use std::time::Duration;

use rvoip_sip::{Endpoint, EndpointProfile};

#[tokio::main]
async fn main() -> rvoip_sip::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .init();

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

    println!("registering {username} with the PBX…");
    endpoint.register().await?;
    println!("✅ registered; calling {target}…");

    let call = endpoint
        .call_and_wait(&target, Some(Duration::from_secs(30)))
        .await?;
    println!("✅ connected call {}", call.id());

    call.hangup_and_wait(Some(Duration::from_secs(5))).await?;
    endpoint.unregister().await?;
    println!("✅ unregistered, shutting down");
    endpoint.shutdown().await
}

fn env(name: &str) -> rvoip_sip::Result<String> {
    std::env::var(name).map_err(|_| {
        rvoip_sip::SessionError::ConfigError(format!("{name} environment variable is required"))
    })
}
