// SPDX-FileCopyrightText: 2026 Bridgefu contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{fs, path::PathBuf, time::Duration};

use anyhow::Context as _;
use moq_native_ietf::{quic, tls};
use moq_transport::session::SessionTarget;
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose,
};
use tempfile::TempDir;
use time::{OffsetDateTime, Time};
use tokio::time::timeout;

mod support;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const REJECT_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Clone)]
struct IdentityFiles {
    cert: PathBuf,
    key: PathBuf,
    leaf_der: Vec<u8>,
}

struct TestPki {
    _directory: TempDir,
    trusted_ca: PathBuf,
    valid: IdentityFiles,
    untrusted: IdentityFiles,
    expired: IdentityFiles,
    wrong_usage: IdentityFiles,
}

impl TestPki {
    fn new() -> anyhow::Result<Self> {
        let directory = tempfile::tempdir()?;
        let (trusted_ca, trusted_key) = certificate_authority()?;
        let (untrusted_ca, untrusted_key) = certificate_authority()?;
        let trusted_ca_path = directory.path().join("trusted-ca.pem");
        fs::write(&trusted_ca_path, trusted_ca.pem())?;

        let valid = client_identity(
            directory.path(),
            "valid",
            &trusted_ca,
            &trusted_key,
            Validity::Current,
            ExtendedKeyUsagePurpose::ClientAuth,
        )?;
        let untrusted = client_identity(
            directory.path(),
            "untrusted",
            &untrusted_ca,
            &untrusted_key,
            Validity::Current,
            ExtendedKeyUsagePurpose::ClientAuth,
        )?;
        let expired = client_identity(
            directory.path(),
            "expired",
            &trusted_ca,
            &trusted_key,
            Validity::Expired,
            ExtendedKeyUsagePurpose::ClientAuth,
        )?;
        let wrong_usage = client_identity(
            directory.path(),
            "wrong-usage",
            &trusted_ca,
            &trusted_key,
            Validity::Current,
            ExtendedKeyUsagePurpose::ServerAuth,
        )?;

        Ok(Self {
            _directory: directory,
            trusted_ca: trusted_ca_path,
            valid,
            untrusted,
            expired,
            wrong_usage,
        })
    }
}

enum Validity {
    Current,
    Expired,
}

fn certificate_authority() -> anyhow::Result<(Certificate, KeyPair)> {
    let mut params = CertificateParams::new(Vec::<String>::new())?;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
    ];
    params.not_before = OffsetDateTime::now_utc() - time::Duration::days(1);
    params.not_after = OffsetDateTime::now_utc() + time::Duration::days(365);
    let key = KeyPair::generate()?;
    let certificate = params.self_signed(&key)?;
    Ok((certificate, key))
}

fn client_identity(
    directory: &std::path::Path,
    name: &str,
    ca: &Certificate,
    ca_key: &KeyPair,
    validity: Validity,
    usage: ExtendedKeyUsagePurpose,
) -> anyhow::Result<IdentityFiles> {
    let mut params = CertificateParams::new(vec![format!("{name}.client.test")])?;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![usage];
    match validity {
        Validity::Current => {
            params.not_before = OffsetDateTime::now_utc() - time::Duration::days(1);
            params.not_after = OffsetDateTime::now_utc() + time::Duration::days(1);
        }
        Validity::Expired => {
            params.not_before = OffsetDateTime::UNIX_EPOCH;
            params.not_after = OffsetDateTime::UNIX_EPOCH.replace_time(Time::from_hms(1, 0, 0)?);
        }
    }
    let key = KeyPair::generate()?;
    let certificate = params.signed_by(&key, ca, ca_key)?;
    let cert = directory.join(format!("{name}-cert.pem"));
    let key_path = directory.join(format!("{name}-key.pem"));
    fs::write(&cert, certificate.pem())?;
    fs::write(&key_path, key.serialize_pem())?;
    Ok(IdentityFiles {
        cert,
        key: key_path,
        leaf_der: certificate.der().as_ref().to_vec(),
    })
}

fn server_tls(
    ca: &std::path::Path,
    client_auth: tls::ClientAuthMode,
) -> anyhow::Result<tls::Config> {
    let client_ca = if client_auth == tls::ClientAuthMode::Disabled {
        Vec::new()
    } else {
        vec![ca.to_path_buf()]
    };
    support::localhost_server_tls(client_auth, &client_ca)
}

fn client_tls(identity: Option<&IdentityFiles>) -> anyhow::Result<tls::Config> {
    tls::Args {
        client_cert: identity.map(|identity| identity.cert.clone()),
        client_key: identity.map(|identity| identity.key.clone()),
        disable_verify: true,
        ..Default::default()
    }
    .load()
}

async fn endpoints(
    pki: &TestPki,
    identity: Option<&IdentityFiles>,
    client_auth: tls::ClientAuthMode,
) -> anyhow::Result<(quic::Client, quic::Server, SessionTarget)> {
    let server_endpoint = quic::Endpoint::new(quic::Config::new(
        "127.0.0.1:0".parse()?,
        None,
        server_tls(&pki.trusted_ca, client_auth)?,
    )?)?;
    let server = server_endpoint
        .server
        .context("test endpoint did not expose a server")?;
    let server_addr = server.local_addr()?;

    let client_endpoint = quic::Endpoint::new(quic::Config::new(
        "127.0.0.1:0".parse()?,
        None,
        client_tls(identity)?,
    )?)?;
    let target = format!("moqt://localhost:{}", server_addr.port()).parse()?;
    Ok((client_endpoint.client, server, target))
}

