#![cfg(all(feature = "tls", feature = "wss"))]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose,
};
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::{Message, Method};
use rvoip_sip_transport::transport::tls::{
    TlsClientAuthMode, TlsClientConfig, TlsServerClientAuthConfig, TlsTransport,
};
use rvoip_sip_transport::transport::ws::WebSocketTransport;
use rvoip_sip_transport::transport::TransportConnectionMetadata;
use rvoip_sip_transport::{Transport, TransportEvent};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::sync::mpsc;

struct TestPki {
    _dir: TempDir,
    ca_cert: PathBuf,
    server_cert: PathBuf,
    server_key: PathBuf,
    client_cert: PathBuf,
    client_key: PathBuf,
    untrusted_client_cert: PathBuf,
    untrusted_client_key: PathBuf,
    client_fingerprint: String,
}

impl TestPki {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("temporary PKI directory");

        let ca_key = KeyPair::generate().expect("CA key");
        let mut ca_params =
            CertificateParams::new(vec!["Bridgefu Test CA".into()]).expect("CA params");
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
            CertificateParams::new(vec!["localhost".into()]).expect("server params");
        server_params.distinguished_name = DistinguishedName::new();
        server_params
            .distinguished_name
            .push(DnType::CommonName, "localhost");
        server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let server_cert = server_params
            .signed_by(&server_key, &issuer)
            .expect("CA-signed server certificate");

        let client_key = KeyPair::generate().expect("client key");
        let mut client_params =
            CertificateParams::new(vec!["bridgefu-client.test".into()]).expect("client params");
        client_params.distinguished_name = DistinguishedName::new();
        client_params
            .distinguished_name
            .push(DnType::CommonName, "bridgefu-client.test");
        client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let client_cert = client_params
            .signed_by(&client_key, &issuer)
            .expect("CA-signed client certificate");

        let untrusted_client_key = KeyPair::generate().expect("untrusted client key");
        let mut untrusted_client_params =
            CertificateParams::new(vec!["untrusted-client.test".into()])
                .expect("untrusted client params");
        untrusted_client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let untrusted_client_cert = untrusted_client_params
            .self_signed(&untrusted_client_key)
            .expect("self-signed untrusted client certificate");

        let ca_cert_path = dir.path().join("ca.pem");
        let server_cert_path = dir.path().join("server.pem");
        let server_key_path = dir.path().join("server-key.pem");
        let client_cert_path = dir.path().join("client.pem");
        let client_key_path = dir.path().join("client-key.pem");
        let untrusted_client_cert_path = dir.path().join("untrusted-client.pem");
        let untrusted_client_key_path = dir.path().join("untrusted-client-key.pem");
        write(&ca_cert_path, &ca_cert.pem());
        write(&server_cert_path, &server_cert.pem());
        write(&server_key_path, &server_key.serialize_pem());
        write(&client_cert_path, &client_cert.pem());
        write(&client_key_path, &client_key.serialize_pem());
        write(&untrusted_client_cert_path, &untrusted_client_cert.pem());
        write(
            &untrusted_client_key_path,
            &untrusted_client_key.serialize_pem(),
        );

        Self {
            _dir: dir,
            ca_cert: ca_cert_path,
            server_cert: server_cert_path,
            server_key: server_key_path,
            client_cert: client_cert_path,
            client_key: client_key_path,
            untrusted_client_cert: untrusted_client_cert_path,
            untrusted_client_key: untrusted_client_key_path,
            client_fingerprint: sha256_hex(client_cert.der().as_ref()),
        }
    }

    fn client_config(&self, present_certificate: bool) -> TlsClientConfig {
        TlsClientConfig {
            extra_ca_path: Some(self.ca_cert.clone()),
            client_cert_path: present_certificate.then(|| self.client_cert.clone()),
            client_key_path: present_certificate.then(|| self.client_key.clone()),
            ..Default::default()
        }
    }

    fn required_client_auth(&self) -> TlsServerClientAuthConfig {
        TlsServerClientAuthConfig::required(self.ca_cert.clone())
    }

    fn optional_client_auth(&self) -> TlsServerClientAuthConfig {
        TlsServerClientAuthConfig::optional(self.ca_cert.clone())
    }

    fn untrusted_client_config(&self) -> TlsClientConfig {
        TlsClientConfig {
            extra_ca_path: Some(self.ca_cert.clone()),
            client_cert_path: Some(self.untrusted_client_cert.clone()),
            client_key_path: Some(self.untrusted_client_key.clone()),
            ..Default::default()
        }
    }
}

