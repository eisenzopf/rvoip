//! Production-authenticated MOQT-over-WebTransport browser test origin.
//!
//! This example starts role-separated publisher and subscriber listeners over
//! one managed relay topology. The publisher uses mutual TLS; the browser
//! listener accepts one short-lived, receive-only JWT through rvoip's normal
//! admission, replay, authorization, and lease pipeline.
//!
//! Exactly one confidential JSON descriptor is written to stdout when the
//! origin is ready. Operational diagnostics go to stderr and never contain
//! the JWT. The subscriber credential is deliberately not placed in a URL.

use std::io::Write as _;
use std::net::{SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rand::{rngs::OsRng, RngCore};
use rcgen::{CertificateParams, KeyPair, PKCS_ECDSA_P256_SHA256};
use rvoip_auth_core::{BearerValidator, JwtValidator};
use rvoip_core_traits::broadcast::{BroadcastPublisher, BroadcastSubstrate};
use rvoip_core_traits::ids::StreamId;
use rvoip_core_traits::stream::{MediaFrame, StreamKind};
use rvoip_moq::{
    BoundedMemoryMoqReplayStore, BoundedMemoryMoqSessionLeaseStore, MoqAction, MoqAuthorizer,
    MoqBroadcastPublisher, MoqPublisherConfig, MoqRelayAdmissionConfig, MoqRelayAdmissionSubstrate,
    MoqRelayClient, MoqRelayConnectionPolicy, MoqRelayDeploymentMode, MoqRelayPublisherBinding,
    MoqRelayRuntime, MoqRelayRuntimeConfig, MoqRelayRuntimeLimits, MoqRelayRuntimeSecurity,
    MoqRelayRuntimeTimeouts, MoqRelayServerTlsConfig, MoqRelaySubstratePolicy, MoqRelayTlsConfig,
    MoqRelayTopology, MoqResource, MoqRevocationChecker, MoqRevocationError, MoqRevocationStatus,
    MoqSessionLeaseLimits, MoqTokenBinding, RvoipMoqRelayAdmission, SecureMoqAuthorizer,
    AUDIO_TRACK, CATALOG_TRACK, LOC_DRAFT, MOQT_NEGOTIATED_PROTOCOL, MSF_DRAFT,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use time::OffsetDateTime;
use tokio_util::sync::CancellationToken;
use url::Url;
use zeroize::Zeroize;

const TENANT: &str = "browser-e2e";
const BROADCAST: &str = "webtransport";
const TOKEN_ISSUER: &str = "https://rvoip.test/moq-browser-e2e";
const TOKEN_AUDIENCE: &str = "rvoip-moq-browser-e2e";
const TOKEN_LIFETIME: chrono::Duration = chrono::Duration::minutes(2);
const CERTIFICATE_LIFETIME: time::Duration = time::Duration::days(1);
const NETWORK_TIMEOUT: Duration = Duration::from_secs(10);

type HarnessResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

struct EphemeralPki {
    _directory: TempDir,
    server_certificate: PathBuf,
    server_private_key: PathBuf,
    publisher_certificate: PathBuf,
    publisher_private_key: PathBuf,
    server_certificate_sha256: String,
    publisher_certificate_sha256: String,
    server_not_before: OffsetDateTime,
    server_not_after: OffsetDateTime,
}

impl EphemeralPki {
    fn generate() -> HarnessResult<Self> {
        let directory = tempfile::Builder::new()
            .prefix("rvoip-moq-browser-e2e-")
            .tempdir()?;
        let server_certificate = directory.path().join("server.pem");
        let server_private_key = directory.path().join("server.key");
        let publisher_certificate = directory.path().join("publisher.pem");
        let publisher_private_key = directory.path().join("publisher.key");

        let server = generate_short_lived_certificate("localhost")?;
        let publisher = generate_short_lived_certificate("publisher.test")?;
        std::fs::write(&server_certificate, server.certificate.pem())?;
        write_private_key(&server_private_key, &server.key_pair.serialize_pem())?;
        std::fs::write(&publisher_certificate, publisher.certificate.pem())?;
        write_private_key(&publisher_private_key, &publisher.key_pair.serialize_pem())?;

        Ok(Self {
            _directory: directory,
            server_certificate,
            server_private_key,
            publisher_certificate,
            publisher_private_key,
            server_certificate_sha256: lower_hex(&Sha256::digest(
                server.certificate.der().as_ref(),
            )),
            publisher_certificate_sha256: lower_hex(&Sha256::digest(
                publisher.certificate.der().as_ref(),
            )),
            server_not_before: server.not_before,
            server_not_after: server.not_after,
        })
    }

    fn publisher_server_tls(&self) -> MoqRelayServerTlsConfig {
        MoqRelayServerTlsConfig {
            server_certificates: vec![self.server_certificate.clone()],
            server_private_keys: vec![self.server_private_key.clone()],
            server_root_certificates: vec![self.server_certificate.clone()],
            publisher_client_ca_certificates: vec![self.publisher_certificate.clone()],
            ..MoqRelayServerTlsConfig::default()
        }
    }

    fn subscriber_server_tls(&self) -> MoqRelayServerTlsConfig {
        MoqRelayServerTlsConfig {
            server_certificates: vec![self.server_certificate.clone()],
            server_private_keys: vec![self.server_private_key.clone()],
            server_root_certificates: vec![self.server_certificate.clone()],
            ..MoqRelayServerTlsConfig::default()
        }
    }

    fn publisher_client_tls(&self) -> MoqRelayTlsConfig {
        MoqRelayTlsConfig {
            root_certificates: vec![self.server_certificate.clone()],
            client_certificate: Some(self.publisher_certificate.clone()),
            client_private_key: Some(self.publisher_private_key.clone()),
            #[cfg(feature = "insecure-development")]
            disable_verification: false,
        }
    }
}

struct GeneratedCertificate {
    certificate: rcgen::Certificate,
    key_pair: KeyPair,
    not_before: OffsetDateTime,
    not_after: OffsetDateTime,
}

fn generate_short_lived_certificate(name: &str) -> HarnessResult<GeneratedCertificate> {
    let now = OffsetDateTime::now_utc();
    let not_before = now - time::Duration::minutes(1);
    let not_after = now + CERTIFICATE_LIFETIME;
    let key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
    let mut parameters = CertificateParams::new(vec![name.to_owned()])?;
    parameters.not_before = not_before;
    parameters.not_after = not_after;
    let certificate = parameters.self_signed(&key_pair)?;
    Ok(GeneratedCertificate {
        certificate,
        key_pair,
        not_before,
        not_after,
    })
}

fn write_private_key(path: &Path, pem: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;

        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(pem.as_bytes())?;
        file.flush()
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, pem)
    }
}

