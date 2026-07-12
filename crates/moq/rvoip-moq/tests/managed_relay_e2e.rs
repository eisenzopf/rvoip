#![cfg(feature = "relay-runtime")]

use std::net::{SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rvoip_auth_core::{BearerAuthError, BearerValidator, ValidatedBearer};
use rvoip_core_traits::broadcast::{
    BroadcastLifecycleState, BroadcastPublisher, BroadcastSubstrate,
};
use rvoip_core_traits::identity::IdentityAssurance;
use rvoip_core_traits::{AuthenticatedPrincipal, AuthenticationMethod};
use rvoip_moq::{
    BoundedMemoryMoqReplayStore, BoundedMemoryMoqSessionLeaseStore, MoqAction, MoqAuthorizer,
    MoqBroadcastPublisher, MoqCatalogDeliveryMode, MoqCatalogSubscriber,
    MoqCatalogSubscriberConfig, MoqCatalogSubscriberLifecycle, MoqCatalogSubscriberTlsConfig,
    MoqNamespace, MoqPeerIdentity, MoqPublisherConfig, MoqRelayAdmissionConfig,
    MoqRelayAdmissionSubstrate, MoqRelayClient, MoqRelayConnectionPolicy, MoqRelayDeploymentMode,
    MoqRelayPublisherBinding, MoqRelayRuntime, MoqRelayRuntimeConfig, MoqRelayRuntimeLimits,
    MoqRelayRuntimeSecurity, MoqRelayRuntimeTimeouts, MoqRelayServerTlsConfig,
    MoqRelaySubstratePolicy, MoqRelayTlsConfig, MoqRelayTopology, MoqResource,
    MoqRevocationChecker, MoqRevocationError, MoqRevocationStatus, MoqSessionLeaseLimits,
    MoqSubscriberCredential, MoqSubscriberCredentialError, MoqSubscriberCredentialProvider,
    MoqSubscriberCredentialRequest, MoqTokenBinding, MsfCatalogState, RvoipMoqRelayAdmission,
    SecureMoqAuthorizer, MOQT_NEGOTIATED_PROTOCOL,
};
use sha2::{Digest, Sha256};
use url::Url;

const TENANT: &str = "tenant-a";
const BROADCAST: &str = "broadcast-1";
const NETWORK_TIMEOUT: Duration = Duration::from_secs(10);

struct TestPki {
    directory: PathBuf,
    server_certificate: PathBuf,
    server_private_key: PathBuf,
    publisher_certificate: PathBuf,
    publisher_private_key: PathBuf,
    publisher_fingerprint: String,
}

impl TestPki {
    fn new() -> Self {
        static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);
        let server = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let publisher = rcgen::generate_simple_self_signed(vec!["publisher.test".into()]).unwrap();
        let directory = std::env::temp_dir().join(format!(
            "rvoip-moq-managed-relay-e2e-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let server_certificate = directory.join("server.pem");
        let server_private_key = directory.join("server.key");
        let publisher_certificate = directory.join("publisher.pem");
        let publisher_private_key = directory.join("publisher.key");
        std::fs::write(&server_certificate, server.cert.pem()).unwrap();
        std::fs::write(&server_private_key, server.signing_key.serialize_pem()).unwrap();
        std::fs::write(&publisher_certificate, publisher.cert.pem()).unwrap();
        std::fs::write(
            &publisher_private_key,
            publisher.signing_key.serialize_pem(),
        )
        .unwrap();
        Self {
            directory,
            server_certificate,
            server_private_key,
            publisher_certificate,
            publisher_private_key,
            publisher_fingerprint: lower_hex(&Sha256::digest(publisher.cert.der().as_ref())),
        }
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

impl Drop for TestPki {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

#[derive(Clone)]
struct TestBearerValidator {
    principal: AuthenticatedPrincipal,
}

#[async_trait]
impl BearerValidator for TestBearerValidator {
    async fn validate(&self, token: &str) -> Result<IdentityAssurance, BearerAuthError> {
        if token.is_empty() {
            return Err(BearerAuthError::Empty);
        }
        Ok(self.principal.assurance.clone())
    }

    async fn validate_credential(&self, token: &str) -> Result<ValidatedBearer, BearerAuthError> {
        if token.is_empty() {
            return Err(BearerAuthError::Empty);
        }
        Ok(ValidatedBearer {
            principal: self.principal.clone(),
            token_id: Some(format!("test-{token}")),
            issued_at: None,
        })
    }
}

struct AlwaysActiveRevocation;

#[async_trait]
impl MoqRevocationChecker for AlwaysActiveRevocation {
    async fn check(
        &self,
        _peer: &MoqPeerIdentity,
        _action: MoqAction,
        _resource: &MoqResource,
        _binding: &MoqTokenBinding,
        _now: DateTime<Utc>,
    ) -> Result<MoqRevocationStatus, MoqRevocationError> {
        Ok(MoqRevocationStatus::Active)
    }
}

struct FreshCredentials {
    next: AtomicU64,
}

#[async_trait]
impl MoqSubscriberCredentialProvider for FreshCredentials {
    async fn issue(
        &self,
        _request: MoqSubscriberCredentialRequest,
    ) -> Result<MoqSubscriberCredential, MoqSubscriberCredentialError> {
        let next = self.next.fetch_add(1, Ordering::Relaxed);
        MoqSubscriberCredential::new(format!("managed-relay-token-{next}").into_bytes())
    }
}

fn subscriber_admission(substrate: MoqRelayAdmissionSubstrate) -> Arc<RvoipMoqRelayAdmission> {
    let principal = AuthenticatedPrincipal {
        subject: "listener".to_owned(),
        tenant: Some(TENANT.to_owned()),
        scopes: vec![format!("broadcast:subscribe:{BROADCAST}")],
        issuer: Some("https://issuer.test".to_owned()),
        expires_at: Some(Utc::now() + chrono::Duration::minutes(5)),
        method: AuthenticationMethod::Jwt,
        assurance: IdentityAssurance::Anonymous,
    };
    let validator: Arc<dyn BearerValidator> = Arc::new(TestBearerValidator { principal });
    let replay = Arc::new(BoundedMemoryMoqReplayStore::new(32).unwrap());
    let revocation: Arc<dyn MoqRevocationChecker> = Arc::new(AlwaysActiveRevocation);
    let authorizer: Arc<dyn MoqAuthorizer> = Arc::new(SecureMoqAuthorizer::new(replay, revocation));
    let leases = Arc::new(
        BoundedMemoryMoqSessionLeaseStore::new(MoqSessionLeaseLimits::new(8, 8).unwrap()).unwrap(),
    );
    Arc::new(
        RvoipMoqRelayAdmission::with_config(
            validator,
            authorizer,
            leases,
            MoqRelayAdmissionConfig::for_substrate(Duration::from_secs(2), substrate).unwrap(),
        )
        .unwrap(),
    )
}

fn unused_udp_address() -> SocketAddr {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.local_addr().unwrap()
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

fn endpoint(address: SocketAddr) -> Url {
    Url::parse(&format!("moqt://localhost:{}", address.port())).unwrap()
}

fn runtime_config(
    bind: SocketAddr,
    tls: MoqRelayServerTlsConfig,
    security: MoqRelayRuntimeSecurity,
) -> MoqRelayRuntimeConfig {
    MoqRelayRuntimeConfig {
        deployment: MoqRelayDeploymentMode::Embedded,
        bind,
        advertised_endpoint: endpoint(bind),
        advertised_socket_addr: Some(bind),
        tls,
        security,
        limits: MoqRelayRuntimeLimits::default(),
        timeouts: MoqRelayRuntimeTimeouts::default(),
    }
}

async fn wait_for_live_catalog(
    subscriber: &MoqCatalogSubscriber,
) -> rvoip_moq::MoqCatalogSubscriptionSnapshot {
    let mut updates = subscriber.updates();
    match tokio::time::timeout(NETWORK_TIMEOUT, async {
        loop {
            let snapshot = updates.borrow_and_update().clone();
            if snapshot.lifecycle == MoqCatalogSubscriberLifecycle::Live {
                return snapshot;
            }
            assert!(
                !snapshot.lifecycle.is_terminal(),
                "managed subscriber terminated before receiving a live catalog: {snapshot:?}"
            );
            updates
                .changed()
                .await
                .expect("subscriber status channel closed");
        }
    })
    .await
    {
        Ok(snapshot) => snapshot,
        Err(_) => panic!(
            "managed catalog subscriber timed out: {:?}",
            subscriber.snapshot()
        ),
    }
}

async fn assert_managed_relay_path(substrate: MoqRelaySubstratePolicy) {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
    let pki = TestPki::new();
    let publisher_address = unused_udp_address();
    let subscriber_address = unused_udp_address();
    assert_ne!(publisher_address, subscriber_address);
    let topology = MoqRelayTopology::new(
        endpoint(publisher_address),
        Some(publisher_address),
        MoqRelayRuntimeLimits::default().max_coordinated_namespaces,
    )
    .unwrap();

    let publisher_runtime = MoqRelayRuntime::start_with_topology(
        runtime_config(
            publisher_address,
            pki.publisher_server_tls(),
            MoqRelayRuntimeSecurity::PublisherMutualTls {
                bindings: vec![MoqRelayPublisherBinding {
                    certificate_sha256: pki.publisher_fingerprint.clone(),
                    scope: format!("/{TENANT}/{BROADCAST}"),
                }],
                max_active_sessions_per_certificate: 4,
            },
        ),
        topology.clone(),
    )
    .unwrap();

    let (admission_substrate, subscriber_security) = match substrate {
        MoqRelaySubstratePolicy::RawQuic => {
            let admission = subscriber_admission(MoqRelayAdmissionSubstrate::RawQuic);
            (
                BroadcastSubstrate::RawQuic,
                MoqRelayRuntimeSecurity::SubscriberRawQuic { admission },
            )
        }
        MoqRelaySubstratePolicy::WebTransport => {
            let admission = subscriber_admission(MoqRelayAdmissionSubstrate::WebTransport);
            (
                BroadcastSubstrate::WebTransport,
                MoqRelayRuntimeSecurity::SubscriberWebTransport { admission },
            )
        }
        _ => panic!("test requires one exact subscriber substrate"),
    };
    let subscriber_runtime = MoqRelayRuntime::start_with_topology(
        runtime_config(
            subscriber_address,
            pki.subscriber_server_tls(),
            subscriber_security,
        ),
        topology.clone(),
    )
    .unwrap();

    let publisher = MoqBroadcastPublisher::new(MoqPublisherConfig {
        tenant_id: TENANT.to_owned(),
        broadcast_id: BROADCAST.to_owned(),
        bitrate: 32_000,
        language: Some("en".to_owned()),
        queue_frames: 10,
    })
    .unwrap();
    let relay_client = MoqRelayClient::bind_with_policy(
        "127.0.0.1:0".parse().unwrap(),
        pki.publisher_client_tls(),
        MoqRelayConnectionPolicy {
            attempt_timeout: Duration::from_secs(5),
            publish_namespace_acceptance_timeout: Duration::from_secs(3),
            substrate: MoqRelaySubstratePolicy::RawQuic,
            max_reconnect_attempts: 1,
            reconnect_initial_backoff: Duration::from_millis(10),
            reconnect_max_backoff: Duration::from_millis(10),
            reconnect_deadline: Duration::from_secs(2),
            jitter_percent: 0,
        },
    )
    .unwrap();
    let publish_target = Url::parse(&format!(
        "moqt://localhost:{}/{TENANT}/{BROADCAST}",
        publisher_address.port()
    ))
    .unwrap();
    let relay_publication = tokio::time::timeout(
        NETWORK_TIMEOUT,
        publisher.publish_to_relay(&relay_client, &publish_target),
    )
    .await
    .expect("publisher relay connection timed out")
    .expect("publisher relay connection failed");
    assert_eq!(relay_publication.substrate, BroadcastSubstrate::RawQuic);
    assert_eq!(
        relay_publication.negotiated_protocol,
        MOQT_NEGOTIATED_PROTOCOL
    );
    assert_eq!(topology.coordinated_namespaces(), 1);

    let namespace = MoqNamespace::new(TENANT, BROADCAST).unwrap();
    let subscriber_target = Url::parse(&format!(
        "moqt://localhost:{}/{TENANT}/{BROADCAST}",
        subscriber_address.port()
    ))
    .unwrap();
    let mut config = MoqCatalogSubscriberConfig::new(subscriber_target, namespace);
    config.substrate = substrate;
    config.attempt_timeout = Duration::from_secs(5);
    config.max_reconnect_attempts = 1;
    config.reconnect_initial_backoff = Duration::from_millis(20);
    config.reconnect_max_backoff = Duration::from_millis(20);
    config.reconnect_deadline = Duration::from_secs(5);
    let warm_config = config.clone();
    let credentials: Arc<dyn MoqSubscriberCredentialProvider> = Arc::new(FreshCredentials {
        next: AtomicU64::new(1),
    });
    let catalog_subscriber = MoqCatalogSubscriber::bind(
        "127.0.0.1:0".parse().unwrap(),
        config,
        MoqCatalogSubscriberTlsConfig {
            root_certificates: vec![pki.server_certificate.clone()],
        },
        credentials,
    )
    .unwrap();

    let snapshot = wait_for_live_catalog(&catalog_subscriber).await;
    assert_eq!(snapshot.substrate, Some(admission_substrate));
    assert_eq!(
        snapshot.negotiated_protocol.as_deref(),
        Some(MOQT_NEGOTIATED_PROTOCOL)
    );
    assert!(snapshot
        .peer_identity
        .as_ref()
        .is_some_and(|identity| identity.is_authenticated()));
    assert_eq!(
        snapshot.delivery_mode,
        Some(MoqCatalogDeliveryMode::LiveFallback)
    );
    let latest = snapshot.latest.expect("live catalog update missing");
    assert_eq!(latest.catalog.state(), MsfCatalogState::Live);
    assert_eq!(latest.catalog.tracks().len(), 1);

    // Keep the cold subscription active so the shared relay cache retains a
    // Largest Object. The next subscriber must use Relative Joining FETCH.
    let warm_credentials: Arc<dyn MoqSubscriberCredentialProvider> = Arc::new(FreshCredentials {
        next: AtomicU64::new(10_000),
    });
    let warm_subscriber = MoqCatalogSubscriber::bind(
        "127.0.0.1:0".parse().unwrap(),
        warm_config,
        MoqCatalogSubscriberTlsConfig {
            root_certificates: vec![pki.server_certificate.clone()],
        },
        warm_credentials,
    )
    .unwrap();
    let warm_snapshot = wait_for_live_catalog(&warm_subscriber).await;
    assert_eq!(warm_snapshot.substrate, Some(admission_substrate));
    assert_eq!(
        warm_snapshot.delivery_mode,
        Some(MoqCatalogDeliveryMode::RelativeJoiningFetch)
    );
    assert_eq!(
        warm_snapshot
            .latest
            .expect("retained catalog update missing")
            .catalog
            .state(),
        MsfCatalogState::Live
    );

    tokio::time::timeout(NETWORK_TIMEOUT, warm_subscriber.close())
        .await
        .expect("warm catalog subscriber close timed out")
        .expect("warm catalog subscriber close failed");
    tokio::time::timeout(NETWORK_TIMEOUT, catalog_subscriber.close())
        .await
        .expect("catalog subscriber close timed out")
        .expect("catalog subscriber close failed");
    tokio::time::timeout(NETWORK_TIMEOUT, Arc::clone(&publisher).close())
        .await
        .expect("publisher close timed out")
        .expect("publisher close failed");
    tokio::time::timeout(NETWORK_TIMEOUT, relay_publication.wait())
        .await
        .expect("publisher relay completion timed out")
        .expect("publisher relay completion failed");
    assert_eq!(publisher.lifecycle().state, BroadcastLifecycleState::Closed);
    assert_eq!(topology.coordinated_namespaces(), 0);
    subscriber_runtime
        .drain(Duration::from_secs(5))
        .await
        .unwrap();
    publisher_runtime
        .drain(Duration::from_secs(5))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn publisher_to_managed_relay_to_catalog_subscriber_over_raw_quic() {
    assert_managed_relay_path(MoqRelaySubstratePolicy::RawQuic).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn publisher_to_managed_relay_to_catalog_subscriber_over_webtransport() {
    assert_managed_relay_path(MoqRelaySubstratePolicy::WebTransport).await;
}
