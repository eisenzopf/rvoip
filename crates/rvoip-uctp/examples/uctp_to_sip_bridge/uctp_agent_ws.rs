//! Phase 4 demo — UCTP-over-WebSocket agent (older-browser fallback path).
//!
//! Dials the `orchestrator_bridge` at `ws://127.0.0.1:7777`, runs the
//! auth handshake, sends a session.invite, streams inbound envelopes
//! until Ctrl-C.
//!
//! Run (after `orchestrator_bridge` is up):
//! ```bash
//! cargo run -p rvoip-uctp --example uctp_agent_ws
//! ```

use std::time::Duration;

use chrono::Utc;
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::payloads::{auth, session};
use rvoip_uctp::types::MessageType;
use rvoip_websocket::UctpWsClient;
use url::Url;

const SERVER_URL: &str = "ws://127.0.0.1:7777";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()))
        .init();

    let url = Url::parse(SERVER_URL)?;
    println!("[uctp_agent_ws] dialing {SERVER_URL}");
    let client = UctpWsClient::connect(&url).await?;
    let mut inbound = client.take_inbound().expect("first take");
    println!("[uctp_agent_ws] connected; sending auth.hello");

    let hello = UctpEnvelope::new(
        MessageType::AuthHello,
        serde_json::to_value(auth::AuthHello {
            device: auth::Device {
                id: "dev_ws_agent".into(),
                kind: "web".into(),
                platform: "older-browser-fallback".into(),
                sdk_version: "uctp_agent_ws/0.1".into(),
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
        "[uctp_agent_ws] received {:?} (in_reply_to={:?})",
        challenge.msg_type, challenge.in_reply_to
    );

    let response = UctpEnvelope::new(
        MessageType::AuthResponse,
        serde_json::to_value(auth::AuthResponse {
            method: "bearer".into(),
            credential: "demo-token-ws".into(),
            actor_token: None,        })?,
    )
    .with_in_reply_to(challenge.id);
    client.send(response).await?;

    let auth_session = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await?
        .ok_or("server closed before auth.session")?;
    println!("[uctp_agent_ws] received {:?}", auth_session.msg_type);

    let sid = format!("sess_{}", uuid::Uuid::new_v4().simple());
    let invite = UctpEnvelope::new(
        MessageType::SessionInvite,
        serde_json::to_value(session::SessionInvite {
            from: "part_ws_agent".into(),
            to: vec!["sip:bob@127.0.0.1:5072".into()],
            medium: "voice".into(),
            intent: "synchronous-engagement".into(),
            capabilities_offer: serde_json::Value::Object(Default::default()),
        })?,
    )
    .with_sid(sid.clone())
    .with_cid(format!("conv_{}", uuid::Uuid::new_v4().simple()));
    client.send(invite).await?;
    println!("[uctp_agent_ws] sent session.invite sid={}", sid);

    println!("[uctp_agent_ws] streaming inbound envelopes (Ctrl-C to quit)");
    while let Some(env) = inbound.recv().await {
        println!("[uctp_agent_ws] {:?}", env.msg_type);
    }

    Ok(())
}
