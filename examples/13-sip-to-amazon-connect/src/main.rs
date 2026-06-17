//! Example: batteries-included SIP → Amazon Connect screen-pop server.
//!
//! `ConnectScreenPopServer` is configured from a single object and then stands
//! up a SIP UAS. Every inbound INVITE (e.g. a Vapi blind transfer) has its
//! custom headers translated into Amazon Connect contact attributes (the
//! screen-pop channel), a WebRTC contact is placed into Connect, and the audio
//! is bridged — all inside the connector.
//!
//! Runs **offline by default** with a mock control plane: it answers SIP and
//! prints the attributes that would be sent to `StartWebRTCContact` (the Connect
//! media leg then fails fast against a fake signaling URL — expected without
//! AWS). With `--features aws-live` (+ AWS creds + the env vars below) it calls
//! the real Amazon Connect API and joins the Chime meeting.
//!
//! ```bash
//! cargo run                                  # offline: SIP UAS on :5060, mock Connect
//! AMAZON_CONNECT_INSTANCE_ID=... AMAZON_CONNECT_FLOW_ID=... AWS_REGION=us-west-2 \
//!   cargo run --features aws-live            # live Amazon Connect
//! ```
//!
//! Then send it an INVITE with custom headers, e.g. with `sipp` or another
//! rvoip endpoint, including headers like `X-Vapi-Customer-Id: cust-42`.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use rvoip_amazon_connect::{
    AttributeMapping, ConnectConfig, ConnectScreenPopServer, ScreenPopServerConfig, SipConfig,
    UnmappedPolicy,
};

#[cfg(not(feature = "aws-live"))]
use async_trait::async_trait;
#[cfg(not(feature = "aws-live"))]
use rvoip_amazon_connect::control::{
    ConnectContactStarter, ConnectionData, MediaPlacement, StartContactRequest,
};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,rvoip_amazon_connect=debug".into()),
        )
        .init();

    // 1. Define how SIP custom headers become Connect contact attributes.
    //
    //    To reuse the SAME screen pop the browser widget drives, the attribute
    //    KEYS must match what the contact flow's "Check contact attributes"
    //    block reads. The hosted widget surfaces attributes as
    //    `$.Attributes.HostedWidget-<name>`, so we emit `HostedWidget-`-prefixed
    //    keys here. We use `Drop` for unmapped headers so ONLY the keys the flow
    //    expects are sent (no stray attributes). Adjust the `from` side to the
    //    actual custom header names Vapi sends, and the `to` side to the exact
    //    keys your flow checks.
    let mapping = AttributeMapping::default()
        .with_unmapped(UnmappedPolicy::Drop)
        .rename("X-Vapi-Customer-Id", "HostedWidget-customerId")
        .rename("X-Vapi-Call-Id", "HostedWidget-vapiCallId")
        .rename("X-Account-Tier", "HostedWidget-accountTier");

    // Offline preview so there's immediate output before any call arrives.
    preview(&mapping);

    // 2. One config object → the whole pipeline.
    let (instance_id, flow_id, region) = aws_ids();
    let connect = ConnectConfig::new(instance_id, flow_id)
        .with_region(region.unwrap_or_else(|| "us-west-2".into()))
        .with_attribute_mapping(mapping);

    let (sip, port) = sip_config();
    let config = ScreenPopServerConfig::new(sip, connect, starter().await);

    // 3. Build + serve. `serve()` runs until the SIP event stream ends.
    let server = match ConnectScreenPopServer::build(config).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to build server: {e}");
            return;
        }
    };
    println!("\nConnectScreenPopServer listening for SIP INVITEs on udp/{port}");
    println!("Send an INVITE with e.g. `X-Vapi-Customer-Id: cust-42` to drive a screen pop.");
    println!("(The widget JWT is NOT used here — the server authenticates to AWS via IAM.)\n");
    if let Err(e) = server.serve().await {
        eprintln!("server stopped: {e}");
    }
}

/// Build the SIP UAS config from the environment so the server is reachable by
/// an external caller (Vapi). `SIP_BIND_IP` defaults to `0.0.0.0` (all
/// interfaces); behind NAT set `SIP_ADVERTISED_ADDR=<public-ip:port>` so the
/// Via/Contact the server emits are routable. Returns the config + bound port.
fn sip_config() -> (SipConfig, u16) {
    let bind_ip: IpAddr = std::env::var("SIP_BIND_IP")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    let port: u16 = std::env::var("SIP_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5060);

    let mut sip = SipConfig::on("connect-bridge", bind_ip, port);
    if let Some(adv) = std::env::var("SIP_ADVERTISED_ADDR")
        .ok()
        .and_then(|s| s.parse::<SocketAddr>().ok())
    {
        sip.sip_advertised_addr = Some(adv);
    }
    (sip, port)
}

fn preview(mapping: &AttributeMapping) {
    let sample = vec![
        ("X-Vapi-Customer-Id".to_string(), "cust-4815162342".to_string()),
        ("X-Account-Tier".to_string(), "platinum".to_string()),
        ("Subject".to_string(), "ignored".to_string()),
    ];
    let mapped = mapping.translate(sample);
    println!("Sample header translation (HostedWidget- keys drive the screen pop):");
    for (k, v) in &mapped.attributes {
        println!("  {k} = {v}");
    }
    if !mapped.skipped.is_empty() {
        println!("  (skipped: {:?})", mapped.skipped);
    }
}

fn aws_ids() -> (String, String, Option<String>) {
    (
        std::env::var("AMAZON_CONNECT_INSTANCE_ID").unwrap_or_else(|_| "demo-instance".into()),
        std::env::var("AMAZON_CONNECT_FLOW_ID").unwrap_or_else(|_| "demo-flow".into()),
        std::env::var("AWS_REGION").ok(),
    )
}

// ---- Control-plane starter: mock offline, real AWS under `aws-live`. ----

#[cfg(not(feature = "aws-live"))]
async fn starter() -> Arc<dyn ConnectContactStarter> {
    Arc::new(MockStarter)
}

#[cfg(feature = "aws-live")]
async fn starter() -> Arc<dyn rvoip_amazon_connect::ConnectContactStarter> {
    Arc::new(rvoip_amazon_connect::AwsConnectStarter::from_env(std::env::var("AWS_REGION").ok()).await)
}

/// Offline mock: logs the attributes that would drive the screen pop, then
/// returns a `ConnectionData` whose signaling URL is unreachable (so the media
/// leg fails fast — expected without AWS).
#[cfg(not(feature = "aws-live"))]
struct MockStarter;

#[cfg(not(feature = "aws-live"))]
#[async_trait]
impl ConnectContactStarter for MockStarter {
    async fn start_webrtc_contact(
        &self,
        request: StartContactRequest,
    ) -> rvoip_amazon_connect::Result<ConnectionData> {
        println!("\n[mock] StartWebRTCContact attributes (screen-pop payload):");
        for (k, v) in &request.attributes {
            println!("    {k} = {v}");
        }
        Ok(ConnectionData {
            contact_id: "mock-contact".into(),
            participant_id: "mock-participant".into(),
            participant_token: "mock-token".into(),
            meeting_id: "mock-meeting".into(),
            media_region: "us-west-2".into(),
            attendee_id: "mock-attendee".into(),
            join_token: "mock-join".into(),
            media_placement: MediaPlacement {
                signaling_url: "wss://signal.invalid/control/mock".into(),
                audio_host_url: "audio.invalid".into(),
                ..Default::default()
            },
        })
    }
}
