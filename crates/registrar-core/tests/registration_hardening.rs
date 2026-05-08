use chrono::{Duration as ChronoDuration, Utc};
use rvoip_registrar_core::api::ServiceMode;
use rvoip_registrar_core::types::RegistrarConfig;
use rvoip_registrar_core::{
    AddressOfRecord, ContactInfo, ContactReachability, CredentialProvider, ExternalIdentity,
    IdentitySyncService, InMemoryIdentityProvider, RegistrarError, RegistrarService, Transport,
};
use std::sync::Arc;
use tokio::time::{sleep, Duration};

fn aor(uri: &str) -> AddressOfRecord {
    AddressOfRecord::parse(uri).unwrap()
}

fn contact(uri: &str, q_value: f32) -> ContactInfo {
    ContactInfo {
        uri: uri.to_string(),
        instance_id: format!("instance-{uri}"),
        transport: Transport::UDP,
        user_agent: "registrar-core-test".to_string(),
        expires: Utc::now() + ChronoDuration::minutes(5),
        q_value,
        received: None,
        path: Vec::new(),
        methods: vec!["INVITE".to_string(), "MESSAGE".to_string()],
        reg_id: None,
        flow_id: None,
        reachability: ContactReachability::Unknown,
    }
}

async fn service_with_config(config: RegistrarConfig) -> RegistrarService {
    RegistrarService::new_with_mode(ServiceMode::P2P, config)
        .await
        .unwrap()
}

#[tokio::test]
async fn live_lookup_orders_contacts_by_reachability_q_expiry_and_uri() {
    let registrar = RegistrarService::new().await.unwrap();
    let alice = aor("sip:alice@example.com");

    let mut high_q = contact("sip:alice@192.0.2.20:5060", 0.9);
    high_q.reachability = ContactReachability::Unknown;
    let mut reachable_low_q = contact("sip:alice@192.0.2.10:5060", 0.2);
    reachable_low_q.reachability = ContactReachability::Reachable;
    let unreachable = ContactInfo {
        reachability: ContactReachability::Unreachable,
        ..contact("sip:alice@192.0.2.30:5060", 1.0)
    };

    registrar
        .register_aor(&alice, high_q, Some(300))
        .await
        .unwrap();
    registrar
        .register_aor(&alice, reachable_low_q, Some(300))
        .await
        .unwrap();
    registrar
        .register_aor(&alice, unreachable, Some(300))
        .await
        .unwrap();

    let contacts = registrar
        .lookup_live_contacts(&alice, "INVITE")
        .await
        .unwrap();
    assert_eq!(contacts.len(), 2);
    assert_eq!(contacts[0].uri, "sip:alice@192.0.2.10:5060");
    assert_eq!(contacts[1].uri, "sip:alice@192.0.2.20:5060");
}

#[tokio::test]
async fn refresh_and_unregister_update_individual_aor_bindings() {
    let registrar = RegistrarService::new().await.unwrap();
    let alice = aor("sip:alice@example.com");
    let desk_uri = "sip:alice@192.0.2.10:5060";
    let mobile_uri = "sip:alice@192.0.2.11:5060";

    registrar
        .register_aor(&alice, contact(desk_uri, 0.5), Some(300))
        .await
        .unwrap();
    registrar
        .register_aor(&alice, contact(mobile_uri, 0.5), Some(300))
        .await
        .unwrap();
    let before = registrar
        .lookup_aor(&alice)
        .await
        .unwrap()
        .into_iter()
        .find(|binding| binding.uri == desk_uri)
        .unwrap()
        .expires;

    registrar
        .refresh_registration_aor(&alice, desk_uri, 900)
        .await
        .unwrap();
    let after = registrar
        .lookup_aor(&alice)
        .await
        .unwrap()
        .into_iter()
        .find(|binding| binding.uri == desk_uri)
        .unwrap()
        .expires;
    assert!(after > before);

    registrar
        .unregister_contact_aor(&alice, desk_uri)
        .await
        .unwrap();
    let contacts = registrar.lookup_aor(&alice).await.unwrap();
    assert_eq!(contacts.len(), 1);
    assert_eq!(contacts[0].uri, mobile_uri);

    registrar.unregister_all_bindings(&alice).await.unwrap();
    assert!(matches!(
        registrar.lookup_aor(&alice).await,
        Err(RegistrarError::UserNotFound(_))
    ));
}

