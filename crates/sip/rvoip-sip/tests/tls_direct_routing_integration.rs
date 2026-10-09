//! `Config::tls_direct_routing` end to end on loopback.
//!
//! A test CA issues one certificate to the SBC (built from the profile) and
//! one to a "cloud" peer standing in for a calling platform's SIP proxy.
//! Both sides require client certificates, as Direct Routing does. The test
//! proves the profile's pieces work together, not interop with any real
//! platform:
//!
//! 1. The SBC's OPTIONS keep-alive reaches the cloud over mutual TLS (so the
//!    SBC presents its client certificate) and reports the peer reachable.
//! 2. The cloud calls the SBC over mutual TLS; the call is answered with
//!    SDES-SRTP and torn down by an in-dialog BYE.
//! 3. A caller without a client certificate never reaches the SBC.

use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::Duration;

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose,
};
use tempfile::TempDir;
use tokio::time::timeout;

use rvoip_sip::{Config, Event, SipTlsMode, StreamPeer};

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

struct Identity {
    cert: PathBuf,
    key: PathBuf,
}

struct TestPki {
    _dir: TempDir,
    ca: PathBuf,
    sbc: Identity,
    cloud: Identity,
}

impl TestPki {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("PKI directory");
        let ca_key = KeyPair::generate().expect("CA key");
        let mut ca_params =
            CertificateParams::new(vec!["Direct Routing Test CA".into()]).expect("CA parameters");
        ca_params.distinguished_name = DistinguishedName::new();
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "Direct Routing Test CA");
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let ca_cert = ca_params.self_signed(&ca_key).expect("CA certificate");
        let issuer = Issuer::from_params(&ca_params, &ca_key);

        let ca = dir.path().join("ca.pem");
        write(&ca, &ca_cert.pem());
        let issue = |name: &str| {
            let key = KeyPair::generate().expect("leaf key");
            let mut params = CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()])
                .expect("leaf parameters");
            params.extended_key_usages = vec![
                ExtendedKeyUsagePurpose::ServerAuth,
                ExtendedKeyUsagePurpose::ClientAuth,
            ];
            let cert = params.signed_by(&key, &issuer).expect("leaf certificate");
            let identity = Identity {
                cert: dir.path().join(format!("{name}.pem")),
                key: dir.path().join(format!("{name}-key.pem")),
            };
            write(&identity.cert, &cert.pem());
            write(&identity.key, &key.serialize_pem());
            identity
        };
        let sbc = issue("sbc");
        let cloud = issue("cloud");
        Self {
            _dir: dir,
            ca,
            sbc,
            cloud,
        }
    }
}

fn write(path: &Path, contents: &str) {
    std::fs::write(path, contents).expect("write PKI material");
}

/// A TLS endpoint standing in for the platform's SIP proxy.
fn platform_config(pki: &TestPki, tls_bind: SocketAddr, present_certificate: bool) -> Config {
    let mut config = Config::local("cloud", 0)
        .tls_reachable_contact(tls_bind, &pki.cloud.cert, &pki.cloud.key)
        .require_tls_client_certificate(&pki.ca);
    config.contact_uri = Some(format!("sip:127.0.0.1:{};transport=tls", tls_bind.port()));
    config.tls_extra_ca_path = Some(pki.ca.clone());
    if present_certificate {
        config.tls_client_cert_path = Some(pki.cloud.cert.clone());
        config.tls_client_key_path = Some(pki.cloud.key.clone());
    }
    config.offer_srtp = true;
    config.srtp_required = true;
    config
}

