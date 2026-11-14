//! Regression: SIP adapter outbound activation must not cancel the first
//! INVITE while its transport is still being prepared.
//!
//! The first INVITE to a new destination includes DNS resolution, the TCP
//! connect and the TLS handshake. `SipAdapter` used to bound that whole send
//! with its 2 s retained-cleanup budget, so a reachable peer whose handshake
//! took longer than 2 s (a distant carrier, a busy SBC) never received the
//! INVITE and the call failed as `InviteFailed`.
//!
//! A loopback relay holds the caller's first TCP connection for longer than
//! that budget before connecting it to a real TLS UAS, so the TLS handshake,
//! and with it the INVITE, completes only after 2 s. Activation must still
//! succeed.

use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Once};
use std::time::{Duration, Instant};

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose,
};
use rvoip_core::adapter::{ConnectionAdapter, EndReason, OriginateRequest};
use rvoip_core::capability::CapabilityDescriptor;
use rvoip_core::connection::{Direction, Transport};
use rvoip_core::ids::{ParticipantId, SessionId};
use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::{SipAdapter, StreamPeer};
use tempfile::TempDir;
use tokio::time::timeout;

/// Longer than the adapter's 2 s retained-task budget, well inside the
/// 30 s outbound activation budget.
const HANDSHAKE_DELAY: Duration = Duration::from_millis(2_600);
const STEP_TIMEOUT: Duration = Duration::from_secs(15);

fn install_crypto_provider() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

fn reserve_tcp_addr() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve TCP port");
    listener.local_addr().expect("reserved TCP address")
}

struct TestPki {
    _dir: TempDir,
    ca: PathBuf,
    cert: PathBuf,
    key: PathBuf,
}

impl TestPki {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("PKI directory");
        let ca_key = KeyPair::generate().expect("CA key");
        let mut ca_params =
            CertificateParams::new(vec!["Slow TLS Test CA".into()]).expect("CA parameters");
        ca_params.distinguished_name = DistinguishedName::new();
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "Slow TLS Test CA");
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let ca_cert = ca_params.self_signed(&ca_key).expect("CA certificate");
        let issuer = Issuer::from_params(&ca_params, &ca_key);

        let key = KeyPair::generate().expect("leaf key");
        let mut params = CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()])
            .expect("leaf parameters");
        params.extended_key_usages = vec![
            ExtendedKeyUsagePurpose::ServerAuth,
            ExtendedKeyUsagePurpose::ClientAuth,
        ];
        let cert = params.signed_by(&key, &issuer).expect("leaf certificate");

        let pki = Self {
            ca: dir.path().join("ca.pem"),
            cert: dir.path().join("leaf.pem"),
            key: dir.path().join("leaf-key.pem"),
            _dir: dir,
        };
        write(&pki.ca, &ca_cert.pem());
        write(&pki.cert, &cert.pem());
        write(&pki.key, &key.serialize_pem());
        pki
    }
}

fn write(path: &Path, contents: &str) {
    std::fs::write(path, contents).expect("write PKI material");
}