fn write(path: &Path, contents: &str) {
    std::fs::write(path, contents).expect("write test certificate material");
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn loopback() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

fn register(call_id: &str) -> Message {
    Message::Request(
        SimpleRequestBuilder::new(Method::Register, "sips:localhost;transport=tls")
            .unwrap()
            .from("alice", "sips:alice@localhost", Some("mtls-test-tag"))
            .to("alice", "sips:alice@localhost", None)
            .call_id(call_id)
            .cseq(1)
            .build(),
    )
}

async fn receive_message(
    events: &mut mpsc::Receiver<TransportEvent>,
    expected_call_id: &str,
) -> Option<TransportConnectionMetadata> {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match events.recv().await.expect("transport event channel closed") {
                TransportEvent::MessageReceived {
                    message,
                    connection_metadata,
                    ..
                } => {
                    let Message::Request(request) = message else {
                        panic!("expected SIP request");
                    };
                    assert_eq!(request.method(), Method::Register);
                    assert_eq!(request.call_id().unwrap().to_string(), expected_call_id);
                    return connection_metadata;
                }
                _ => continue,
            }
        }
    })
    .await
    .expect("timed out waiting for SIP message")
}

async fn assert_no_message(events: &mut mpsc::Receiver<TransportEvent>) {
    let message_seen = tokio::time::timeout(Duration::from_millis(300), async {
        loop {
            match events.recv().await {
                Some(TransportEvent::MessageReceived { .. }) => return true,
                Some(_) => continue,
                None => return false,
            }
        }
    })
    .await;
    assert!(!matches!(message_seen, Ok(true)));
}

async fn assert_tls_connection_closed(client: &TlsTransport, destination: SocketAddr) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while client.has_connection_to(destination) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("rejected TLS connection remained registered");
}

fn assert_verified_client(metadata: Option<TransportConnectionMetadata>, pki: &TestPki) {
    let identity = metadata
        .expect("mTLS connection metadata")
        .tls_peer_identity;
    assert_eq!(identity.leaf_certificate_sha256, pki.client_fingerprint);
    assert_eq!(identity.presented_chain_len, 1);
}

