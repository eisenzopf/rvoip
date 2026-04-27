//! Register session-core with a remote Asterisk SIP server using settings
//! from a .env file.
//!
//! Usage:
//!   cd crates/session-core/examples/asterisk
//!   cp .env.example .env       # edit values for your PBX
//!   cargo run -p rvoip-session-core --example asterisk_softphone

use std::net::IpAddr;
use std::time::Duration;

use rvoip_session_core::{types::Credentials, Config, Registration, StreamPeer};
use tokio::time::sleep;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _ = dotenvy::from_filename(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples/asterisk/.env"),
    );
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "info,rvoip_dialog_core=warn".into()),
        )
        .init();

    let sip_server = env_or("SIP_SERVER", "192.168.1.103");
    let sip_port: u16 = env_or("SIP_PORT", "5060").parse()?;
    let transport = env_or("SIP_TRANSPORT", "UDP").to_lowercase();
    let username = env_or("SIP_USERNAME", "1001");
    let auth_user = env_or("SIP_AUTH_USERNAME", &username);
    let password = env_or("SIP_PASSWORD", "password123");
    let local_ip: IpAddr = env_or("LOCAL_IP", "0.0.0.0").parse()?;
    let local_port: u16 = env_or("LOCAL_PORT", "5070").parse()?;
    let idle_secs: u64 = env_or("IDLE_SECS", "30").parse()?;

    let transport_suffix = match transport.as_str() {
        "tcp" => ";transport=tcp",
        _ => "",
    };
    let registrar_uri = format!("sip:{}:{}{}", sip_server, sip_port, transport_suffix);

    // Install default credentials so future outbound INVITE retries (added in
    // a later test) can authenticate against the same Asterisk box.
    let mut config = Config::on(&username, local_ip, local_port);
    config.local_uri = format!(
        "sip:{}@{}:{}{}",
        username, sip_server, sip_port, transport_suffix
    );
    config.credentials = Some(Credentials::new(&auth_user, &password));

    println!("[softphone] Local bind: {}:{}", local_ip, local_port);
    println!("[softphone] AOR:        sip:{}@{}", username, sip_server);
    println!("[softphone] Registrar:  {}", registrar_uri);
    println!("[softphone] Transport:  {}", transport.to_uppercase());

    let mut peer = StreamPeer::with_config(config).await?;

    println!("[softphone] Registering...");
    let reg = peer
        .register_with(Registration::new(&registrar_uri, &auth_user, &password))
        .await?;

    sleep(Duration::from_secs(2)).await;
    match peer.is_registered(&reg).await {
        Ok(true) => println!("[softphone] Registered."),
        Ok(false) => println!("[softphone] Registration not yet confirmed."),
        Err(e) => println!("[softphone] Registration check error: {}", e),
    }

    println!("[softphone] Holding registration for {}s...", idle_secs);
    sleep(Duration::from_secs(idle_secs)).await;

    println!("[softphone] Unregistering...");
    peer.unregister(&reg).await.ok();
    println!("[softphone] Done.");
    std::process::exit(0);
}