#[derive(Serialize)]
struct SubscriberClaims<'a> {
    sub: &'a str,
    tenant_id: &'a str,
    scope: String,
    iss: &'a str,
    aud: &'a str,
    iat: u64,
    exp: u64,
    jti: String,
}

struct MintedSubscriberToken {
    encoded: String,
    expires_at: DateTime<Utc>,
    validator: Arc<dyn BearerValidator>,
}

fn mint_subscriber_token() -> HarnessResult<MintedSubscriberToken> {
    let issued_at = Utc::now();
    let expires_at = issued_at + TOKEN_LIFETIME;
    let mut secret = [0_u8; 32];
    let mut token_id = [0_u8; 16];
    OsRng.fill_bytes(&mut secret);
    OsRng.fill_bytes(&mut token_id);
    let claims = SubscriberClaims {
        sub: "browser-test-listener",
        tenant_id: TENANT,
        scope: format!("broadcast:subscribe:{BROADCAST}"),
        iss: TOKEN_ISSUER,
        aud: TOKEN_AUDIENCE,
        iat: issued_at.timestamp().try_into()?,
        exp: expires_at.timestamp().try_into()?,
        jti: lower_hex(&token_id),
    };
    let encoded = jsonwebtoken::encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(&secret),
    )?;
    let validator: Arc<dyn BearerValidator> = JwtValidator::from_hmac_secret(&secret)
        .with_issuer([TOKEN_ISSUER])
        .with_audience([TOKEN_AUDIENCE])
        .with_required_jti()
        .into_arc();
    secret.zeroize();
    token_id.zeroize();
    Ok(MintedSubscriberToken {
        encoded,
        expires_at,
        validator,
    })
}

struct AlwaysActiveRevocation;

#[async_trait]
impl MoqRevocationChecker for AlwaysActiveRevocation {
    async fn check(
        &self,
        _peer: &rvoip_moq::MoqPeerIdentity,
        _action: MoqAction,
        _resource: &MoqResource,
        _binding: &MoqTokenBinding,
        _now: DateTime<Utc>,
    ) -> Result<MoqRevocationStatus, MoqRevocationError> {
        Ok(MoqRevocationStatus::Active)
    }
}