async fn next_matching<F>(peer: &mut StreamPeer, wait: Duration, mut pred: F) -> Option<Event>
where
    F: FnMut(&Event) -> bool,
{
    timeout(wait, async {
        loop {
            match peer.next_event().await {
                Some(event) if pred(&event) => return Some(event),
                Some(_) => {}
                None => return None,
            }
        }
    })
    .await
    .ok()
    .flatten()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn direct_routing_profile_runs_mutual_tls_srtp_and_options_keepalive() {
    install_crypto_provider();
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();
    let pki = TestPki::new();
    let sbc_tls = reserve_tcp_addr();
    let cloud_tls = reserve_tcp_addr();
    let peer_uri = format!("sip:127.0.0.1:{};transport=tls", cloud_tls.port());

    let mut sbc_config = Config::tls_direct_routing(
        "sbc",
        "localhost",
        sbc_tls,
        "127.0.0.1".parse().unwrap(),
        &pki.sbc.cert,
        &pki.sbc.key,
        &pki.ca,
        peer_uri.clone(),
    );
    // The platform's server certificate comes from the test CA, not a
    // public root; ping fast so the test does not wait a minute.
    sbc_config.tls_extra_ca_path = Some(pki.ca.clone());
    sbc_config.options_keepalive_interval_secs = 1;
    sbc_config.media_port_start = 41600;
    sbc_config.media_port_end = 41700;
    assert_eq!(sbc_config.sip_tls_mode, SipTlsMode::ClientAndServer);
    assert_eq!(
        sbc_config.contact_uri.as_deref(),
        Some(format!("sip:localhost:{};transport=tls", sbc_tls.port()).as_str())
    );

    let mut cloud_config = platform_config(&pki, cloud_tls, true);
    cloud_config.media_port_start = 41700;
    cloud_config.media_port_end = 41800;
    let mut cloud = StreamPeer::with_config(cloud_config).await.expect("cloud");
    let mut sbc = StreamPeer::with_config(sbc_config).await.expect("sbc");

    // 1. OPTIONS keep-alive over mutual TLS.
    let reachability = next_matching(&mut sbc, Duration::from_secs(8), |event| {
        matches!(event, Event::PeerReachabilityChanged { .. })
    })
    .await
    .expect("keep-alive outcome");
    match reachability {
        Event::PeerReachabilityChanged {
            target,
            reachable,
            status_code,
        } => {
            assert_eq!(target, peer_uri);
            assert!(reachable, "cloud must answer the SBC's mTLS OPTIONS");
            assert_eq!(status_code, Some(200));
        }
        _ => unreachable!(),
    }

    // 2. Inbound call from the platform over mutual TLS with SDES-SRTP.
    let call_id = cloud
        .invite(format!(
            "sip:+15551234567@127.0.0.1:{};transport=tls",
            sbc_tls.port()
        ))
        .send()
        .await
        .expect("cloud invite");
    let incoming = timeout(Duration::from_secs(8), sbc.wait_for_incoming())
        .await
        .expect("SBC sees the call over mTLS")
        .expect("incoming call");
    let sbc_call = incoming.accept().await.expect("SBC answers");
    timeout(Duration::from_secs(8), cloud.wait_for_answered(&call_id))
        .await
        .expect("answered in time")
        .expect("answered");
    let secured = next_matching(&mut sbc, Duration::from_secs(5), |event| {
        matches!(event, Event::MediaSecurityNegotiated { .. })
    })
    .await;
    assert!(secured.is_some(), "SBC media must negotiate SRTP");

    // In-dialog BYE from the platform reaches the SBC.
    cloud
        .coordinator()
        .hangup(&call_id)
        .await
        .expect("cloud hangup");
    let ended = next_matching(
        &mut sbc,
        Duration::from_secs(5),
        |event| matches!(event, Event::CallEnded { call_id, .. } if *call_id == *sbc_call.id()),
    )
    .await;
    assert!(ended.is_some(), "SBC must see the platform's BYE");

    // 3. A caller without a client certificate never reaches the SBC.
    let anonymous_tls = reserve_tcp_addr();
    let mut anonymous_config = platform_config(&pki, anonymous_tls, false);
    anonymous_config.media_port_start = 41800;
    anonymous_config.media_port_end = 41900;
    let anonymous = StreamPeer::with_config(anonymous_config)
        .await
        .expect("anonymous peer");
    let _ = anonymous
        .invite(format!(
            "sip:+15551234567@127.0.0.1:{};transport=tls",
            sbc_tls.port()
        ))
        .send()
        .await;
    let leaked = next_matching(&mut sbc, Duration::from_secs(3), |event| {
        matches!(event, Event::IncomingCall { .. })
    })
    .await;
    assert!(
        leaked.is_none(),
        "an unauthenticated TLS peer must not reach the SBC"
    );

    let _ = anonymous.shutdown().await;
    let _ = cloud.shutdown().await;
    let _ = sbc.shutdown().await;
}
