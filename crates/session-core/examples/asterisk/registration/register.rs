//! Register session-core with a remote Asterisk SIP server using settings
//! from a .env file.
//!
//! Usage:
//!   cd crates/session-core/examples/asterisk
//!   cp .env.example .env       # edit values for your PBX
//!   ./registration/run.sh

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use std::path::PathBuf;

use rvoip_session_core::{types::Credentials, Config, Registration, SipContactMode, StreamPeer};
use tokio::time::sleep;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

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

fn required_path(key: &str) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let value =
        std::env::var(key).map_err(|_| format!("{} must be set for SIP TLS listener mode", key))?;
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{} must not be empty", key).into());
    }
    Ok(PathBuf::from(value))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TlsContactMode {
    ReachableContact,
    RegisteredFlowRfc5626,
    RegisteredFlowSymmetric,
}

impl TlsContactMode {
    fn from_env() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        if env_bool("ASTERISK_TLS_FLOW_REUSE", false)? {
            return Ok(Self::RegisteredFlowSymmetric);
        }

        match env_or("ASTERISK_TLS_CONTACT_MODE", "reachable-contact")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "reachable-contact" | "reachable" | "listener" | "uas" => Ok(Self::ReachableContact),
            "registered-flow" | "registered-flow-rfc5626" | "rfc5626" | "outbound" => {
                Ok(Self::RegisteredFlowRfc5626)
            }
            "registered-flow-symmetric" | "symmetric" | "symmetric-transport"
            | "flow-reuse" | "client-only" => Ok(Self::RegisteredFlowSymmetric),
            other => Err(format!(
                "ASTERISK_TLS_CONTACT_MODE must be reachable-contact, registered-flow-rfc5626, or registered-flow-symmetric, got '{}'",
                other
            )
            .into()),
        }
    }

    fn uses_listener(self) -> bool {
        self == Self::ReachableContact
    }

    fn label(self) -> &'static str {
        match self {
            Self::ReachableContact => "reachable-contact",
            Self::RegisteredFlowRfc5626 => "registered-flow-rfc5626",
            Self::RegisteredFlowSymmetric => "registered-flow-symmetric",
        }
    }
}

fn deterministic_sip_instance(username: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in username.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!(
        "urn:uuid:00000000-0000-4000-8000-{:012x}",
        hash & 0xffff_ffff_ffff
    )
}

fn sip_instance_urn(username: &str) -> String {
    std::env::var(format!("ENDPOINT_{}_SIP_INSTANCE", username))
        .or_else(|_| std::env::var("SIP_INSTANCE"))
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| deterministic_sip_instance(username))
}

fn tls_contact_mode() -> Result<TlsContactMode, Box<dyn std::error::Error + Send + Sync>> {
    TlsContactMode::from_env()
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
    let endpoint_tls_port_key = format!("ENDPOINT_{}_TLS_LOCAL_PORT", username);
    let tls_local_port: u16 = std::env::var(&endpoint_tls_port_key)
        .unwrap_or_else(|_| local_port.saturating_add(1).to_string())
        .parse()?;
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
    let contact_mode = if is_tls {
        tls_contact_mode()?
    } else {
        TlsContactMode::ReachableContact
    };
    let contact_port = if is_tls && contact_mode.uses_listener() {
        tls_local_port
    } else {
        local_port
    };
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
    config.sip_contact_mode = if is_tls {
        match contact_mode {
            TlsContactMode::ReachableContact => SipContactMode::ReachableContact,
            TlsContactMode::RegisteredFlowRfc5626 => SipContactMode::RegisteredFlowRfc5626,
            TlsContactMode::RegisteredFlowSymmetric => SipContactMode::RegisteredFlowSymmetric,
        }
    } else {
        SipContactMode::ReachableContact
    };
    config.credentials = Some(Credentials::new(&auth_user, &password));
    if is_tls {
        match contact_mode {
            TlsContactMode::ReachableContact => {
                config = config.tls_reachable_contact(
                    SocketAddr::new(local_ip, tls_local_port),
                    required_path("TLS_CERT_PATH")?,
                    required_path("TLS_KEY_PATH")?,
                );
            }
            TlsContactMode::RegisteredFlowRfc5626 => {
                config = config.tls_registered_flow_rfc5626(sip_instance_urn(&username));
            }
            TlsContactMode::RegisteredFlowSymmetric => {
                config = config.tls_registered_flow_symmetric(sip_instance_urn(&username));
            }
        }
        config.tls_extra_ca_path = optional_path("TLS_CA_PATH");
        config.tls_client_cert_path = optional_path("TLS_CLIENT_CERT_PATH");
        config.tls_client_key_path = optional_path("TLS_CLIENT_KEY_PATH");
        #[cfg(feature = "dev-insecure-tls")]
        {
            config.tls_insecure_skip_verify = env_bool("TLS_INSECURE", false)?;
        }
    }

    println!("[registration] Local bind: {}:{}", local_ip, local_port);
    if is_tls {
        println!(
            "[registration] TLS mode:   {}{}",
            contact_mode.label(),
            if contact_mode.uses_listener() {
                format!(" (listener {}:{})", local_ip, tls_local_port)
            } else {
                String::new()
            }
        );
    }
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