fn subscriber_admission(
    validator: Arc<dyn BearerValidator>,
) -> HarnessResult<Arc<RvoipMoqRelayAdmission>> {
    let replay = Arc::new(BoundedMemoryMoqReplayStore::new(64)?);
    let revocation: Arc<dyn MoqRevocationChecker> = Arc::new(AlwaysActiveRevocation);
    let authorizer: Arc<dyn MoqAuthorizer> = Arc::new(SecureMoqAuthorizer::new(replay, revocation));
    let leases = Arc::new(BoundedMemoryMoqSessionLeaseStore::new(
        MoqSessionLeaseLimits::new(8, 8)?,
    )?);
    Ok(Arc::new(RvoipMoqRelayAdmission::with_config(
        validator,
        authorizer,
        leases,
        MoqRelayAdmissionConfig::for_substrate(
            Duration::from_secs(2),
            MoqRelayAdmissionSubstrate::WebTransport,
        )?,
    )?))
}

fn unused_udp_address() -> std::io::Result<SocketAddr> {
    let socket = UdpSocket::bind("127.0.0.1:0")?;
    socket.local_addr()
}

fn moqt_endpoint(address: SocketAddr) -> HarnessResult<Url> {
    Ok(Url::parse(&format!("moqt://localhost:{}", address.port()))?)
}

fn runtime_config(
    bind: SocketAddr,
    tls: MoqRelayServerTlsConfig,
    security: MoqRelayRuntimeSecurity,
) -> HarnessResult<MoqRelayRuntimeConfig> {
    Ok(MoqRelayRuntimeConfig {
        deployment: MoqRelayDeploymentMode::Embedded,
        bind,
        advertised_endpoint: moqt_endpoint(bind)?,
        advertised_socket_addr: Some(bind),
        tls,
        security,
        limits: MoqRelayRuntimeLimits::default(),
        timeouts: MoqRelayRuntimeTimeouts::default(),
    })
}

fn lower_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    bytes.iter().fold(
        String::with_capacity(bytes.len() * 2),
        |mut output, byte| {
            write!(output, "{byte:02x}").expect("writing to a String cannot fail");
            output
        },
    )
}

fn spawn_audio_source(
    publisher: &Arc<MoqBroadcastPublisher>,
) -> (CancellationToken, tokio::task::JoinHandle<()>) {
    let stop = CancellationToken::new();
    let task_stop = stop.clone();
    let frames = publisher.frames_out();
    let stream_id = StreamId::new();
    let task = tokio::spawn(async move {
        let mut timestamp_rtp = 0_u32;
        let mut cadence = tokio::time::interval(Duration::from_millis(20));
        cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                () = task_stop.cancelled() => break,
                _ = cadence.tick() => {
                    let frame = MediaFrame {
                        stream_id: stream_id.clone(),
                        kind: StreamKind::Audio,
                        // Hybrid configuration 15: one 20 ms mono Opus frame.
                        payload: Bytes::from_static(&[0x78, 0x00]),
                        timestamp_rtp,
                        captured_at: Utc::now(),
                        payload_type: Some(111),
                    };
                    match frames.try_send(frame) {
                        Ok(()) | Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {}
                        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => break,
                    }
                    timestamp_rtp = timestamp_rtp.wrapping_add(960);
                }
            }
        }
    });
    (stop, task)
}

#[derive(Serialize)]
struct ReadyDescriptor<'a> {
    kind: &'static str,
    endpoint: String,
    namespace: &'a str,
    catalog_track: &'static str,
    audio_track: &'static str,
    token: &'a str,
    token_expires_at: DateTime<Utc>,
    certificate_sha256: &'a str,
    certificate_not_before: String,
    certificate_not_after: String,
    protocol: &'static str,
    msf: &'static str,
    loc: &'static str,
    substrate: BroadcastSubstrate,
}