#[tokio::test]
async fn tls_required_without_client_ca_is_rejected_at_bind() {
    let pki = TestPki::new();
    let result = TlsTransport::bind_server_only_with_client_auth(
        loopback(),
        &pki.server_cert,
        &pki.server_key,
        None,
        TlsServerClientAuthConfig {
            mode: TlsClientAuthMode::Required,
            client_ca_path: None,
        },
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn wss_optional_without_client_ca_is_rejected_at_bind() {
    let pki = TestPki::new();
    let result = WebSocketTransport::bind_with_tls_configs(
        loopback(),
        true,
        pki.server_cert.to_str(),
        pki.server_key.to_str(),
        None,
        None,
        TlsServerClientAuthConfig {
            mode: TlsClientAuthMode::Optional,
            client_ca_path: None,
        },
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tls_legacy_bind_keeps_client_auth_disabled_by_default() {
    let pki = TestPki::new();
    let (server, mut events) =
        TlsTransport::bind(loopback(), &pki.server_cert, &pki.server_key, None)
            .await
            .expect("legacy TLS listener");
    let (client, _) = TlsTransport::client_only(loopback(), None, pki.client_config(false))
        .await
        .expect("anonymous TLS client");

    client
        .send_message(
            register("tls-disabled-default"),
            server.local_addr().unwrap(),
        )
        .await
        .expect("legacy TLS anonymous send");
    assert!(receive_message(&mut events, "tls-disabled-default")
        .await
        .is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wss_legacy_bind_keeps_client_auth_disabled_by_default() {
    let pki = TestPki::new();
    let (server, mut events) = WebSocketTransport::bind(
        loopback(),
        true,
        pki.server_cert.to_str(),
        pki.server_key.to_str(),
        None,
    )
    .await
    .expect("legacy WSS listener");
    let (client, _) = WebSocketTransport::bind_with_client_tls(
        loopback(),
        true,
        pki.server_cert.to_str(),
        pki.server_key.to_str(),
        None,
        Some(pki.client_config(false)),
    )
    .await
    .expect("anonymous WSS client");

    client
        .send_message(
            register("wss-disabled-default"),
            server.local_addr().unwrap(),
        )
        .await
        .expect("legacy WSS anonymous send");
    assert!(receive_message(&mut events, "wss-disabled-default")
        .await
        .is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tls_required_accepts_trusted_client_and_propagates_identity() {
    let pki = TestPki::new();
    let (server, mut events) = TlsTransport::bind_server_only_with_client_auth(
        loopback(),
        &pki.server_cert,
        &pki.server_key,
        None,
        pki.required_client_auth(),
    )
    .await
    .expect("required-mTLS TLS listener");
    let destination = server.local_addr().unwrap();
    let (client, _) = TlsTransport::client_only(loopback(), None, pki.client_config(true))
        .await
        .expect("mTLS client");

    client
        .send_message(register("tls-required-positive"), destination)
        .await
        .expect("trusted mTLS send");
    assert_verified_client(
        receive_message(&mut events, "tls-required-positive").await,
        &pki,
    );
    client
        .send_message(register("tls-required-second"), destination)
        .await
        .expect("second trusted mTLS send");
    assert_verified_client(
        receive_message(&mut events, "tls-required-second").await,
        &pki,
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tls_required_rejects_client_without_certificate() {
    let pki = TestPki::new();
    let (server, mut events) = TlsTransport::bind_server_only_with_client_auth(
        loopback(),
        &pki.server_cert,
        &pki.server_key,
        None,
        pki.required_client_auth(),
    )
    .await
    .expect("required-mTLS TLS listener");
    let (client, _) = TlsTransport::client_only(loopback(), None, pki.client_config(false))
        .await
        .expect("anonymous TLS client");

    let destination = server.local_addr().unwrap();
    let _ = client
        .send_message(register("tls-required-negative"), destination)
        .await;
    assert_no_message(&mut events).await;
    assert_tls_connection_closed(&client, destination).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tls_optional_accepts_client_without_certificate_and_emits_no_identity() {
    let pki = TestPki::new();
    let (server, mut events) = TlsTransport::bind_server_only_with_client_auth(
        loopback(),
        &pki.server_cert,
        &pki.server_key,
        None,
        pki.optional_client_auth(),
    )
    .await
    .expect("optional-mTLS TLS listener");
    let (client, _) = TlsTransport::client_only(loopback(), None, pki.client_config(false))
        .await
        .expect("anonymous TLS client");

    client
        .send_message(
            register("tls-optional-anonymous"),
            server.local_addr().unwrap(),
        )
        .await
        .expect("optional mTLS anonymous send");
    assert!(receive_message(&mut events, "tls-optional-anonymous")
        .await
        .is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tls_optional_rejects_an_untrusted_presented_certificate() {
    let pki = TestPki::new();
    let (server, mut events) = TlsTransport::bind_server_only_with_client_auth(
        loopback(),
        &pki.server_cert,
        &pki.server_key,
        None,
        pki.optional_client_auth(),
    )
    .await
    .expect("optional-mTLS TLS listener");
    let (client, _) = TlsTransport::client_only(loopback(), None, pki.untrusted_client_config())
        .await
        .expect("untrusted TLS client");

    let destination = server.local_addr().unwrap();
    let _ = client
        .send_message(register("tls-optional-untrusted"), destination)
        .await;
    assert_no_message(&mut events).await;
    assert_tls_connection_closed(&client, destination).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wss_required_accepts_trusted_client_and_propagates_identity() {
    let pki = TestPki::new();
    let (server, mut events) = WebSocketTransport::bind_with_tls_configs(
        loopback(),
        true,
        pki.server_cert.to_str(),
        pki.server_key.to_str(),
        None,
        None,
        pki.required_client_auth(),
    )
    .await
    .expect("required-mTLS WSS listener");
    let (client, _) = WebSocketTransport::bind_with_tls_configs(
        loopback(),
        true,
        pki.server_cert.to_str(),
        pki.server_key.to_str(),
        None,
        Some(pki.client_config(true)),
        TlsServerClientAuthConfig::default(),
    )
    .await
    .expect("mTLS WSS client");

    let destination = server.local_addr().unwrap();
    client
        .send_message(register("wss-required-positive"), destination)
        .await
        .expect("trusted mTLS WSS send");
    assert_verified_client(
        receive_message(&mut events, "wss-required-positive").await,
        &pki,
    );
    client
        .send_message(register("wss-required-second"), destination)
        .await
        .expect("second trusted mTLS WSS send");
    assert_verified_client(
        receive_message(&mut events, "wss-required-second").await,
        &pki,
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wss_required_rejects_client_without_certificate() {
    let pki = TestPki::new();
    let (server, mut events) = WebSocketTransport::bind_with_tls_configs(
        loopback(),
        true,
        pki.server_cert.to_str(),
        pki.server_key.to_str(),
        None,
        None,
        pki.required_client_auth(),
    )
    .await
    .expect("required-mTLS WSS listener");
    let (client, _) = WebSocketTransport::bind_with_tls_configs(
        loopback(),
        true,
        pki.server_cert.to_str(),
        pki.server_key.to_str(),
        None,
        Some(pki.client_config(false)),
        TlsServerClientAuthConfig::default(),
    )
    .await
    .expect("anonymous WSS client");

    let result = client
        .send_message(
            register("wss-required-negative"),
            server.local_addr().unwrap(),
        )
        .await;
    assert!(
        result.is_err(),
        "required WSS mTLS accepted an anonymous client"
    );
    assert_no_message(&mut events).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wss_optional_accepts_client_without_certificate_and_emits_no_identity() {
    let pki = TestPki::new();
    let (server, mut events) = WebSocketTransport::bind_with_tls_configs(
        loopback(),
        true,
        pki.server_cert.to_str(),
        pki.server_key.to_str(),
        None,
        None,
        pki.optional_client_auth(),
    )
    .await
    .expect("optional-mTLS WSS listener");
    let (client, _) = WebSocketTransport::bind_with_tls_configs(
        loopback(),
        true,
        pki.server_cert.to_str(),
        pki.server_key.to_str(),
        None,
        Some(pki.client_config(false)),
        TlsServerClientAuthConfig::default(),
    )
    .await
    .expect("anonymous WSS client");

    client
        .send_message(
            register("wss-optional-anonymous"),
            server.local_addr().unwrap(),
        )
        .await
        .expect("optional mTLS WSS send");
    assert!(receive_message(&mut events, "wss-optional-anonymous")
        .await
        .is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wss_optional_rejects_an_untrusted_presented_certificate() {
    let pki = TestPki::new();
    let (server, mut events) = WebSocketTransport::bind_with_tls_configs(
        loopback(),
        true,
        pki.server_cert.to_str(),
        pki.server_key.to_str(),
        None,
        None,
        pki.optional_client_auth(),
    )
    .await
    .expect("optional-mTLS WSS listener");
    let (client, _) = WebSocketTransport::bind_with_tls_configs(
        loopback(),
        true,
        pki.server_cert.to_str(),
        pki.server_key.to_str(),
        None,
        Some(pki.untrusted_client_config()),
        TlsServerClientAuthConfig::default(),
    )
    .await
    .expect("untrusted WSS client");

    let result = client
        .send_message(
            register("wss-optional-untrusted"),
            server.local_addr().unwrap(),
        )
        .await;
    assert!(
        result.is_err(),
        "optional WSS mTLS accepted an untrusted presented certificate"
    );
    assert_no_message(&mut events).await;
}