#[tokio::test]
async fn expired_contacts_are_not_returned_by_live_lookup() {
    let mut config = RegistrarConfig::default();
    config.min_expires = 1;
    config.default_expires = 1;
    let registrar = service_with_config(config).await;
    let alice = aor("sip:alice@example.com");

    registrar
        .register_aor(&alice, contact("sip:alice@192.0.2.10:5060", 1.0), Some(1))
        .await
        .unwrap();

    assert_eq!(
        registrar
            .lookup_live_contacts(&alice, "INVITE")
            .await
            .unwrap()
            .len(),
        1
    );
    sleep(Duration::from_millis(1200)).await;
    assert!(registrar
        .lookup_live_contacts(&alice, "INVITE")
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn aor_domains_and_aliases_are_explicit() {
    let registrar = RegistrarService::new().await.unwrap();
    let alice_example = aor("sip:alice@example.com");
    let alice_other = aor("sip:alice@other.example");

    registrar
        .register_aor(
            &alice_example,
            contact("sip:alice@192.0.2.10:5060", 1.0),
            Some(300),
        )
        .await
        .unwrap();
    registrar
        .register_aor(
            &alice_other,
            contact("sip:alice@192.0.2.20:5060", 1.0),
            Some(300),
        )
        .await
        .unwrap();

    assert_eq!(
        registrar
            .lookup_live_contacts(&alice_example, "INVITE")
            .await
            .unwrap()[0]
            .uri,
        "sip:alice@192.0.2.10:5060"
    );
    assert_eq!(
        registrar
            .lookup_live_contacts(&alice_other, "INVITE")
            .await
            .unwrap()[0]
            .uri,
        "sip:alice@192.0.2.20:5060"
    );

    registrar.add_domain_alias("alias.example", "example.com");
    let alias = aor("sip:alice@alias.example");
    assert_eq!(
        registrar
            .lookup_live_contacts(&alias, "INVITE")
            .await
            .unwrap()[0]
            .uri,
        "sip:alice@192.0.2.10:5060"
    );
}

#[tokio::test]
async fn path_and_outbound_flow_metadata_round_trip() {
    let registrar = RegistrarService::new().await.unwrap();
    let alice = aor("sip:alice@example.com");

    let mut flow_one = contact("sip:alice@192.0.2.10:5060", 1.0);
    flow_one.instance_id = "urn:uuid:device-1".to_string();
    flow_one.reg_id = Some(1);
    flow_one.flow_id = Some("flow-a".to_string());
    flow_one.transport = Transport::TCP;
    flow_one.received = Some("203.0.113.10:62000".to_string());
    flow_one.path = vec![
        "<sip:edge-1.example.com;lr>".to_string(),
        "<sip:proxy.example.com;lr>".to_string(),
    ];

    registrar
        .register_aor(&alice, flow_one, Some(300))
        .await
        .unwrap();

    let stored = registrar
        .lookup_live_contacts(&alice, "INVITE")
        .await
        .unwrap();
    assert_eq!(stored[0].path.len(), 2);
    assert_eq!(stored[0].flow_id.as_deref(), Some("flow-a"));
    assert_eq!(stored[0].reg_id, Some(1));
    assert_eq!(stored[0].transport, Transport::TCP);
    assert_eq!(stored[0].received.as_deref(), Some("203.0.113.10:62000"));

    let mut refreshed_flow = contact("sip:alice@192.0.2.11:5060", 1.0);
    refreshed_flow.instance_id = "urn:uuid:device-1".to_string();
    refreshed_flow.reg_id = Some(1);
    refreshed_flow.flow_id = Some("flow-a-refreshed".to_string());
    registrar
        .register_aor(&alice, refreshed_flow, Some(300))
        .await
        .unwrap();
    let stored = registrar.lookup_aor(&alice).await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].uri, "sip:alice@192.0.2.11:5060");
    assert_eq!(stored[0].flow_id.as_deref(), Some("flow-a-refreshed"));

    let mut second_flow = contact("sip:alice@192.0.2.12:5060", 0.9);
    second_flow.instance_id = "urn:uuid:device-1".to_string();
    second_flow.reg_id = Some(2);
    second_flow.flow_id = Some("flow-b".to_string());
    registrar
        .register_aor(&alice, second_flow, Some(300))
        .await
        .unwrap();
    assert_eq!(registrar.lookup_aor(&alice).await.unwrap().len(), 2);

    let mut no_path_config = RegistrarConfig::default();
    no_path_config.support_path = false;
    let no_path_registrar = service_with_config(no_path_config).await;
    let bob = aor("sip:bob@example.com");
    let mut path_contact = contact("sip:bob@192.0.2.10:5060", 1.0);
    path_contact.path = vec!["<sip:edge.example.com;lr>".to_string()];
    no_path_registrar
        .register_aor(&bob, path_contact, Some(300))
        .await
        .unwrap();
    assert!(no_path_registrar.lookup_aor(&bob).await.unwrap()[0]
        .path
        .is_empty());
}

