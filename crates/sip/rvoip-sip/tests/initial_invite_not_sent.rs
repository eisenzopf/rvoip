//! An initial INVITE whose transport fails before any SIP byte is written
//! (here: the TLS connection is refused) is reported as "not sent", both
//! from `UnifiedCoordinator::invite(..).send()` and from the
//! `ConnectionAdapter` activation used by `rvoip-core`. The peer never saw
//! the request, so callers can retry without risking a duplicate call and
//! owe no CANCEL or BYE.

use std::net::SocketAddr;
use std::sync::{Arc, Once};
use std::time::Duration;

use rvoip_core::adapter::{ConnectionAdapter, OriginateRequest};
use rvoip_core::capability::CapabilityDescriptor;
use rvoip_core::connection::{Direction, Transport};
use rvoip_core::ids::{ParticipantId, SessionId};
use rvoip_core::RvoipError;
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::{SessionError, SipAdapter, SipTlsMode};
use tokio::time::timeout;

fn install_crypto_provider() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// A loopback TCP address that refuses connections: its listener has
/// already closed.
fn refusing_addr() -> SocketAddr {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("reserve port")
        .local_addr()
        .expect("refusing address")
}

fn tls_client_config(name: &str) -> Config {
    let mut config = Config::local(name, 0);
    config.sip_tls_mode = SipTlsMode::ClientOnly;
    config.contact_uri = Some(format!("sip:{name}@127.0.0.1;transport=tls"));
    config
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refused_tls_connection_reports_invite_not_sent() {
    install_crypto_provider();
    let refused = refusing_addr();
    let coordinator = UnifiedCoordinator::new(tls_client_config("not-sent-uac"))
        .await
        .expect("coordinator");

    let result = timeout(
        Duration::from_secs(5),
        coordinator
            .invite(None, format!("sip:bob@{refused};transport=tls"))
            .send(),
    )
    .await
    .expect("send deadline");
    match result {
        Err(SessionError::InviteNotSent(_)) => {}
        other => panic!("expected InviteNotSent, got {other:?}"),
    }
    assert!(
        coordinator.list_sessions().await.is_empty(),
        "a not-sent INVITE must release its session"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refused_tls_connection_reports_outbound_not_sent_to_core() {
    install_crypto_provider();
    let refused = refusing_addr();
    let coordinator = UnifiedCoordinator::new(tls_client_config("not-sent-adapter"))
        .await
        .expect("coordinator");
    let adapter = SipAdapter::new(Arc::clone(&coordinator))
        .await
        .expect("adapter");

    let prepared = <SipAdapter as ConnectionAdapter>::originate(
        adapter.as_ref(),
        OriginateRequest::new(
            SessionId::new(),
            ParticipantId::new(),
            format!("sip:bob@{refused};transport=tls"),
            Direction::Outbound,
            CapabilityDescriptor::default(),
        )
        .with_transport(Transport::Sip),
    )
    .await
    .expect("dormant route");
    let result = timeout(
        Duration::from_secs(5),
        <SipAdapter as ConnectionAdapter>::activate_outbound_with_receipt(
            adapter.as_ref(),
            prepared.connection.id.clone(),
        ),
    )
    .await
    .expect("activation deadline");
    let error = result.expect_err("activation must fail");
    assert!(
        matches!(error, RvoipError::OutboundNotSent),
        "expected OutboundNotSent, got {error:?}"
    );
    assert_eq!(error.diagnostic_class(), "outbound-not-sent");
}