#[tokio::main]
async fn main() -> HarnessResult<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .try_init();

    let pki = EphemeralPki::generate()?;
    let subscriber_token = mint_subscriber_token()?;
    let publisher_address = unused_udp_address()?;
    let subscriber_address = unused_udp_address()?;
    if publisher_address == subscriber_address {
        return Err("publisher and subscriber listeners selected the same address".into());
    }
    let topology = MoqRelayTopology::new(
        moqt_endpoint(publisher_address)?,
        Some(publisher_address),
        MoqRelayRuntimeLimits::default().max_coordinated_namespaces,
    )?;

    let publisher_runtime = MoqRelayRuntime::start_with_topology(
        runtime_config(
            publisher_address,
            pki.publisher_server_tls(),
            MoqRelayRuntimeSecurity::PublisherMutualTls {
                bindings: vec![MoqRelayPublisherBinding {
                    certificate_sha256: pki.publisher_certificate_sha256.clone(),
                    scope: format!("/{TENANT}/{BROADCAST}"),
                }],
                max_active_sessions_per_certificate: 1,
            },
        )?,
        topology.clone(),
    )?;
    let subscriber_runtime = MoqRelayRuntime::start_with_topology(
        runtime_config(
            subscriber_address,
            pki.subscriber_server_tls(),
            MoqRelayRuntimeSecurity::SubscriberWebTransport {
                admission: subscriber_admission(subscriber_token.validator.clone())?,
            },
        )?,
        topology.clone(),
    )?;

    let publisher = MoqBroadcastPublisher::new(MoqPublisherConfig {
        tenant_id: TENANT.to_owned(),
        broadcast_id: BROADCAST.to_owned(),
        bitrate: 32_000,
        language: Some("en".to_owned()),
        queue_frames: 10,
    })?;
    let relay_client = MoqRelayClient::bind_with_policy(
        "127.0.0.1:0".parse()?,
        pki.publisher_client_tls(),
        MoqRelayConnectionPolicy {
            attempt_timeout: Duration::from_secs(5),
            publish_namespace_acceptance_timeout: Duration::from_secs(3),
            substrate: MoqRelaySubstratePolicy::RawQuic,
            max_reconnect_attempts: 1,
            reconnect_initial_backoff: Duration::from_millis(20),
            reconnect_max_backoff: Duration::from_millis(20),
            reconnect_deadline: Duration::from_secs(2),
            jitter_percent: 0,
        },
    )?;
    let publish_target = Url::parse(&format!(
        "moqt://localhost:{}/{TENANT}/{BROADCAST}",
        publisher_address.port()
    ))?;
    let publication = tokio::time::timeout(
        NETWORK_TIMEOUT,
        publisher.publish_to_relay(&relay_client, &publish_target),
    )
    .await
    .map_err(|_| "publisher connection timed out")??;
    if topology.coordinated_namespaces() != 1 {
        return Err("publisher namespace was not registered in the managed topology".into());
    }

    let (audio_stop, audio_task) = spawn_audio_source(&publisher);
    let namespace = format!("{TENANT}/{BROADCAST}");
    let ready = ReadyDescriptor {
        kind: "rvoip-moq-browser-e2e-ready",
        endpoint: format!(
            "https://127.0.0.1:{}/{namespace}",
            subscriber_address.port()
        ),
        namespace: &namespace,
        catalog_track: CATALOG_TRACK,
        audio_track: AUDIO_TRACK,
        token: &subscriber_token.encoded,
        token_expires_at: subscriber_token.expires_at,
        certificate_sha256: &pki.server_certificate_sha256,
        certificate_not_before: pki.server_not_before.to_string(),
        certificate_not_after: pki.server_not_after.to_string(),
        protocol: MOQT_NEGOTIATED_PROTOCOL,
        msf: MSF_DRAFT,
        loc: LOC_DRAFT,
        substrate: BroadcastSubstrate::WebTransport,
    };
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer(&mut stdout, &ready)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    eprintln!(
        "rvoip MOQT browser harness ready on WebTransport port {} (token omitted)",
        subscriber_address.port()
    );

    tokio::signal::ctrl_c().await?;
    eprintln!("rvoip MOQT browser harness draining");
    audio_stop.cancel();
    let _ = audio_task.await;
    Arc::clone(&publisher).close().await?;
    tokio::time::timeout(NETWORK_TIMEOUT, publication.wait())
        .await
        .map_err(|_| "publisher relay shutdown timed out")??;
    subscriber_runtime.drain(Duration::from_secs(5)).await?;
    publisher_runtime.drain(Duration::from_secs(5)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn minted_token_is_receive_only_tenant_bound_and_short_lived() {
        let token = mint_subscriber_token().unwrap();
        let principal = token
            .validator
            .validate_credential(&token.encoded)
            .await
            .unwrap()
            .principal;
        assert_eq!(principal.tenant.as_deref(), Some(TENANT));
        assert_eq!(
            principal.scopes,
            vec![format!("broadcast:subscribe:{BROADCAST}")]
        );
        assert!(!principal
            .scopes
            .iter()
            .any(|scope| scope.contains("publish")));
        assert!(token.expires_at <= Utc::now() + chrono::Duration::minutes(3));
    }

    #[test]
    fn browser_certificate_is_hash_pinned_and_valid_for_one_day() {
        let pki = EphemeralPki::generate().unwrap();
        assert_eq!(pki.server_certificate_sha256.len(), 64);
        assert!(
            pki.server_not_after - pki.server_not_before
                <= CERTIFICATE_LIFETIME + time::Duration::minutes(1)
        );
        assert!(pki.server_not_after - pki.server_not_before < time::Duration::days(14));
    }
}
