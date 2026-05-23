//! Phase 4 demo — UCTP-over-raw-QUIC agent.
//!
//! Dials the `orchestrator_bridge` at `127.0.0.1:4433` (ALPN `uctp/1`),
//! runs the auth handshake, sends a session.invite, prints inbound
//! envelopes, and stays connected until Ctrl-C.
//!
//! Run (after `orchestrator_bridge` is up):
//! ```bash
//! cargo run -p rvoip-uctp --example uctp_agent_quic
//! ```

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use rvoip_quic::UctpQuicClient;
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads::{auth, session};
use rvoip_uctp::substrate::dev_client_config_trusting;
use rvoip_uctp::types::MessageType;
use rustls::pki_types::CertificateDer;

const CERT_DER_PATH: &str = "/tmp/uctp_demo_cert.der";
const SERVER_ADDR: &str = "127.0.0.1:4433";

fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()))
        .init();
    install_crypto_provider();

    println!("[uctp_agent_quic] reading cert from {CERT_DER_PATH}");
    let cert_bytes = std::fs::read(CERT_DER_PATH).map_err(|e| {
        format!(
            "couldn't read {CERT_DER_PATH}: {e} \
             (start orchestrator_bridge first)"
        )
    })?;
    let cert = CertificateDer::from(cert_bytes);
    let client_cfg = dev_client_config_trusting(&cert)?;

    // Standalone client endpoint.
    let socket = std::net::UdpSocket::bind("127.0.0.1:0")?;
    let client_ep = quinn::Endpoint::new(
        quinn::EndpointConfig::default(),
        None,
        socket,
        Arc::new(quinn::TokioRuntime),
    )?;

    println!("[uctp_agent_quic] dialing {SERVER_ADDR}");
    let server_addr = SERVER_ADDR.parse()?;
    let client =
        UctpQuicClient::connect(&client_ep, server_addr, "localhost", Arc::new(client_cfg)).await?;
    let mut inbound = client.take_inbound().expect("first take");
    println!("[uctp_agent_quic] connected; sending auth.hello");

    let hello = UctpEnvelope::new(
        MessageType::AuthHello,
        serde_json::to_value(auth::AuthHello {
            device: auth::Device {
                id: "dev_quic_agent".into(),
                kind: "desktop".into(),
                platform: "linux-x86_64".into(),
                sdk_version: "uctp_agent_quic/0.1".into(),
            },
            auth_methods: vec!["bearer".into()],
            capabilities: serde_json::Value::Object(Default::default()),
        })?,
    );
    client.send(hello).await?;

    let challenge = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await?
        .ok_or("server closed before challenge")?;
    println!(
        "[uctp_agent_quic] received {:?} (in_reply_to={:?})",
        challenge.msg_type, challenge.in_reply_to
    );

    // Reply to the challenge with a non-empty bearer token (stub accepts any).
    let response = UctpEnvelope::new(
        MessageType::AuthResponse,
        serde_json::to_value(auth::AuthResponse {
            method: "bearer".into(),
            credential: "demo-token".into(),
        })?,
    )
    .with_in_reply_to(challenge.id);
    client.send(response).await?;

    let auth_session = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await?
        .ok_or("server closed before auth.session")?;
    println!(
        "[uctp_agent_quic] received {:?}",
        auth_session.msg_type
    );

    // Initiate an outbound session.invite (cross-transport target — would
    // bridge to SIP via the orchestrator in v0.x).
    let sid = format!("sess_{}", uuid::Uuid::new_v4().simple());
    let invite = UctpEnvelope::new(
        MessageType::SessionInvite,
        serde_json::to_value(session::SessionInvite {
            from: "part_quic_agent".into(),
            to: vec!["sip:bob@127.0.0.1:5072".into()],
            medium: "voice".into(),
            intent: "synchronous-engagement".into(),
            capabilities_offer: serde_json::Value::Object(Default::default()),
        })?,
    )
    .with_sid(sid.clone())
    .with_cid(format!("conv_{}", uuid::Uuid::new_v4().simple()));
    client.send(invite).await?;
    println!("[uctp_agent_quic] sent session.invite sid={}", sid);

    println!("[uctp_agent_quic] streaming inbound envelopes (Ctrl-C to quit)");
    while let Some(env) = inbound.recv().await {
        println!("[uctp_agent_quic] {:?}", env.msg_type);
    }

    Ok(())
}