#[tokio::test]
async fn max_contact_policy_rejects_replaces_or_removes_unavailable() {
    let mut reject_config = RegistrarConfig::default();
    reject_config.max_contacts_per_aor = 1;
    reject_config.remove_existing = false;
    reject_config.remove_unavailable = false;
    let reject_registrar = service_with_config(reject_config).await;
    let alice = aor("sip:alice@example.com");
    reject_registrar
        .register_aor(&alice, contact("sip:alice@192.0.2.10:5060", 0.5), Some(300))
        .await
        .unwrap();
    assert!(matches!(
        reject_registrar
            .register_aor(&alice, contact("sip:alice@192.0.2.11:5060", 0.9), Some(300))
            .await,
        Err(RegistrarError::MaxContactsExceeded { .. })
    ));
    let too_many = aor("sip:too-many@example.com");
    assert!(matches!(
        reject_registrar
            .register_contacts(
                &too_many,
                vec![
                    contact("sip:too-many@192.0.2.10:5060", 0.5),
                    contact("sip:too-many@192.0.2.11:5060", 0.9),
                ],
                Some(300),
            )
            .await,
        Err(RegistrarError::MaxContactsExceeded { .. })
    ));
    assert!(matches!(
        reject_registrar.lookup_aor(&too_many).await,
        Err(RegistrarError::UserNotFound(_))
    ));

    let mut replace_config = RegistrarConfig::default();
    replace_config.max_contacts_per_aor = 1;
    replace_config.remove_existing = true;
    let replace_registrar = service_with_config(replace_config).await;
    let bob = aor("sip:bob@example.com");
    replace_registrar
        .register_aor(&bob, contact("sip:bob@192.0.2.10:5060", 0.1), Some(300))
        .await
        .unwrap();
    replace_registrar
        .register_aor(&bob, contact("sip:bob@192.0.2.11:5060", 1.0), Some(300))
        .await
        .unwrap();
    let contacts = replace_registrar.lookup_aor(&bob).await.unwrap();
    assert_eq!(contacts.len(), 1);
    assert_eq!(contacts[0].uri, "sip:bob@192.0.2.11:5060");

    let mut remove_unavailable_config = RegistrarConfig::default();
    remove_unavailable_config.max_contacts_per_aor = 1;
    remove_unavailable_config.remove_existing = false;
    remove_unavailable_config.remove_unavailable = true;
    let remove_unavailable_registrar = service_with_config(remove_unavailable_config).await;
    let carol = aor("sip:carol@example.com");
    remove_unavailable_registrar
        .register_aor(&carol, contact("sip:carol@192.0.2.10:5060", 1.0), Some(300))
        .await
        .unwrap();
    remove_unavailable_registrar
        .set_contact_reachability(
            &carol,
            "sip:carol@192.0.2.10:5060",
            ContactReachability::Unreachable,
        )
        .await
        .unwrap();
    remove_unavailable_registrar
        .register_aor(&carol, contact("sip:carol@192.0.2.11:5060", 0.5), Some(300))
        .await
        .unwrap();
    let contacts = remove_unavailable_registrar
        .lookup_aor(&carol)
        .await
        .unwrap();
    assert_eq!(contacts.len(), 1);
    assert_eq!(contacts[0].uri, "sip:carol@192.0.2.11:5060");
}

#[tokio::test]
async fn fake_identity_provider_controls_registration_and_live_lookup() {
    let provider = Arc::new(InMemoryIdentityProvider::new());
    let alice = aor("sip:alice@example.com");
    let bob = aor("sip:bob@example.com");

    provider.upsert_identity(ExternalIdentity::enabled(alice.clone(), "idp-alice"));
    provider.set_digest_secret(&alice, "secret");
    assert_eq!(
        CredentialProvider::sip_digest_secret(provider.as_ref(), &alice)
            .await
            .unwrap()
            .as_deref(),
        Some("secret")
    );
    let sync = IdentitySyncService::new(provider.clone());
    assert_eq!(sync.fetch_identities().await.unwrap().len(), 1);

    let registrar = RegistrarService::new()
        .await
        .unwrap()
        .with_identity_provider(provider.clone());
    registrar
        .register_aor(&alice, contact("sip:alice@192.0.2.10:5060", 1.0), Some(300))
        .await
        .unwrap();
    assert!(matches!(
        registrar
            .register_aor(&bob, contact("sip:bob@192.0.2.20:5060", 1.0), Some(300))
            .await,
        Err(RegistrarError::UserNotFound(_))
    ));

    provider.disable_identity(&alice).unwrap();
    assert!(registrar
        .lookup_live_contacts(&alice, "INVITE")
        .await
        .unwrap()
        .is_empty());
    assert!(matches!(
        registrar.lookup_live_contacts(&bob, "INVITE").await,
        Err(RegistrarError::UserNotFound(_))
    ));
}
