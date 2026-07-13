//! End-to-end tenant binding at the SIP listener -> SipAdapter boundary.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, UdpSocket as StdUdpSocket};
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::Duration;

use ipnet::IpNet;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose,
};
use rvoip_core::adapter::OrchestratorAdapterEvent;
use rvoip_core::identity::{
    AuthenticatedPrincipal, AuthenticationMethod, CredentialKind, IdentityAssurance,
};
use rvoip_sip::{
    Config, DigestAuth, DigestAuthenticator, MediaMode, SipAdapter, SipAuthService,
    SipListenerAuthPolicy,
};
use rvoip_sip_core::types::headers::HeaderAccess;
use rvoip_sip_core::{parse_message, HeaderName, Message, StatusCode};
use rvoip_sip_transport::transport::tls::{TlsClientConfig, TlsTransport};
use rvoip_sip_transport::Transport;
use serial_test::serial;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::net::UdpSocket;

const TENANT: &str = "bridgefu-test";

fn install_crypto_provider() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

fn reserve_udp_addr() -> SocketAddr {
    let socket = StdUdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("reserve UDP port");
    socket.local_addr().expect("reserved UDP address")
}

fn reserve_tcp_addr() -> SocketAddr {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("reserve TCP port");
    listener.local_addr().expect("reserved TCP address")
}

fn signaling_config(bind: SocketAddr) -> Config {
    let mut config = Config::on("bridge", bind.ip(), bind.port());
    config.bind_addr = bind;
    config.media_mode = MediaMode::SignalingOnly { sdp_rtp_port: 9 };
    config
}

fn principal(subject: &str, tenant: Option<&str>) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        subject: subject.to_string(),
        tenant: tenant.map(str::to_string),
        scopes: vec!["call:attach".to_string()],
        issuer: Some("adapter-listener-test".to_string()),
        expires_at: None,
        method: AuthenticationMethod::ApiKey,
        assurance: IdentityAssurance::Identified {
            credential_kind: CredentialKind::SipDigest,
        },
    }
}

fn invite_wire(
    destination: SocketAddr,
    transport: &str,
    call_id: &str,
    cseq: u32,
    branch: &str,
    authorization: Option<&str>,
) -> Vec<u8> {
    let scheme = if transport.eq_ignore_ascii_case("TLS") {
        "sips"
    } else {
        "sip"
    };
    let host = if transport.eq_ignore_ascii_case("TLS") {
        "localhost".to_string()
    } else {
        destination.ip().to_string()
    };
    let mut wire = format!(
        "INVITE {scheme}:bridge@{host}:{};transport={} SIP/2.0\r\n\
         Via: SIP/2.0/{transport} 127.0.0.1:5099;branch={branch}\r\n\
         From: <sip:caller@example.test>;tag=tenant-binding\r\n\
         To: <{scheme}:bridge@{host}:{}>\r\n\
         Call-ID: {call_id}\r\n\
         CSeq: {cseq} INVITE\r\n\
         Max-Forwards: 70\r\n\
         Contact: <sip:caller@127.0.0.1:5099>\r\n",
        destination.port(),
        transport.to_ascii_lowercase(),
        destination.port(),
    );
    if let Some(authorization) = authorization {
        wire.push_str("Authorization: ");
        wire.push_str(authorization);
        wire.push_str("\r\n");
    }
    wire.push_str("Content-Length: 0\r\n\r\n");
    wire.into_bytes()
}

async fn next_authenticated_principal(
    events: &mut tokio::sync::mpsc::Receiver<OrchestratorAdapterEvent>,
) -> AuthenticatedPrincipal {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await.expect("adapter event stream closed") {
                OrchestratorAdapterEvent::AuthenticatedInboundConnection { principal, .. } => {
                    return principal;
                }
                _ => continue,
            }
        }
    })
    .await
    .expect("timed out waiting for authenticated adapter handoff")
}

