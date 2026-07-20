#![cfg(feature = "relay-runtime")]

use std::net::{SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use rvoip_auth_core::{BearerAuthError, BearerValidator, ValidatedBearer};
use rvoip_core_traits::broadcast::{
    BroadcastLifecycleState, BroadcastPublisher, BroadcastSubstrate,
};
use rvoip_core_traits::identity::IdentityAssurance;
use rvoip_core_traits::ids::StreamId;
use rvoip_core_traits::stream::{MediaFrame, StreamKind};
use rvoip_core_traits::{AuthenticatedPrincipal, AuthenticationMethod};
use rvoip_moq::{
    BoundedMemoryMoqReplayStore, BoundedMemoryMoqSessionLeaseStore, MoqAction, MoqAudioSubscriber,
    MoqAudioSubscriberConfig, MoqAudioSubscriberLifecycle, MoqAuthorizer, MoqBroadcastPublisher,
    MoqCatalogDeliveryMode, MoqCatalogSubscriber, MoqCatalogSubscriberConfig,
    MoqCatalogSubscriberLifecycle, MoqCatalogSubscriberTlsConfig, MoqNamespace, MoqPeerIdentity,
    MoqPublisherConfig, MoqRelayAdmissionConfig, MoqRelayAdmissionSubstrate, MoqRelayClient,
    MoqRelayConnectionPolicy, MoqRelayDeploymentMode, MoqRelayPublisherBinding, MoqRelayRuntime,
    MoqRelayRuntimeConfig, MoqRelayRuntimeLimits, MoqRelayRuntimeSecurity, MoqRelayRuntimeTimeouts,
    MoqRelayServerTlsConfig, MoqRelaySubstratePolicy, MoqRelayTlsConfig, MoqRelayTopology,
    MoqRelayTopologyLimits, MoqRelayUpstreamHealth, MoqRelayUpstreamReconnectMode,
    MoqRelayUpstreamRoute, MoqRelayUpstreamRouteError, MoqRelayUpstreamRoutes, MoqResource,
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
    relay_certificate: PathBuf,
    relay_private_key: PathBuf,
    relay_fingerprint: String,
}

impl TestPki {
    fn new() -> Self {
        static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);
        let server = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let publisher = rcgen::generate_simple_self_signed(vec!["publisher.test".into()]).unwrap();
        let relay = rcgen::generate_simple_self_signed(vec!["relay.test".into()]).unwrap();
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
        let relay_certificate = directory.join("relay.pem");
        let relay_private_key = directory.join("relay.key");
        std::fs::write(&server_certificate, server.cert.pem()).unwrap();
        std::fs::write(&server_private_key, server.signing_key.serialize_pem()).unwrap();
        std::fs::write(&publisher_certificate, publisher.cert.pem()).unwrap();
        std::fs::write(
            &publisher_private_key,
            publisher.signing_key.serialize_pem(),
        )
        .unwrap();
        std::fs::write(&relay_certificate, relay.cert.pem()).unwrap();
        std::fs::write(&relay_private_key, relay.signing_key.serialize_pem()).unwrap();
        Self {
            directory,
            server_certificate,
            server_private_key,
            publisher_certificate,
            publisher_private_key,
            publisher_fingerprint: lower_hex(&Sha256::digest(publisher.cert.der().as_ref())),
            relay_certificate,
            relay_private_key,
            relay_fingerprint: lower_hex(&Sha256::digest(relay.cert.der().as_ref())),
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

    fn relay_subscriber_server_tls(&self) -> MoqRelayServerTlsConfig {
        MoqRelayServerTlsConfig {
            server_certificates: vec![self.server_certificate.clone()],
            server_private_keys: vec![self.server_private_key.clone()],
            server_root_certificates: vec![self.server_certificate.clone()],
            publisher_client_ca_certificates: vec![self.relay_certificate.clone()],
            ..MoqRelayServerTlsConfig::default()
        }
    }

    fn downstream_server_tls(&self) -> MoqRelayServerTlsConfig {
        MoqRelayServerTlsConfig {
            server_certificates: vec![self.server_certificate.clone()],
            server_private_keys: vec![self.server_private_key.clone()],
            server_root_certificates: vec![self.server_certificate.clone()],
            outbound_client_certificate: Some(self.relay_certificate.clone()),
            outbound_client_private_key: Some(self.relay_private_key.clone()),
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

    fn relay_client_tls(&self) -> MoqRelayTlsConfig {
        MoqRelayTlsConfig {
            root_certificates: vec![self.server_certificate.clone()],
            client_certificate: Some(self.relay_certificate.clone()),
            client_private_key: Some(self.relay_private_key.clone()),
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

async fn wait_for_live_audio_catalog(
    subscriber: &MoqAudioSubscriber,
) -> rvoip_moq::MoqCatalogSubscriptionSnapshot {
    let mut updates = subscriber.updates();
    tokio::time::timeout(NETWORK_TIMEOUT, async {
        loop {
            let snapshot = updates.borrow_and_update().clone();
            if snapshot.lifecycle == MoqCatalogSubscriberLifecycle::Live {
                return snapshot;
            }
            assert!(!snapshot.lifecycle.is_terminal(), "{snapshot:?}");
            updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "managed audio catalog timed out: {:?}",
            subscriber.snapshot()
        )
    })
}

async fn wait_for_live_audio_track(subscriber: &MoqAudioSubscriber) {
    let mut updates = subscriber.audio_updates();
    tokio::time::timeout(NETWORK_TIMEOUT, async {
        loop {
            let snapshot = updates.borrow_and_update().clone();
            if snapshot.lifecycle == MoqAudioSubscriberLifecycle::Live {
                return;
            }
            assert!(!snapshot.lifecycle.is_terminal(), "{snapshot:?}");
            updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "managed audio track timed out: {:?}",
            subscriber.audio_snapshot()
        )
    });
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
    let mut config = MoqAudioSubscriberConfig::new(subscriber_target, namespace);
    config.catalog.substrate = substrate;
    config.catalog.attempt_timeout = Duration::from_secs(5);
    config.catalog.max_reconnect_attempts = 1;
    config.catalog.reconnect_initial_backoff = Duration::from_millis(20);
    config.catalog.reconnect_max_backoff = Duration::from_millis(20);
    config.catalog.reconnect_deadline = Duration::from_secs(5);
    let warm_config = config.catalog.clone();
    let credentials: Arc<dyn MoqSubscriberCredentialProvider> = Arc::new(FreshCredentials {
        next: AtomicU64::new(1),
    });
    let catalog_subscriber = MoqAudioSubscriber::bind(
        "127.0.0.1:0".parse().unwrap(),
        config,
        MoqCatalogSubscriberTlsConfig {
            root_certificates: vec![pki.server_certificate.clone()],
        },
        credentials,
    )
    .unwrap();
    let mut audio_objects = catalog_subscriber.audio_objects();

    let snapshot = wait_for_live_audio_catalog(&catalog_subscriber).await;
    wait_for_live_audio_track(&catalog_subscriber).await;
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

    publisher
        .frames_out()
        .send(MediaFrame {
            stream_id: StreamId::new(),
            kind: StreamKind::Audio,
            payload: Bytes::from_static(&[0x78, 0x00]),
            timestamp_rtp: 960,
            captured_at: Utc::now(),
            payload_type: Some(111),
        })
        .await
        .unwrap();
    let received = tokio::time::timeout(NETWORK_TIMEOUT, audio_objects.recv())
        .await
        .expect("managed audio object timed out")
        .expect("managed audio receiver closed");
    assert_eq!(received.object.object_id, 0);
    assert_eq!(received.object.timestamp, 960);
    assert_eq!(received.object.timescale, 48_000);
    assert_eq!(received.object.payload, Bytes::from_static(&[0x78, 0x00]));
    assert_eq!(catalog_subscriber.audio_snapshot().received_objects, 1);

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
    tokio::time::timeout(NETWORK_TIMEOUT, Arc::clone(&publisher).close())
        .await
        .expect("publisher close timed out")
        .expect("publisher close failed");
    tokio::time::timeout(NETWORK_TIMEOUT, catalog_subscriber.wait())
        .await
        .expect("audio subscriber terminal catalog timed out")
        .expect("audio subscriber terminal catalog failed");
    assert_eq!(
        catalog_subscriber.snapshot().lifecycle,
        MoqCatalogSubscriberLifecycle::PermanentlyCompleted
    );
    assert_eq!(
        catalog_subscriber.audio_snapshot().lifecycle,
        MoqAudioSubscriberLifecycle::Closed
    );
    tokio::time::timeout(NETWORK_TIMEOUT, catalog_subscriber.close())
        .await
        .expect("audio subscriber close timed out")
        .expect("audio subscriber close failed");
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

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn external_mtls_route_crosses_independent_relay_topologies_and_drains() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
    let pki = TestPki::new();
    let origin_publisher_address = unused_udp_address();
    let origin_relay_address = unused_udp_address();
    let restarted_origin_relay_address = unused_udp_address();
    let downstream_address = unused_udp_address();
    assert_ne!(origin_publisher_address, origin_relay_address);
    assert_ne!(origin_relay_address, downstream_address);
    assert_ne!(origin_relay_address, restarted_origin_relay_address);

    let origin_topology = MoqRelayTopology::new(
        endpoint(origin_publisher_address),
        Some(origin_publisher_address),
        8,
    )
    .unwrap();
    let downstream_topology = MoqRelayTopology::with_limits_and_upstream_routes(
        endpoint(downstream_address),
        Some(downstream_address),
        MoqRelayTopologyLimits {
            max_namespaces: 8,
            max_namespace_subscriptions: 8,
            namespace_update_queue_capacity: 8,
        },
        MoqRelayUpstreamRoutes::new(std::iter::empty(), 2).unwrap(),
    )
    .unwrap();

    let origin_publisher_runtime = MoqRelayRuntime::start_with_topology(
        runtime_config(
            origin_publisher_address,
            pki.publisher_server_tls(),
            MoqRelayRuntimeSecurity::PublisherMutualTls {
                bindings: vec![MoqRelayPublisherBinding {
                    certificate_sha256: pki.publisher_fingerprint.clone(),
                    scope: format!("/{TENANT}/{BROADCAST}"),
                }],
                max_active_sessions_per_certificate: 4,
            },
        ),
        origin_topology.clone(),
    )
    .unwrap();
    let origin_relay_runtime = MoqRelayRuntime::start_with_topology(
        runtime_config(
            origin_relay_address,
            pki.relay_subscriber_server_tls(),
            MoqRelayRuntimeSecurity::RelaySubscriberMutualTls {
                bindings: vec![rvoip_moq::MoqRelayCertificateBinding {
                    certificate_sha256: pki.relay_fingerprint.clone(),
                    scope: format!("/{TENANT}/{BROADCAST}"),
                }],
                max_active_sessions_per_certificate: 4,
            },
        ),
        origin_topology.clone(),
    )
    .unwrap();
    let mut downstream_config = runtime_config(
        downstream_address,
        pki.downstream_server_tls(),
        MoqRelayRuntimeSecurity::SubscriberRawQuic {
            admission: subscriber_admission(MoqRelayAdmissionSubstrate::RawQuic),
        },
    );
    downstream_config.deployment = MoqRelayDeploymentMode::Standalone;
    let downstream_runtime =
        MoqRelayRuntime::start_with_topology(downstream_config, downstream_topology.clone())
            .unwrap();

    // Install after the listener is ready. This proves the standalone control
    // plane does not need to restart the relay to add a newly-created broadcast.
    let namespace = MoqNamespace::new(TENANT, BROADCAST).unwrap();
    let route_registration = downstream_runtime
        .register_upstream_route(
            MoqRelayUpstreamRoute::new(
                namespace.clone(),
                endpoint(origin_relay_address),
                Some(origin_relay_address),
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(downstream_topology.upstream_routes(), 1);

    // The exact same certificate used by the downstream RemoteManager is
    // subscribe-only at the origin. It must not be usable to publish.
    let denied_publisher = MoqBroadcastPublisher::new(MoqPublisherConfig {
        tenant_id: TENANT.to_owned(),
        broadcast_id: BROADCAST.to_owned(),
        bitrate: 32_000,
        language: None,
        queue_frames: 10,
    })
    .unwrap();
    let relay_identity_client = MoqRelayClient::bind_with_policy(
        "127.0.0.1:0".parse().unwrap(),
        pki.relay_client_tls(),
        MoqRelayConnectionPolicy {
            attempt_timeout: Duration::from_secs(3),
            publish_namespace_acceptance_timeout: Duration::from_secs(2),
            substrate: MoqRelaySubstratePolicy::RawQuic,
            max_reconnect_attempts: 1,
            reconnect_initial_backoff: Duration::from_millis(10),
            reconnect_max_backoff: Duration::from_millis(10),
            reconnect_deadline: Duration::from_secs(1),
            jitter_percent: 0,
        },
    )
    .unwrap();
    let origin_relay_target = Url::parse(&format!(
        "moqt://localhost:{}/{TENANT}/{BROADCAST}",
        origin_relay_address.port()
    ))
    .unwrap();
    let denied = tokio::time::timeout(
        NETWORK_TIMEOUT,
        denied_publisher.publish_to_relay(&relay_identity_client, &origin_relay_target),
    )
    .await
    .expect("relay-identity publish denial timed out");
    assert!(denied.is_err());
    assert_eq!(origin_topology.coordinated_namespaces(), 0);
    let _ = tokio::time::timeout(NETWORK_TIMEOUT, denied_publisher.close()).await;

    let publisher = MoqBroadcastPublisher::new(MoqPublisherConfig {
        tenant_id: TENANT.to_owned(),
        broadcast_id: BROADCAST.to_owned(),
        bitrate: 32_000,
        language: Some("en".to_owned()),
        queue_frames: 10,
    })
    .unwrap();
    let publisher_client = MoqRelayClient::bind_with_policy(
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
    let origin_publish_target = Url::parse(&format!(
        "moqt://localhost:{}/{TENANT}/{BROADCAST}",
        origin_publisher_address.port()
    ))
    .unwrap();
    let publication = tokio::time::timeout(
        NETWORK_TIMEOUT,
        publisher.publish_to_relay(&publisher_client, &origin_publish_target),
    )
    .await
    .expect("origin publisher connection timed out")
    .expect("origin publisher connection failed");

    let subscriber_target = Url::parse(&format!(
        "moqt://localhost:{}/{TENANT}/{BROADCAST}",
        downstream_address.port()
    ))
    .unwrap();
    let mut subscriber_config = MoqCatalogSubscriberConfig::new(subscriber_target, namespace);
    subscriber_config.substrate = MoqRelaySubstratePolicy::RawQuic;
    subscriber_config.attempt_timeout = Duration::from_secs(5);
    subscriber_config.max_reconnect_attempts = 1;
    subscriber_config.reconnect_initial_backoff = Duration::from_millis(20);
    subscriber_config.reconnect_max_backoff = Duration::from_millis(20);
    subscriber_config.reconnect_deadline = Duration::from_secs(5);
    let reconnect_subscriber_config = subscriber_config.clone();
    let catalog_subscriber = MoqCatalogSubscriber::bind(
        "127.0.0.1:0".parse().unwrap(),
        subscriber_config,
        MoqCatalogSubscriberTlsConfig {
            root_certificates: vec![pki.server_certificate.clone()],
        },
        Arc::new(FreshCredentials {
            next: AtomicU64::new(50_000),
        }),
    )
    .unwrap();
    let catalog = wait_for_live_catalog(&catalog_subscriber).await;
    assert_eq!(catalog.substrate, Some(BroadcastSubstrate::RawQuic));
    assert_eq!(
        catalog
            .latest
            .expect("external route catalog missing")
            .catalog
            .state(),
        MsfCatalogState::Live
    );

    // The origin listener trusts only the relay certificate. A successful
    // object traversal therefore proves RemoteManager used the runtime's
    // configured verified roots plus outbound client certificate/key.
    let active = downstream_runtime.snapshot().await;
    assert_eq!(active.configured_upstream_routes, 1);
    assert_eq!(active.max_upstream_routes, 2);
    assert!(active.upstream_route_resolutions >= 1);
    assert_eq!(active.upstream_route_misses, 0);
    assert_eq!(active.upstream_health, MoqRelayUpstreamHealth::Connected);
    assert_eq!(
        active.upstream_reconnect_mode,
        MoqRelayUpstreamReconnectMode::OnDemand
    );
    assert_eq!(active.cached_upstream_connections, 1);
    assert!(active.retained_upstream_connections >= 1);
    assert!(active.retained_upstream_tracks >= 1);

    catalog_subscriber.close().await.unwrap();

    // Lose the upstream listener while the publisher remains live, then
    // replace the exact route with a new listener generation. The next
    // downstream subscribe must leave the closed cached session behind and
    // reconnect on demand with the same mTLS identity.
    origin_relay_runtime
        .drain(Duration::from_secs(5))
        .await
        .unwrap();
    drop(route_registration);
    let origin_relay_runtime = MoqRelayRuntime::start_with_topology(
        runtime_config(
            restarted_origin_relay_address,
            pki.relay_subscriber_server_tls(),
            MoqRelayRuntimeSecurity::RelaySubscriberMutualTls {
                bindings: vec![rvoip_moq::MoqRelayCertificateBinding {
                    certificate_sha256: pki.relay_fingerprint.clone(),
                    scope: format!("/{TENANT}/{BROADCAST}"),
                }],
                max_active_sessions_per_certificate: 4,
            },
        ),
        origin_topology.clone(),
    )
    .unwrap();
    let route_registration = downstream_runtime
        .register_upstream_route(
            MoqRelayUpstreamRoute::new(
                MoqNamespace::new(TENANT, BROADCAST).unwrap(),
                endpoint(restarted_origin_relay_address),
                Some(restarted_origin_relay_address),
            )
            .unwrap(),
        )
        .unwrap();
    let reconnect_subscriber = MoqCatalogSubscriber::bind(
        "127.0.0.1:0".parse().unwrap(),
        reconnect_subscriber_config,
        MoqCatalogSubscriberTlsConfig {
            root_certificates: vec![pki.server_certificate.clone()],
        },
        Arc::new(FreshCredentials {
            next: AtomicU64::new(60_000),
        }),
    )
    .unwrap();
    let reconnected = wait_for_live_catalog(&reconnect_subscriber).await;
    assert_eq!(
        reconnected
            .latest
            .expect("reconnected external route catalog missing")
            .catalog
            .state(),
        MsfCatalogState::Live
    );
    reconnect_subscriber.close().await.unwrap();
    assert!(
        downstream_runtime
            .snapshot()
            .await
            .upstream_route_resolutions
            >= 2
    );

    Arc::clone(&publisher).close().await.unwrap();
    publication.wait().await.unwrap();
    assert_eq!(origin_topology.coordinated_namespaces(), 0);

    // Drain owns dynamic-route shutdown even while the caller still holds the
    // RAII lease, and the closed runtime rejects a new generation.
    downstream_runtime
        .drain(Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(downstream_topology.upstream_routes(), 0);
    assert!(matches!(
        downstream_runtime.register_upstream_route(
            MoqRelayUpstreamRoute::new(
                MoqNamespace::new(TENANT, BROADCAST).unwrap(),
                endpoint(restarted_origin_relay_address),
                Some(restarted_origin_relay_address),
            )
            .unwrap()
        ),
        Err(MoqRelayUpstreamRouteError::Draining)
    ));
    let stopped = downstream_runtime.snapshot().await;
    assert_eq!(stopped.upstream_health, MoqRelayUpstreamHealth::Stopped);
    assert_eq!(stopped.cached_upstream_connections, 0);
    assert_eq!(stopped.retained_upstream_connections, 0);
    assert_eq!(stopped.retained_upstream_tracks, 0);
    assert_eq!(stopped.supervised_upstream_tasks, 0);
    drop(route_registration);

    origin_relay_runtime
        .drain(Duration::from_secs(5))
        .await
        .unwrap();
    origin_publisher_runtime
        .drain(Duration::from_secs(5))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn publisher_to_managed_relay_to_audio_subscriber_over_raw_quic() {
    assert_managed_relay_path(MoqRelaySubstratePolicy::RawQuic).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn publisher_to_managed_relay_to_audio_subscriber_over_webtransport() {
    assert_managed_relay_path(MoqRelaySubstratePolicy::WebTransport).await;
}
