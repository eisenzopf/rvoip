use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use rvoip_core_traits::broadcast::{
    BroadcastProtocolFamily, BroadcastPublisher, BroadcastResource,
};
use rvoip_moq::{
    InMemoryMoqGroupIdAllocator, LocOpusPacketizer, MoqBroadcastPublisher, MoqCatalogApplyOutcome,
    MoqCatalogObject, MoqCatalogStateMachine, MoqCatalogSubscriber, MoqCatalogSubscriberConfig,
    MoqCatalogSubscriberLifecycle, MoqCatalogSubscriberTlsConfig, MoqCompatibility,
    MoqEndOfGroupEvidence, MoqGroupIdAllocator, MoqNamespace, MoqProtocolVersion,
    MoqPublisherConfig, MoqRelayConnectionPolicy, MoqRelayPeerIdentity, MoqRelaySubstratePolicy,
    MoqRelayTlsConfig, MoqSanitizedEvent, MoqSanitizedEventKind, MoqSanitizedEventsConfig,
    MoqSubscriberCredential, MoqSubscriberCredentialError, MoqSubscriberCredentialProvider,
    MoqSubscriberCredentialRequest, MsfCatalog, MsfCatalogState, CATALOG_TRACK, EVENTS_TRACK,
    LOC_DRAFT, MOQT_DRAFT, MOQT_NEGOTIATED_PROTOCOL, MSF_DRAFT,
};
use url::Url;

struct TestCredentialProvider;

fn assert_send_sync<T: Send + Sync>() {}

#[async_trait]
impl MoqSubscriberCredentialProvider for TestCredentialProvider {
    async fn issue(
        &self,
        _request: MoqSubscriberCredentialRequest,
    ) -> Result<MoqSubscriberCredential, MoqSubscriberCredentialError> {
        MoqSubscriberCredential::new(b"single-use-test-token".to_vec())
    }
}

#[tokio::test]
async fn application_contract_uses_only_rvoip_owned_models() {
    let namespace = MoqNamespace::new("tenant", "broadcast").unwrap();
    let catalog = MsfCatalog::opus_audio(&namespace, 24_000, Some("en".into()), 0).unwrap();
    catalog.validate().unwrap();
    assert_eq!(catalog.state(), MsfCatalogState::Live);
    let terminal_catalog = MsfCatalog::permanently_completed(1);
    terminal_catalog.validate().unwrap();
    assert_eq!(
        terminal_catalog.state(),
        MsfCatalogState::PermanentlyCompleted
    );

    let subscriber_config = MoqCatalogSubscriberConfig::new(
        Url::parse("moqt://relay.example/tenant/broadcast").unwrap(),
        namespace.clone(),
    );
    subscriber_config.validate().unwrap();
    let credential_request = subscriber_config.credential_request(0).unwrap();
    let credential = TestCredentialProvider
        .issue(credential_request)
        .await
        .unwrap();
    assert_eq!(credential.len(), b"single-use-test-token".len());
    assert!(!format!("{credential:?}").contains("single-use-test-token"));

    let catalog_payload = catalog.to_json_bytes().unwrap();
    let mut subscriber_state = MoqCatalogStateMachine::new(&subscriber_config).unwrap();
    let outcome = subscriber_state
        .apply(MoqCatalogObject {
            namespace: namespace.as_str(),
            track: CATALOG_TRACK,
            group_id: 7,
            subgroup_id: 0,
            object_id: 0,
            first_object: true,
            end_of_group: MoqEndOfGroupEvidence::Signaled,
            extension_header_count: 0,
            declared_payload_len: catalog_payload.len() as u64,
            payload: &catalog_payload,
            received_at: Utc::now(),
        })
        .unwrap();
    assert!(matches!(outcome, MoqCatalogApplyOutcome::Update(_)));
    assert_eq!(subscriber_state.update_count(), 1);
    assert!(!MoqCatalogSubscriberLifecycle::Live.is_terminal());
    assert!(MoqCatalogSubscriberLifecycle::PermanentlyCompleted.is_terminal());
    let subscriber_tls = MoqCatalogSubscriberTlsConfig::default();
    assert!(subscriber_tls.root_certificates.is_empty());
    assert!(!format!("{subscriber_tls:?}").contains("PRIVATE KEY"));
    let _managed_constructor = MoqCatalogSubscriber::bind;
    assert_send_sync::<MoqCatalogSubscriber>();

    let _packetizer = LocOpusPacketizer::new();
    assert_eq!(
        MoqCompatibility::PINNED
            .require(MoqProtocolVersion::PINNED)
            .unwrap(),
        MoqProtocolVersion::PINNED
    );

    let allocator: Arc<dyn MoqGroupIdAllocator> = Arc::new(InMemoryMoqGroupIdAllocator::new());
    let publisher = MoqBroadcastPublisher::new_with_group_id_allocator_and_sanitized_events(
        MoqPublisherConfig {
            tenant_id: "tenant".into(),
            broadcast_id: "broadcast".into(),
            bitrate: 24_000,
            language: Some("en".into()),
            queue_frames: 10,
        },
        MoqSanitizedEventsConfig::new(8, 8).unwrap(),
        allocator,
    )
    .unwrap();
    let protocol = publisher.protocol();
    assert_eq!(protocol.family, BroadcastProtocolFamily::Moqt);
    assert_eq!(protocol.transport_version, MOQT_DRAFT);
    assert_eq!(protocol.media_format_version.as_deref(), Some(MSF_DRAFT));
    assert_eq!(protocol.object_format_version.as_deref(), Some(LOC_DRAFT));
    assert_eq!(MOQT_NEGOTIATED_PROTOCOL, "moqt-19");
    assert_eq!(
        MoqRelayConnectionPolicy::default().substrate,
        MoqRelaySubstratePolicy::RawQuic
    );
    let tls = MoqRelayTlsConfig::default();
    assert!(tls.client_certificate.is_none());
    assert!(tls.client_private_key.is_none());
    let relay_identity = MoqRelayPeerIdentity::VerifiedCertificate {
        leaf_sha256: "aa".repeat(32),
        chain_len: 1,
        total_der_bytes: 512,
    };
    assert!(relay_identity.is_authenticated());
    assert_eq!(
        serde_json::from_value::<MoqRelayPeerIdentity>(
            serde_json::to_value(&relay_identity).unwrap()
        )
        .unwrap(),
        relay_identity
    );
    assert!(matches!(
        publisher.endpoint().resource,
        BroadcastResource::Moqt {
            events_track: Some(ref track),
            ..
        } if track == EVENTS_TRACK
    ));
    let transport_neutral: Arc<dyn BroadcastPublisher> = publisher.clone();
    let event_capability = transport_neutral
        .sanitized_event_capability()
        .expect("event-enabled MOQT publisher must expose its capability");
    assert_eq!(event_capability.queue_capacity, 8);
    assert_eq!(event_capability.history_capacity, 8);
    transport_neutral
        .try_publish_sanitized_event(
            MoqSanitizedEvent::at_unix_millis(MoqSanitizedEventKind::CallConnected, 1_000).unwrap(),
        )
        .unwrap();
    publisher.close().await.unwrap();
}