async fn assert_no_authenticated_event(
    events: &mut tokio::sync::mpsc::Receiver<OrchestratorAdapterEvent>,
) {
    let received = tokio::time::timeout(Duration::from_millis(400), async {
        loop {
            match events.recv().await {
                Some(OrchestratorAdapterEvent::AuthenticatedInboundConnection { .. }) => {
                    return true;
                }
                Some(_) => continue,
                None => return false,
            }
        }
    })
    .await;
    assert!(!matches!(received, Ok(true)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn trusted_cidr_reaches_sip_adapter_with_exact_policy_tenant() {
    let bind = reserve_udp_addr();
    let expected = principal("trusted-edge", Some(TENANT));
    let policy = SipListenerAuthPolicy::enabled_for_tenant(TENANT)
        .expect("tenant policy")
        .with_trusted_cidr(
            IpNet::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 32).expect("loopback CIDR"),
            expected.clone(),
        );
    let adapter = SipAdapter::from_config_with_listener_auth(signaling_config(bind), policy)
        .await
        .expect("tenant-bound SIP adapter");
    let mut events = adapter
        .try_subscribe_atomic_events()
        .expect("adapter event stream");
    let client = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("UDP client");

    client
        .send_to(
            &invite_wire(
                bind,
                "UDP",
                "trusted-cidr@adapter.test",
                1,
                "z9hG4bK.cidrt",
                None,
            ),
            bind,
        )
        .await
        .expect("send trusted INVITE");
    let admitted = next_authenticated_principal(&mut events).await;
    assert_eq!(admitted.ownership_key(), expected.ownership_key());
    assert_eq!(admitted.tenant.as_deref(), Some(TENANT));

    adapter
        .coordinator()
        .shutdown_gracefully(Some(Duration::from_secs(1)))
        .await
        .expect("shutdown");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn digest_reaches_sip_adapter_stamped_with_policy_tenant() {
    let bind = reserve_udp_addr();
    let policy = SipListenerAuthPolicy::authenticated_for_tenant(
        TENANT,
        SipAuthService::digest("bridgefu-listener")
            .with_digest_user("alice", "correct horse battery staple"),
    )
    .expect("tenant policy");
    let adapter = SipAdapter::from_config_with_listener_auth(signaling_config(bind), policy)
        .await
        .expect("tenant-bound SIP adapter");
    let mut events = adapter
        .try_subscribe_atomic_events()
        .expect("adapter event stream");
    let client = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("UDP client");

    client
        .send_to(
            &invite_wire(
                bind,
                "UDP",
                "digest@adapter.test",
                1,
                "z9hG4bK.digest1",
                None,
            ),
            bind,
        )
        .await
        .expect("send unauthenticated INVITE");

    let mut buffer = vec![0_u8; 16 * 1024];
    let challenge = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let (len, _) = client
                .recv_from(&mut buffer)
                .await
                .expect("receive challenge");
            let Message::Response(response) =
                parse_message(&buffer[..len]).expect("parse response")
            else {
                continue;
            };
            if response.status_code() == StatusCode::Unauthorized.as_u16() {
                let value = response
                    .raw_header_value(&HeaderName::WwwAuthenticate)
                    .expect("WWW-Authenticate");
                return DigestAuthenticator::parse_challenge(&value).expect("Digest challenge");
            }
        }
    })
    .await
    .expect("timed out waiting for Digest challenge");

    let uri = format!("sip:bridge@{}:{};transport=udp", bind.ip(), bind.port());
    let computed = DigestAuth::compute_response_with_state(
        "alice",
        "correct horse battery staple",
        &challenge,
        "INVITE",
        &uri,
        1,
        None,
    )
    .expect("Digest response");
    let authorization =
        DigestAuth::format_authorization_with_state("alice", &challenge, &uri, &computed);
    client
        .send_to(
            &invite_wire(
                bind,
                "UDP",
                "digest@adapter.test",
                2,
                "z9hG4bK.digest2",
                Some(&authorization),
            ),
            bind,
        )
        .await
        .expect("send authenticated INVITE");

    let admitted = next_authenticated_principal(&mut events).await;
    assert_eq!(admitted.subject, "alice");
    assert_eq!(admitted.tenant.as_deref(), Some(TENANT));
    assert_eq!(admitted.method, AuthenticationMethod::SipDigest);

    adapter
        .coordinator()
        .shutdown_gracefully(Some(Duration::from_secs(1)))
        .await
        .expect("shutdown");
}

