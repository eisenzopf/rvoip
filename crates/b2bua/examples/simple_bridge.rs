//! Minimal B2BUA bridge server.
//!
//! Environment variables:
//!
//! - `B2BUA_SIP_PORT` (default `5060`)
//! - `B2BUA_NAME` (default `b2bua`)
//! - `B2BUA_TARGET` (required, for example `sip:agent@127.0.0.1:5070`)
//! - `B2BUA_MEDIA_START` / `B2BUA_MEDIA_END` (default `16000` / `17000`)
//! - `B2BUA_CALL_DURATION_SECS` (optional demo auto-hangup)

use std::sync::Arc;
use std::time::Duration;

use rvoip_b2bua::{B2buaEvent, B2buaService, SessionConfig, StaticRouter};
use tokio::time::sleep;

fn env_u16(name: &str, default: u16) -> u16 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()))
        .init();

    let target = std::env::var("B2BUA_TARGET").map_err(|_| {
        "B2BUA_TARGET is required, for example sip:agent@127.0.0.1:5070".to_string()
    })?;
    let sip_port = env_u16("B2BUA_SIP_PORT", 5060);
    let name = std::env::var("B2BUA_NAME").unwrap_or_else(|_| "b2bua".to_string());

    let mut session_config = SessionConfig::local(&name, sip_port);
    session_config.media_port_start = env_u16("B2BUA_MEDIA_START", 16000);
    session_config.media_port_end = env_u16("B2BUA_MEDIA_END", 17000);

    let service = B2buaService::new(session_config).await?;
    let router = Arc::new(StaticRouter::dial(target));
    let mut events = service.events();
    let coordinator = service.coordinator();
    let max_call_duration = env_u64("B2BUA_CALL_DURATION_SECS");

    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            match event {
                B2buaEvent::IncomingReceived {
                    call_id, from, to, ..
                } => {
                    println!("[B2BUA] incoming {call_id}: {from} -> {to}");
                }
                B2buaEvent::BridgeEstablished {
                    call_id,
                    bridge_id,
                    inbound_session_id,
                    outbound_session_id,
                    ..
                } => {
                    println!("[B2BUA] bridged {call_id} via {bridge_id}");
                    if let Some(seconds) = max_call_duration {
                        let coordinator = coordinator.clone();
                        tokio::spawn(async move {
                            sleep(Duration::from_secs(seconds)).await;
                            let _ = coordinator.hangup(&inbound_session_id).await;
                            let _ = coordinator.hangup(&outbound_session_id).await;
                        });
                    }
                }
                B2buaEvent::CallEnded { call_id, reason } => {
                    println!("[B2BUA] ended {call_id}: {reason}");
                }
                B2buaEvent::CallFailed { call_id, reason } => {
                    println!("[B2BUA] failed {call_id}: {reason}");
                }
                other => {
                    println!("[B2BUA] {other:?}");
                }
            }
        }
    });

    println!("[B2BUA] listening on sip:{name}@127.0.0.1:{sip_port}");
    service.serve(router).await?;
    Ok(())
}