#[cfg(feature = "relay-runtime")]
#[test]
fn relay_runtime_contract_uses_only_rvoip_owned_models() {
    use rvoip_moq::{
        MoqRelayCertificateBinding, MoqRelayDeploymentMode, MoqRelayListenerKind,
        MoqRelayPublisherBinding, MoqRelayRuntimeLimits, MoqRelayRuntimeSecurity,
        MoqRelayRuntimeTimeouts, MoqRelayServerTlsConfig, MoqRelayTopology, MoqRelayTopologyLimits,
    };

    let security = MoqRelayRuntimeSecurity::PublisherMutualTls {
        bindings: vec![MoqRelayPublisherBinding {
            certificate_sha256: "ab".repeat(32),
            scope: "/tenant/broadcast".to_string(),
        }],
        max_active_sessions_per_certificate: 8,
    };
    assert_eq!(
        security.listener_kind(),
        MoqRelayListenerKind::PublisherMutualTls
    );
    let relay_subscriber = MoqRelayRuntimeSecurity::RelaySubscriberMutualTls {
        bindings: vec![MoqRelayCertificateBinding {
            certificate_sha256: "cd".repeat(32),
            scope: "/tenant/broadcast".to_string(),
        }],
        max_active_sessions_per_certificate: 4,
    };
    assert_eq!(
        relay_subscriber.listener_kind(),
        MoqRelayListenerKind::RelaySubscriberMutualTls
    );
    assert_eq!(
        MoqRelayDeploymentMode::default(),
        MoqRelayDeploymentMode::Embedded
    );
    assert!(MoqRelayRuntimeLimits::default().max_active_sessions > 0);
    assert!(!MoqRelayRuntimeTimeouts::default().drop_cleanup.is_zero());
    assert!(MoqRelayServerTlsConfig::default()
        .server_certificates
        .is_empty());
    let topology = MoqRelayTopology::with_limits(
        Url::parse("moqt://publisher.internal:443").unwrap(),
        None,
        MoqRelayTopologyLimits {
            max_namespaces: 64,
            max_namespace_subscriptions: 8,
            namespace_update_queue_capacity: 4,
        },
    )
    .unwrap();
    assert_eq!(topology.coordinated_namespaces(), 0);
    assert_eq!(topology.namespace_subscriptions(), 0);
    assert!(!format!("{topology:?}").contains("publisher.internal"));
}
