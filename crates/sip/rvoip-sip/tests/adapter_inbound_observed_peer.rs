//! End-to-end: a real INVITE over UDP reaches the SIP adapter's inbound
//! context with the transport-observed peer address and, for a peer admitted
//! by trusted-trunk policy, the observed caller number.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket as StdUdpSocket};
use std::time::Duration;

use ipnet::IpNet;
use rvoip_core::adapter::{ConnectionAdapter, InboundConnectionContext, OrchestratorAdapterEvent};
use rvoip_core::identity::{AuthenticatedPrincipal, AuthenticationMethod, IdentityAssurance};
use rvoip_core::{ObservedCallerNumber, ObservedCallerSource};
use rvoip_sip::{Config, MediaMode, SipAdapter, SipListenerAuthPolicy};
use serial_test::serial;
use tokio::net::UdpSocket;

const TENANT: &str = "observed-peer-test";
/// The issuer the SIP stack stamps on principals admitted by a trusted CIDR.
const TRUSTED_TRUNK_ISSUER: &str = "rvoip-app-trusted-trunk";

fn reserve_udp_addr() -> SocketAddr {
    let socket = StdUdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("reserve UDP port");
    socket.local_addr().expect("reserved UDP address")
}

fn signaling_config(bind: SocketAddr) -> Config {
    let mut config = Config::on("bridge", bind.ip(), bind.port());
    config.bind_addr = bind;
    config.media_mode = MediaMode::SignalingOnly { sdp_rtp_port: 9 };
    config
}

fn principal(issuer: &str) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        subject: "carrier-trunk".to_string(),
        tenant: Some(TENANT.to_string()),
        scopes: vec!["call:attach".to_string()],
        issuer: Some(issuer.to_string()),
        expires_at: None,
        method: AuthenticationMethod::ApiKey,
        assurance: IdentityAssurance::Anonymous,
    }
}

fn invite_wire(
    destination: SocketAddr,
    call_id: &str,
    from_user: &str,
    pai: Option<&str>,
) -> Vec<u8> {
    let mut wire = format!(
        "INVITE sip:+15550100@{destination};transport=udp SIP/2.0\r\n\
         Via: SIP/2.0/UDP 127.0.0.1:5099;branch=z9hG4bK.{call_id};rport\r\n\
         From: <sip:{from_user}@carrier.example>;tag=observed\r\n\
         To: <sip:+15550100@{destination}>\r\n\
         Call-ID: {call_id}\r\n\
         CSeq: 1 INVITE\r\n\
         Max-Forwards: 70\r\n\
         Contact: <sip:caller@127.0.0.1:5099>\r\n"
    );
    if let Some(pai) = pai {
        wire.push_str(&format!("P-Asserted-Identity: {pai}\r\n"));
    }
    wire.push_str("Content-Length: 0\r\n\r\n");
    wire.into_bytes()
}

/// Send one INVITE from loopback and return the adapter's inbound context.
async fn context_for(issuer: &str, from_user: &str, pai: Option<&str>) -> InboundConnectionContext {
    let bind = reserve_udp_addr();
    let policy = SipListenerAuthPolicy::enabled_for_tenant(TENANT)
        .expect("tenant policy")
        .with_trusted_cidr(
            IpNet::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 32).expect("loopback CIDR"),
            principal(issuer),
        );
    let adapter = SipAdapter::from_config_with_listener_auth(signaling_config(bind), policy)
        .await
        .expect("SIP adapter");
    let mut events = adapter
        .try_subscribe_atomic_events()
        .expect("adapter event stream");
    let client = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("UDP client");
    client
        .send_to(
            &invite_wire(bind, &format!("observed-{from_user}"), from_user, pai),
            bind,
        )
        .await
        .expect("send INVITE");
    let connection_id = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await.expect("adapter event stream closed") {
                OrchestratorAdapterEvent::AuthenticatedInboundConnection { connection, .. } => {
                    return connection.id;
                }
                _ => continue,
            }
        }
    })
    .await
    .expect("authenticated inbound connection");
    let context = ConnectionAdapter::take_inbound_context(adapter.as_ref(), &connection_id)
        .expect("inbound context");
    adapter
        .coordinator()
        .shutdown_gracefully(Some(Duration::from_secs(1)))
        .await
        .expect("shutdown");
    context
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn trusted_trunk_context_carries_peer_address_and_asserted_caller_number() {
    let context = context_for(
        TRUSTED_TRUNK_ISSUER,
        "+14085550000",
        Some("<sip:+14085551212@carrier.example>"),
    )
    .await;
    assert_eq!(
        context.observed_peer_address(),
        Some(IpAddr::V4(Ipv4Addr::LOCALHOST))
    );
    let caller = context.observed_caller_number().expect("observed caller");
    assert_eq!(caller.number(), "+14085551212");
    assert_eq!(
        caller.source(),
        ObservedCallerSource::TrustedAssertedIdentity
    );
    // Neither value leaks through Debug.
    let debug = format!("{context:?}");
    assert!(!debug.contains("14085551212") && !debug.contains("127.0.0.1"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn trusted_trunk_without_usable_assertion_falls_back_to_e164_from() {
    let context = context_for(
        TRUSTED_TRUNK_ISSUER,
        "+14085550000",
        Some("<sip:anonymous@anonymous.invalid>"),
    )
    .await;
    assert_eq!(
        context.observed_caller_number(),
        Some(
            &ObservedCallerNumber::new("+14085550000", ObservedCallerSource::AdmittedFrom).unwrap()
        )
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn other_principals_get_the_peer_address_but_no_caller_number() {
    let context = context_for(
        "some-other-issuer",
        "+14085550000",
        Some("<sip:+14085551212@carrier.example>"),
    )
    .await;
    assert_eq!(
        context.observed_peer_address(),
        Some(IpAddr::V4(Ipv4Addr::LOCALHOST))
    );
    assert!(context.observed_caller_number().is_none());
}