/// Accept connections on `listener`; hold the first one for
/// `HANDSHAKE_DELAY` before connecting it to `upstream`, then relay bytes
/// both ways. Later connections are relayed at once.
fn spawn_delaying_relay(
    listener: tokio::net::TcpListener,
    upstream: SocketAddr,
) -> (
    tokio::task::JoinHandle<()>,
    tokio::sync::oneshot::Receiver<Instant>,
) {
    let (first_accept_tx, first_accept_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut first_accept_tx = Some(first_accept_tx);
        loop {
            let Ok((mut downstream, _)) = listener.accept().await else {
                return;
            };
            let delay = match first_accept_tx.take() {
                Some(tx) => {
                    let _ = tx.send(Instant::now());
                    HANDSHAKE_DELAY
                }
                None => Duration::ZERO,
            };
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                let Ok(mut upstream) = tokio::net::TcpStream::connect(upstream).await else {
                    return;
                };
                let _ = tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await;
            });
        }
    });
    (task, first_accept_rx)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn adapter_first_invite_survives_tls_handshake_slower_than_two_seconds() {
    install_crypto_provider();
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();
    let pki = TestPki::new();

    // UAS: a TLS listener whose certificate the caller trusts.
    let uas_tls = reserve_tcp_addr();
    let mut uas_config =
        Config::local("slow-tls-uas", 0).tls_reachable_contact(uas_tls, &pki.cert, &pki.key);
    uas_config.contact_uri = Some(format!("sip:127.0.0.1:{};transport=tls", uas_tls.port()));
    uas_config.tls_extra_ca_path = Some(pki.ca.clone());
    let mut uas = timeout(STEP_TIMEOUT, StreamPeer::with_config(uas_config))
        .await
        .expect("UAS start deadline")
        .expect("UAS");

    let relay_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("relay bind");
    let relay_addr = relay_listener.local_addr().expect("relay address");
    let (relay, first_accept) = spawn_delaying_relay(relay_listener, uas_tls);

    // UAC: a TLS endpoint behind the SIP adapter.
    let uac_tls = reserve_tcp_addr();
    let mut uac_config =
        Config::local("slow-tls-uac", 0).tls_reachable_contact(uac_tls, &pki.cert, &pki.key);
    uac_config.contact_uri = Some(format!("sip:127.0.0.1:{};transport=tls", uac_tls.port()));
    uac_config.tls_extra_ca_path = Some(pki.ca.clone());
    let coordinator = timeout(STEP_TIMEOUT, UnifiedCoordinator::new(uac_config))
        .await
        .expect("UAC start deadline")
        .expect("UAC coordinator");
    let adapter = SipAdapter::new(Arc::clone(&coordinator))
        .await
        .expect("adapter");

    let prepared = <SipAdapter as ConnectionAdapter>::originate(
        adapter.as_ref(),
        OriginateRequest::new(
            SessionId::new(),
            ParticipantId::new(),
            format!("sip:callee@127.0.0.1:{};transport=tls", relay_addr.port()),
            Direction::Outbound,
            CapabilityDescriptor::default(),
        )
        .with_transport(Transport::Sip),
    )
    .await
    .expect("dormant route");
    let connection_id = prepared.connection.id.clone();

    let activation_adapter = Arc::clone(&adapter);
    let activation_connection = connection_id.clone();
    let activation = tokio::spawn(async move {
        <SipAdapter as ConnectionAdapter>::activate_outbound_with_receipt(
            activation_adapter.as_ref(),
            activation_connection,
        )
        .await
    });

    let first_accept = timeout(STEP_TIMEOUT, first_accept)
        .await
        .expect("caller connects to the relay")
        .expect("relay accept time");
    let incoming = timeout(STEP_TIMEOUT, uas.wait_for_incoming())
        .await
        .expect("delayed INVITE reaches the UAS")
        .expect("incoming call");
    assert!(
        first_accept.elapsed() >= HANDSHAKE_DELAY,
        "the INVITE must have waited for the delayed TLS handshake"
    );
    let _uas_call = timeout(STEP_TIMEOUT, incoming.accept())
        .await
        .expect("UAS accept deadline")
        .expect("UAS answers");

    timeout(STEP_TIMEOUT, activation)
        .await
        .expect("activation deadline")
        .expect("activation task")
        .expect("activation succeeds although the TLS handshake took longer than 2 s");

    timeout(
        STEP_TIMEOUT,
        <SipAdapter as ConnectionAdapter>::end(adapter.as_ref(), connection_id, EndReason::Normal),
    )
    .await
    .expect("teardown deadline")
    .expect("teardown");
    adapter.drain().await.expect("adapter drain");
    coordinator
        .shutdown_gracefully(Some(Duration::from_secs(1)))
        .await
        .expect("UAC shutdown");
    let _ = timeout(STEP_TIMEOUT, uas.shutdown()).await;
    relay.abort();
}