async fn assert_rejected(pki: &TestPki, identity: Option<&IdentityFiles>) -> anyhow::Result<()> {
    let (client, mut server, target) =
        endpoints(pki, identity, tls::ClientAuthMode::Required).await?;
    let address = server.local_addr()?;
    let (client_result, server_result) = tokio::join!(
        timeout(
            CONNECT_TIMEOUT,
            client.connect_target(&target, quic::SubstratePolicy::RawQuic, Some(address)),
        ),
        timeout(REJECT_TIMEOUT, server.accept_connection()),
    );
    if let Ok(connection) = client_result.context("client TLS attempt timed out")? {
        timeout(CONNECT_TIMEOUT, connection.session.closed())
            .await
            .context("client connection was not closed after server-side TLS rejection")?;
    }
    assert!(
        server_result.is_err(),
        "server surfaced an unauthenticated connection"
    );
    Ok(())
}

#[tokio::test]
async fn required_mtls_accepts_a_trusted_client_and_exposes_bounded_identity() -> anyhow::Result<()>
{
    let pki = TestPki::new()?;
    let (client, mut server, target) =
        endpoints(&pki, Some(&pki.valid), tls::ClientAuthMode::Required).await?;
    let address = server.local_addr()?;
    let (client_connection, server_connection) = tokio::join!(
        timeout(
            CONNECT_TIMEOUT,
            client.connect_target(&target, quic::SubstratePolicy::RawQuic, Some(address)),
        ),
        timeout(CONNECT_TIMEOUT, server.accept_connection()),
    );
    let client_connection = client_connection??;
    assert!(matches!(
        &client_connection.peer_identity,
        tls::PeerIdentity::UnverifiedCertificate(_)
    ));
    assert!(!client_connection.peer_identity.is_authenticated());
    let server_connection =
        server_connection?.context("server stopped before accepting authenticated connection")?;
    let identity = server_connection
        .peer_identity
        .certificate()
        .context("verified peer certificate identity was not retained")?;
    let expected = ring::digest::digest(&ring::digest::SHA256, &pki.valid.leaf_der);
    assert_eq!(identity.leaf_sha256().as_slice(), expected.as_ref());
    assert_eq!(identity.chain_len(), 1);
    assert!(identity.total_der_bytes() > 0);
    let debug = format!("{:?}", server_connection.peer_identity);
    assert!(debug.contains('…'));
    assert!(!debug.contains(&identity.leaf_sha256_hex()));
    Ok(())
}

#[tokio::test]
async fn required_mtls_rejects_missing_client_certificate() -> anyhow::Result<()> {
    let pki = TestPki::new()?;
    assert_rejected(&pki, None).await
}

#[tokio::test]
async fn required_mtls_rejects_untrusted_client_chain() -> anyhow::Result<()> {
    let pki = TestPki::new()?;
    assert_rejected(&pki, Some(&pki.untrusted)).await
}

#[tokio::test]
async fn required_mtls_rejects_expired_client_certificate() -> anyhow::Result<()> {
    let pki = TestPki::new()?;
    assert_rejected(&pki, Some(&pki.expired)).await
}

#[tokio::test]
async fn required_mtls_rejects_certificate_with_wrong_extended_usage() -> anyhow::Result<()> {
    let pki = TestPki::new()?;
    assert_rejected(&pki, Some(&pki.wrong_usage)).await
}

async fn assert_anonymous_is_explicit(mode: tls::ClientAuthMode) -> anyhow::Result<()> {
    let pki = TestPki::new()?;
    let (client, mut server, target) = endpoints(&pki, None, mode).await?;
    let address = server.local_addr()?;
    let (client_connection, server_connection) = tokio::join!(
        timeout(
            CONNECT_TIMEOUT,
            client.connect_target(&target, quic::SubstratePolicy::RawQuic, Some(address)),
        ),
        timeout(CONNECT_TIMEOUT, server.accept_connection()),
    );
    let _client_connection = client_connection??;
    let server_connection = server_connection?.context("server did not accept anonymous peer")?;
    assert_eq!(
        server_connection.peer_identity,
        tls::PeerIdentity::Anonymous
    );
    assert!(!server_connection.peer_identity.is_authenticated());
    Ok(())
}

#[tokio::test]
async fn optional_client_auth_marks_missing_certificate_anonymous() -> anyhow::Result<()> {
    assert_anonymous_is_explicit(tls::ClientAuthMode::Optional).await
}

#[tokio::test]
async fn disabled_client_auth_marks_peer_anonymous() -> anyhow::Result<()> {
    assert_anonymous_is_explicit(tls::ClientAuthMode::Disabled).await
}

#[tokio::test]
async fn required_mtls_verifies_clients_over_webtransport() -> anyhow::Result<()> {
    let pki = TestPki::new()?;
    let (client, mut server, target) =
        endpoints(&pki, Some(&pki.valid), tls::ClientAuthMode::Required).await?;
    let address = server.local_addr()?;
    let (client_connection, server_connection) = tokio::join!(
        timeout(
            CONNECT_TIMEOUT,
            client.connect_target(&target, quic::SubstratePolicy::WebTransport, Some(address),),
        ),
        timeout(CONNECT_TIMEOUT, server.accept_connection()),
    );
    let _client_connection = client_connection??;
    let server_connection =
        server_connection?.context("server did not accept authenticated WebTransport peer")?;
    assert!(server_connection.peer_identity.is_authenticated());
    assert_eq!(
        server_connection.negotiated.substrate,
        moq_transport::session::Transport::WebTransport
    );
    Ok(())
}