struct TestPki {
    _dir: TempDir,
    ca_cert: PathBuf,
    server_cert: PathBuf,
    server_key: PathBuf,
    client_cert: PathBuf,
    client_key: PathBuf,
    untrusted_cert: PathBuf,
    untrusted_key: PathBuf,
    client_fingerprint: String,
}

impl TestPki {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("temporary PKI directory");
        let ca_key = KeyPair::generate().expect("CA key");
        let mut ca_params =
            CertificateParams::new(vec!["Bridgefu Test CA".into()]).expect("CA parameters");
        ca_params.distinguished_name = DistinguishedName::new();
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "Bridgefu Test CA");
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let ca_cert = ca_params.self_signed(&ca_key).expect("self-signed CA");
        let issuer = Issuer::from_params(&ca_params, &ca_key);

        let server_key = KeyPair::generate().expect("server key");
        let mut server_params =
            CertificateParams::new(vec!["localhost".into()]).expect("server parameters");
        server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let server_cert = server_params
            .signed_by(&server_key, &issuer)
            .expect("server certificate");

        let client_key = KeyPair::generate().expect("client key");
        let mut client_params =
            CertificateParams::new(vec!["trusted-client.test".into()]).expect("client parameters");
        client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let client_cert = client_params
            .signed_by(&client_key, &issuer)
            .expect("client certificate");

        let untrusted_key = KeyPair::generate().expect("untrusted key");
        let mut untrusted_params = CertificateParams::new(vec!["untrusted-client.test".into()])
            .expect("untrusted parameters");
        untrusted_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let untrusted_cert = untrusted_params
            .self_signed(&untrusted_key)
            .expect("untrusted certificate");

        let ca_cert_path = dir.path().join("ca.pem");
        let server_cert_path = dir.path().join("server.pem");
        let server_key_path = dir.path().join("server-key.pem");
        let client_cert_path = dir.path().join("client.pem");
        let client_key_path = dir.path().join("client-key.pem");
        let untrusted_cert_path = dir.path().join("untrusted.pem");
        let untrusted_key_path = dir.path().join("untrusted-key.pem");
        write(&ca_cert_path, &ca_cert.pem());
        write(&server_cert_path, &server_cert.pem());
        write(&server_key_path, &server_key.serialize_pem());
        write(&client_cert_path, &client_cert.pem());
        write(&client_key_path, &client_key.serialize_pem());
        write(&untrusted_cert_path, &untrusted_cert.pem());
        write(&untrusted_key_path, &untrusted_key.serialize_pem());

        Self {
            _dir: dir,
            ca_cert: ca_cert_path,
            server_cert: server_cert_path,
            server_key: server_key_path,
            client_cert: client_cert_path,
            client_key: client_key_path,
            untrusted_cert: untrusted_cert_path,
            untrusted_key: untrusted_key_path,
            client_fingerprint: Sha256::digest(client_cert.der().as_ref())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        }
    }

    fn client_config(&self, identity: Option<(&Path, &Path)>) -> TlsClientConfig {
        TlsClientConfig {
            extra_ca_path: Some(self.ca_cert.clone()),
            client_cert_path: identity.map(|(cert, _)| cert.to_path_buf()),
            client_key_path: identity.map(|(_, key)| key.to_path_buf()),
            ..Default::default()
        }
    }
}

fn write(path: &Path, contents: &str) {
    std::fs::write(path, contents).expect("write test PKI material");
}

