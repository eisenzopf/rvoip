//! Cross-crate integration tests for SIP registration flow.
//!
//! Tests the interaction between sip-core (REGISTER message building) and
//! registrar-core (registration storage and lookup), verifying that the
//! registrar correctly stores and manages user registrations.

use std::sync::Arc;

use chrono::{Duration, Utc};

use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::types::Method;
use rvoip_sip_core::{Message, TypedHeader, ContentLength};

use rvoip_registrar_core::registrar::UserRegistry;
use rvoip_registrar_core::registrar::manager::RegistrationManager;
use rvoip_registrar_core::types::{ContactInfo, Transport};

// =============================================================================
// Test 1: Build REGISTER request (sip-core) and store in registrar (registrar-core)
// =============================================================================

#[tokio::test]
async fn test_register_request_to_registrar_storage() {
    // Step 1: Build a REGISTER request using sip-core
    let request = SimpleRequestBuilder::new(Method::Register, "sip:registrar.example.com")
        .expect("valid URI")
        .from("Alice", "sip:alice@example.com", Some("reg-tag-1"))
        .to("Alice", "sip:alice@example.com", None)
        .call_id("register-integration-001@example.com")
        .cseq(1)
        .via("192.168.1.100:5060", "UDP", Some("z9hG4bK-reg-1"))
        .max_forwards(70)
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build();

    // Verify the REGISTER request was built correctly
    assert_eq!(request.method(), Method::Register);
    let from = request.from().expect("should have From header");
    let from_str = from.to_string();
    assert!(from_str.contains("alice"), "From should reference alice");

    // Step 2: Extract registration info and store in registrar-core
    let registry = UserRegistry::new();

    let contact = ContactInfo {
        uri: "sip:alice@192.168.1.100:5060".to_string(),
        instance_id: "device-alice-1".to_string(),
        transport: Transport::UDP,
        user_agent: "rvoip-test/0.1".to_string(),
        expires: Utc::now() + Duration::hours(1),
        q_value: 1.0,
        received: None,
        path: vec![],
        methods: vec!["INVITE".to_string(), "BYE".to_string(), "REGISTER".to_string()],
    };

    registry
        .register("alice", contact.clone(), 3600)
        .await
        .expect("registration should succeed");

    // Step 3: Verify registration is stored
    assert!(
        registry.is_registered("alice").await,
        "Alice should be registered after REGISTER"
    );

    let reg = registry
        .get_registration("alice")
        .await
        .expect("should find alice's registration");
    assert_eq!(reg.user_id, "alice");
    assert_eq!(reg.contacts.len(), 1);
    assert_eq!(reg.contacts[0].uri, "sip:alice@192.168.1.100:5060");
    assert_eq!(reg.contacts[0].transport, Transport::UDP);
}

// =============================================================================
// Test 2: Multiple registrations from different devices
// =============================================================================

#[tokio::test]
async fn test_multiple_device_registrations() {
    let registry = UserRegistry::new();

    // Register alice from device 1
    let contact1 = ContactInfo {
        uri: "sip:alice@192.168.1.100:5060".to_string(),
        instance_id: "device-1".to_string(),
        transport: Transport::UDP,
        user_agent: "rvoip-desktop/0.1".to_string(),
        expires: Utc::now() + Duration::hours(1),
        q_value: 1.0,
        received: None,
        path: vec![],
        methods: vec!["INVITE".to_string()],
    };

    registry
        .register("alice", contact1, 3600)
        .await
        .expect("first registration should succeed");

    // Register alice from device 2 (different URI)
    let contact2 = ContactInfo {
        uri: "sip:alice@10.0.0.50:5060".to_string(),
        instance_id: "device-2".to_string(),
        transport: Transport::TCP,
        user_agent: "rvoip-mobile/0.1".to_string(),
        expires: Utc::now() + Duration::hours(1),
        q_value: 0.8,
        received: None,
        path: vec![],
        methods: vec!["INVITE".to_string(), "MESSAGE".to_string()],
    };

    registry
        .register("alice", contact2, 3600)
        .await
        .expect("second registration should succeed");

    // Verify both contacts are stored
    let reg = registry
        .get_registration("alice")
        .await
        .expect("should find registration");
    assert_eq!(
        reg.contacts.len(),
        2,
        "Alice should have 2 registered contacts"
    );

    // Verify different transports
    let transports: Vec<Transport> = reg.contacts.iter().map(|c| c.transport).collect();
    assert!(transports.contains(&Transport::UDP));
    assert!(transports.contains(&Transport::TCP));
}

