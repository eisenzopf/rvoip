//! Register session-core with a remote Asterisk SIP server using settings
//! from a .env file.
//!
//! Usage:
//!   cd crates/session-core/examples/asterisk
//!   cp .env.example .env       # edit values for your PBX
//!   ./registration/run.sh

use std::net::IpAddr;
use std::time::Duration;

use std::path::PathBuf;

use rvoip_session_core::{types::Credentials, Config, Registration, SipTlsMode, StreamPeer};
use tokio::time::sleep;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

#[cfg(feature = "dev-insecure-tls")]
fn env_bool(key: &str, default: bool) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    match std::env::var(key) {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            other => Err(format!("{} must be boolean, got '{}'", key, other).into()),
        },
        Err(_) => Ok(default),
    }
}

fn optional_path(key: &str) -> Option<PathBuf> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _ = dotenvy::from_filename(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/asterisk/.env"),
    );
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,rvoip_dialog_core=warn".into()),
        )
        .init();

    let sip_server = env_or("SIP_SERVER", "192.168.1.103");
    let transport = env_or("SIP_TRANSPORT", "TLS").to_lowercase();
    let sip_port: u16 = if transport == "tls" {
        env_or("SIP_TLS_PORT", "5061").parse()?
    } else {
        env_or("SIP_PORT", "5060").parse()?
    };
    let username = env_or("SIP_USERNAME", "1001");
    let auth_user = env_or("SIP_AUTH_USERNAME", &username);
    let password = env_or("SIP_PASSWORD", "password123");
    let local_ip: IpAddr = env_or("LOCAL_IP", "0.0.0.0").parse()?;
    let local_port: u16 = env_or("LOCAL_PORT", "5070").parse()?;
    let advertised_ip: IpAddr = match std::env::var("ADVERTISED_IP") {
        Ok(value) => value.parse()?,
        Err(_) if !local_ip.is_unspecified() => local_ip,
        Err(_) => {
            return Err("ADVERTISED_IP is required when LOCAL_IP is 0.0.0.0 or ::".into());
        }
    };
    let idle_secs: u64 = env_or("IDLE_SECS", "30").parse()?;

    let is_tls = transport == "tls";
    let scheme = if is_tls { "sips" } else { "sip" };
    let transport_suffix = match transport.as_str() {
        "tcp" => ";transport=tcp",
        "tls" => ";transport=tls",
        _ => "",
    };
    let contact_port = local_port;
    let registrar_uri = format!("{}:{}:{}{}", scheme, sip_server, sip_port, transport_suffix);
    let aor_uri = format!("{}:{}@{}", scheme, username, sip_server);
    let contact_uri = format!(
        "{}:{}@{}:{}{}",
        scheme, username, advertised_ip, contact_port, transport_suffix
    );

    // Install default credentials so future outbound INVITE retries (added in
    // a later test) can authenticate against the same Asterisk box.
    let mut config = Config::on(&username, local_ip, local_port);
    config.local_uri = aor_uri.clone();
    config.contact_uri = Some(contact_uri.clone());
    config.credentials = Some(Credentials::new(&auth_user, &password));
    if is_tls {
        config.sip_tls_mode = SipTlsMode::ClientOnly;
        config.tls_extra_ca_path = optional_path("TLS_CA_PATH");
        config.tls_client_cert_path = optional_path("TLS_CLIENT_CERT_PATH");
        config.tls_client_key_path = optional_path("TLS_CLIENT_KEY_PATH");
        #[cfg(feature = "dev-insecure-tls")]
        {
            config.tls_insecure_skip_verify = env_bool("TLS_INSECURE", false)?;
        }
    }

    println!("[registration] Local bind: {}:{}", local_ip, local_port);
    println!("[registration] AOR:        {}", aor_uri);
    println!("[registration] Contact:    {}", contact_uri);
    println!("[registration] Registrar:  {}", registrar_uri);
    println!("[registration] Transport:  {}", transport.to_uppercase());

    let mut peer = StreamPeer::with_config(config).await?;

    println!("[registration] Registering...");
    let reg = peer
        .register_with(
            Registration::new(&registrar_uri, &auth_user, &password)
                .from_uri(aor_uri)
                .contact_uri(contact_uri),
        )
        .await?;

    sleep(Duration::from_secs(2)).await;
    match peer.is_registered(&reg).await {
        Ok(true) => println!("[registration] Registered."),
        Ok(false) => println!("[registration] Registration not yet confirmed."),
        Err(e) => println!("[registration] Registration check error: {}", e),
    }

    println!("[registration] Holding registration for {}s...", idle_secs);
    sleep(Duration::from_secs(idle_secs)).await;

    println!("[registration] Unregistering...");
    peer.unregister(&reg).await.ok();
    println!("[registration] Done.");
    std::process::exit(0);
}