fn tls_invite(destination: SocketAddr, call_id: &str) -> Message {
    parse_message(&invite_wire(
        destination,
        "TLS",
        call_id,
        1,
        &format!("z9hG4bK.{call_id}"),
        None,
    ))
    .expect("parse TLS INVITE")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn verified_mtls_certificate_reaches_adapter_and_unverified_peers_do_not() {
    install_crypto_provider();
    let pki = TestPki::new();
    let sip_bind = reserve_udp_addr();
    let tls_bind = reserve_tcp_addr();
    let mut config = signaling_config(sip_bind).tls_reachable_contact(
        tls_bind,
        &pki.server_cert,
        &pki.server_key,
    );
    config = config.require_tls_client_certificate(&pki.ca_cert);
    let expected = principal("mtls-edge", Some(TENANT));
    let policy = SipListenerAuthPolicy::enabled_for_tenant(TENANT)
        .expect("tenant policy")
        .with_verified_mtls_peer(pki.client_fingerprint.clone(), expected.clone());
    let adapter = SipAdapter::from_config_with_listener_auth(config, policy)
        .await
        .expect("mTLS SIP adapter");
    let mut events = adapter
        .try_subscribe_atomic_events()
        .expect("adapter event stream");

    let (anonymous, _) = TlsTransport::client_only(
        "127.0.0.1:0".parse().unwrap(),
        None,
        pki.client_config(None),
    )
    .await
    .expect("anonymous TLS client");
    // A TLS 1.3 client may finish its first write before observing the
    // server's certificate-required alert. The security boundary is that no
    // request from that connection reaches the authenticated adapter handoff.
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        anonymous.send_message(tls_invite(tls_bind, "anonymous"), tls_bind),
    )
    .await
    .expect("anonymous handshake deadline");
    assert_no_authenticated_event(&mut events).await;

    let (untrusted, _) = TlsTransport::client_only(
        "127.0.0.1:0".parse().unwrap(),
        None,
        pki.client_config(Some((&pki.untrusted_cert, &pki.untrusted_key))),
    )
    .await
    .expect("untrusted TLS client configuration");
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        untrusted.send_message(tls_invite(tls_bind, "untrusted"), tls_bind),
    )
    .await
    .expect("untrusted handshake deadline");
    assert_no_authenticated_event(&mut events).await;

    let (trusted, _) = TlsTransport::client_only(
        "127.0.0.1:0".parse().unwrap(),
        None,
        pki.client_config(Some((&pki.client_cert, &pki.client_key))),
    )
    .await
    .expect("trusted mTLS client");
    tokio::time::timeout(
        Duration::from_secs(3),
        trusted.send_message(tls_invite(tls_bind, "trusted"), tls_bind),
    )
    .await
    .expect("trusted handshake deadline")
    .expect("send verified mTLS INVITE");
    let admitted = next_authenticated_principal(&mut events).await;
    assert_eq!(admitted.ownership_key(), expected.ownership_key());
    assert_eq!(admitted.method, AuthenticationMethod::MutualTls);
    assert_eq!(admitted.tenant.as_deref(), Some(TENANT));

    adapter
        .coordinator()
        .shutdown_gracefully(Some(Duration::from_secs(1)))
        .await
        .expect("shutdown");
}

#[tokio::test]
#[serial]
async fn invalid_or_unverified_tenant_policy_is_rejected_before_listener_start() {
    let bind = reserve_udp_addr();
    let mut mismatched = principal("trusted-edge", Some("other-tenant"));
    mismatched.method = AuthenticationMethod::MutualTls;
    let mismatch_policy = SipListenerAuthPolicy::enabled_for_tenant(TENANT)
        .expect("tenant policy")
        .with_verified_mtls_peer("ab".repeat(32), mismatched);
    assert!(
        SipAdapter::from_config_with_listener_auth(signaling_config(bind), mismatch_policy,)
            .await
            .is_err()
    );

    let exact = principal("trusted-edge", Some(TENANT));
    let unverified_policy = SipListenerAuthPolicy::enabled_for_tenant(TENANT)
        .expect("tenant policy")
        .with_verified_mtls_peer("ab".repeat(32), exact);
    assert!(SipAdapter::from_config_with_listener_auth(
        signaling_config(reserve_udp_addr()),
        unverified_policy,
    )
    .await
    .is_err());

    assert!(SipListenerAuthPolicy::enabled_for_tenant(" tenant").is_err());
    assert!(SipListenerAuthPolicy::enabled_for_tenant("").is_err());
}
