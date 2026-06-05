//! Integration tests for `VconBuilder`, `MemoryVconStore`, and
//! optional JWS signing — the v0 vCon surface (plan C1).

use chrono::Utc;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey};
use rvoip_vcon::{
    builder::verify_jws, sign_jws, DialogKind, MemoryVconStore, Party, Vcon, VconBuilder,
    VconStore, VconStoreError,
};

#[test]
fn builder_constructs_minimal_vcon() {
    let now = Utc::now();
    let vcon = VconBuilder::new()
        .subject("Quarterly review with Acme")
        .with_party(Party {
            name: Some("Alice (agent)".into()),
            role: Some("agent".into()),
            ..Default::default()
        })
        .with_party(Party {
            tel: Some("tel:+15551234".into()),
            name: Some("Bob".into()),
            role: Some("customer".into()),
            ..Default::default()
        })
        .recording(now, 120_000, vec![0, 1], "audio/opus")
        .build();

    assert_eq!(vcon.vcon, "0.0.1");
    assert_eq!(vcon.subject.as_deref(), Some("Quarterly review with Acme"));
    assert_eq!(vcon.parties.len(), 2);
    assert_eq!(vcon.parties[1].tel.as_deref(), Some("tel:+15551234"));
    assert_eq!(vcon.dialog.len(), 1);
    assert_eq!(vcon.dialog[0].kind, DialogKind::Recording);
    assert_eq!(vcon.dialog[0].duration_ms, Some(120_000));
    assert_eq!(vcon.dialog[0].mediatype.as_deref(), Some("audio/opus"));
}

#[test]
fn builder_supports_text_dialogs() {
    let now = Utc::now();
    let mut builder = VconBuilder::new();
    let alice = builder.party(Party {
        name: Some("Alice".into()),
        ..Default::default()
    });
    let vcon = builder.text(now, alice, "hello from the test").build();

    assert_eq!(vcon.dialog.len(), 1);
    assert_eq!(vcon.dialog[0].kind, DialogKind::Text);
    assert_eq!(vcon.dialog[0].body.as_deref(), Some("hello from the test"));
    assert_eq!(vcon.dialog[0].parties, vec![alice]);
}

#[test]
fn vcon_round_trips_through_json() {
    let vcon = VconBuilder::new()
        .subject("Round-trip")
        .with_party(Party {
            name: Some("Charlie".into()),
            ..Default::default()
        })
        .recording(Utc::now(), 5_000, vec![0], "audio/PCMA")
        .build();

    let json = serde_json::to_string(&vcon).expect("serialize");
    let restored: Vcon = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(restored.uuid, vcon.uuid);
    assert_eq!(restored.parties.len(), 1);
    assert_eq!(restored.dialog.len(), 1);
    assert_eq!(restored.dialog[0].mediatype.as_deref(), Some("audio/PCMA"));
}

#[tokio::test]
async fn memory_store_put_get_round_trip() {
    let store = MemoryVconStore::new();
    let vcon = VconBuilder::new()
        .subject("Memory store smoke")
        .with_party(Party::default())
        .build();
    let uuid = vcon.uuid;
    store.put(vcon.clone()).await.expect("put");
    assert_eq!(store.len().await, Some(1));

    let fetched = store.get(&uuid).await.expect("get");
    assert_eq!(fetched.uuid, uuid);
    assert_eq!(fetched.subject.as_deref(), Some("Memory store smoke"));
}

#[tokio::test]
async fn memory_store_refuses_silent_overwrite() {
    let store = MemoryVconStore::new();
    let vcon = VconBuilder::new().with_uuid(uuid::Uuid::new_v4()).build();
    let uuid = vcon.uuid;
    store.put(vcon.clone()).await.expect("first put");

    // Second put with the same uuid → error (preserves immutability
    // unless caller explicitly opts into overwrite).
    let err = store
        .put(vcon.clone())
        .await
        .expect_err("second put must error");
    assert!(matches!(err, VconStoreError::Backend(_)));

    // Overwrite variant succeeds.
    store.put_overwrite(vcon).await.expect("overwrite");
    assert!(store.get(&uuid).await.is_ok());
}

#[tokio::test]
async fn memory_store_get_unknown_uuid_yields_not_found() {
    let store = MemoryVconStore::new();
    let result = store.get(&uuid::Uuid::new_v4()).await;
    assert!(matches!(result, Err(VconStoreError::NotFound(_))));
}

#[tokio::test]
async fn memory_store_delete_is_idempotent() {
    let store = MemoryVconStore::new();
    let vcon = VconBuilder::new().build();
    let uuid = vcon.uuid;
    store.put(vcon).await.unwrap();
    store.delete(&uuid).await.unwrap();
    // Second delete: still Ok.
    store.delete(&uuid).await.unwrap();
    // Get: NotFound.
    assert!(matches!(
        store.get(&uuid).await,
        Err(VconStoreError::NotFound(_))
    ));
}

#[test]
fn jws_sign_and_verify_round_trip() {
    let secret = b"vcon-test-secret";
    let vcon = VconBuilder::new()
        .subject("JWS round-trip")
        .with_party(Party {
            name: Some("Signer".into()),
            ..Default::default()
        })
        .build();
    let original_uuid = vcon.uuid;

    let signed =
        sign_jws(&vcon, &EncodingKey::from_secret(secret), Algorithm::HS256).expect("sign");
    assert!(signed.split('.').count() == 3, "JWS compact form is x.y.z");

    let restored =
        verify_jws(&signed, &DecodingKey::from_secret(secret), Algorithm::HS256).expect("verify");
    assert_eq!(restored.uuid, original_uuid);
    assert_eq!(restored.subject.as_deref(), Some("JWS round-trip"));
}

#[test]
fn jws_verify_rejects_tampered_payload() {
    let secret = b"vcon-test-secret";
    let vcon = VconBuilder::new()
        .with_party(Party {
            name: Some("Original".into()),
            ..Default::default()
        })
        .build();
    let signed = sign_jws(&vcon, &EncodingKey::from_secret(secret), Algorithm::HS256).unwrap();

    // Tamper: flip a character mid-payload (second segment is base64
    // of the JSON body).
    let mut chars: Vec<char> = signed.chars().collect();
    let mid = chars.len() / 2;
    chars[mid] = if chars[mid] == 'A' { 'B' } else { 'A' };
    let tampered: String = chars.into_iter().collect();

    let result = verify_jws(
        &tampered,
        &DecodingKey::from_secret(secret),
        Algorithm::HS256,
    );
    assert!(result.is_err(), "tampered JWS must fail verification");
}