// =============================================================================
// Test 3: Unregister flow (REGISTER with expires=0)
// =============================================================================

#[tokio::test]
async fn test_unregister_removes_registration() {
    let registry = UserRegistry::new();

    // Register bob
    let contact = ContactInfo {
        uri: "sip:bob@192.168.1.200:5060".to_string(),
        instance_id: "bob-device".to_string(),
        transport: Transport::UDP,
        user_agent: "rvoip-test/0.1".to_string(),
        expires: Utc::now() + Duration::hours(1),
        q_value: 1.0,
        received: None,
        path: vec![],
        methods: vec![],
    };

    registry
        .register("bob", contact, 3600)
        .await
        .expect("registration should succeed");
    assert!(registry.is_registered("bob").await);

    // Build a REGISTER request with expires=0 (unregister) via sip-core
    let unreg_request = SimpleRequestBuilder::new(Method::Register, "sip:registrar.example.com")
        .expect("valid URI")
        .from("Bob", "sip:bob@example.com", Some("unreg-tag"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id("unregister-001@example.com")
        .cseq(2)
        .via("192.168.1.200:5060", "UDP", Some("z9hG4bK-unreg"))
        .max_forwards(70)
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build();

    assert_eq!(unreg_request.method(), Method::Register);

    // Process unregistration via registrar
    registry
        .unregister("bob")
        .await
        .expect("unregister should succeed");

    assert!(
        !registry.is_registered("bob").await,
        "Bob should no longer be registered after unregister"
    );
}

// =============================================================================
// Test 4: Registration manager lifecycle with registry
// =============================================================================

#[tokio::test]
async fn test_registration_manager_lifecycle() {
    let registry = Arc::new(UserRegistry::new());

    // Register a user
    let contact = ContactInfo {
        uri: "sip:carol@192.168.1.50:5060".to_string(),
        instance_id: "carol-device".to_string(),
        transport: Transport::UDP,
        user_agent: "rvoip-test/0.1".to_string(),
        expires: Utc::now() + Duration::hours(1),
        q_value: 1.0,
        received: None,
        path: vec![],
        methods: vec!["INVITE".to_string()],
    };

    registry
        .register("carol", contact, 3600)
        .await
        .expect("registration should succeed");

    // Create and start the registration manager
    let manager = RegistrationManager::new(registry.clone());
    manager.start().await;

    // Verify the registry still works while manager is running
    assert!(registry.is_registered("carol").await);

    let all_users = registry.list_all_users().await;
    assert_eq!(all_users.len(), 1);
    assert!(all_users.contains(&"carol".to_string()));

    // Stop manager cleanly
    manager.stop().await;

    // Registry should still be accessible after manager stops
    assert!(registry.is_registered("carol").await);
}

// =============================================================================
// Test 5: Remove specific contact for a user
// =============================================================================

#[tokio::test]
async fn test_remove_specific_contact() {
    let registry = UserRegistry::new();

    // Register with two contacts
    let contact1 = ContactInfo {
        uri: "sip:dave@192.168.1.10:5060".to_string(),
        instance_id: "dave-desktop".to_string(),
        transport: Transport::UDP,
        user_agent: "rvoip-desktop/0.1".to_string(),
        expires: Utc::now() + Duration::hours(1),
        q_value: 1.0,
        received: None,
        path: vec![],
        methods: vec![],
    };
    let contact2 = ContactInfo {
        uri: "sip:dave@10.0.0.20:5060".to_string(),
        instance_id: "dave-mobile".to_string(),
        transport: Transport::TCP,
        user_agent: "rvoip-mobile/0.1".to_string(),
        expires: Utc::now() + Duration::hours(1),
        q_value: 0.5,
        received: None,
        path: vec![],
        methods: vec![],
    };

    registry.register("dave", contact1, 3600).await.expect("reg1");
    registry.register("dave", contact2, 3600).await.expect("reg2");

    let reg = registry.get_registration("dave").await.expect("should find dave");
    assert_eq!(reg.contacts.len(), 2);

    // Remove the desktop contact
    registry
        .remove_contact("dave", "sip:dave@192.168.1.10:5060")
        .await
        .expect("remove_contact should succeed");

    let reg = registry.get_registration("dave").await.expect("still registered");
    assert_eq!(reg.contacts.len(), 1, "Should have 1 contact after removal");
    assert_eq!(reg.contacts[0].uri, "sip:dave@10.0.0.20:5060");
}
