use chrono::{TimeZone as _, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey};
use rvoip_vcon::{
    append_signature, content_hash, encode_base64url, sign_jws, verify_jws, verify_jws_with,
    Analysis, Attachment, CertificateReference, ContentEncoding, ContentHashes, Dialog, DialogKind,
    Disposition, IndexReferences, MemoryVconStore, Party, PartyIndices, SessionId,
    SessionIdChannel, SessionIds, TrustedKey, Vcon, VconBuilder, VconError, VconStore,
    VconStoreError,
};
use serde_json::{json, Value};
use uuid::{Uuid, Version};

const ED25519_PRIVATE_KEY: &[u8] = br#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIGrD/e7uKYqSY4twDEsRfMMuLSrODf14dpTiTK6K1YI0
-----END PRIVATE KEY-----
"#;
const ED25519_PUBLIC_KEY: &[u8] = br#"-----BEGIN PUBLIC KEY-----
MCowBQYDK2VwAyEA2+Jj2UvNCvQiUPNYRgSi0cJSPiJI6Rs6D0UTeEpQVj8=
-----END PUBLIC KEY-----
"#;
const RSA_PRIVATE_KEY: &[u8] = include_bytes!("fixtures/jws-rs256-private-key.txt");
const RSA_PUBLIC_KEY: &[u8] = include_bytes!("fixtures/jws-rs256-public-key.txt");
const TEST_X5C: &str = include_str!("fixtures/jws-rs256-cert.der.b64");

fn timestamp() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 29, 12, 0, 0).unwrap()
}

fn complete_vcon() -> Vcon {
    let mut builder = VconBuilder::new()
        .created_at(timestamp())
        .subject("A \"quoted\" subject\nwith controls")
        .extension("example-extension", false)
        .extension_parameter(
            "example-extension",
            "example_parameter",
            json!({"preserved": true}),
            false,
        );
    let alice = builder.party(Party {
        name: Some("Alice \\\n\"Agent\"".into()),
        kind: Some("person".into()),
        did: Some("did:example:alice".into()),
        ..Party::default()
    });
    let bob = builder.party(Party {
        tel: Some("tel:+15551234".into()),
        name: Some("Bob".into()),
        ..Party::default()
    });
    builder
        .recording_inline(
            timestamp(),
            120.25,
            vec![alice, bob],
            "audio/opus",
            b"\x00\xfffull recording bytes",
        )
        .text(timestamp(), alice, "hello")
        .analysis(Analysis {
            kind: "transcript".into(),
            vendor: "Example".into(),
            dialog: Some(IndexReferences::Many(vec![0, 1])),
            mediatype: Some("application/json".into()),
            body: Some(json!({"text": "hello"})),
            encoding: Some(ContentEncoding::Json),
            ..Analysis::default()
        })
        .attachment(Attachment {
            purpose: Some("prompt".into()),
            start: timestamp(),
            party: bob,
            dialog: 1,
            mediatype: Some("text/plain".into()),
            body: Some(Value::String("attached".into())),
            encoding: Some(ContentEncoding::None),
            ..Attachment::default()
        })
        .build_validated()
        .unwrap()
}

#[test]
fn builder_emits_current_version_uuid_v8_and_seconds() {
    let vcon = complete_vcon();
    assert_eq!(vcon.vcon.as_deref(), Some("0.4.0"));
    assert_eq!(vcon.uuid.get_version(), Some(Version::Custom));
    assert_eq!(vcon.dialog[0].duration, Some(120.25));
    assert_eq!(vcon.dialog[0].encoding, Some(ContentEncoding::Base64Url));

    let value = serde_json::to_value(&vcon).unwrap();
    assert_eq!(value["dialog"][0]["duration"], 120.25);
    assert_eq!(value["dialog"][0]["encoding"], "base64url");
    assert!(value["dialog"][0].get("duration_ms").is_none());
    assert!(value["parties"][0].get("role").is_none());
    assert_eq!(value["parties"][0]["type"], "person");
}

#[test]
fn json_escaping_and_unknown_extensions_round_trip_losslessly() {
    let vcon = complete_vcon();
    let encoded = serde_json::to_vec(&vcon).unwrap();
    let decoded: Vcon = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded, vcon);
    assert_eq!(
        decoded.extra["example_parameter"],
        json!({"preserved": true})
    );
    assert!(decoded.validate().is_ok());
}

#[test]
fn omitted_vcon_version_is_accepted_but_wrong_version_is_rejected() {
    let mut value = serde_json::to_value(Vcon::new_now()).unwrap();
    value.as_object_mut().unwrap().remove("vcon");
    let without_version: Vcon = serde_json::from_value(value).unwrap();
    assert_eq!(without_version.vcon, None);
    assert!(without_version.validate().is_ok());

    let mut wrong = Vcon::new_now();
    wrong.vcon = Some("0.0.1".into());
    assert!(wrong.validate().is_err());
}

#[test]
fn explicit_json_null_body_is_preserved() {
    let raw = json!({
        "uuid": Uuid::new_v4(),
        "created_at": timestamp(),
        "dialog": [{
            "type": "text",
            "start": timestamp(),
            "body": null,
            "encoding": "json",
            "mediatype": "application/json"
        }]
    });
    let vcon: Vcon = serde_json::from_value(raw).unwrap();
    assert_eq!(vcon.dialog[0].body, Some(Value::Null));
    assert!(vcon.validate().is_ok());
    assert!(serde_json::to_value(vcon).unwrap()["dialog"][0]["body"].is_null());
}

#[test]
fn empty_string_body_may_omit_encoding_but_other_bodies_may_not() {
    let mut empty = Vcon::new_now();
    empty.dialog.push(Dialog {
        kind: DialogKind::Text,
        start: timestamp(),
        mediatype: Some("text/plain".into()),
        body: Some(Value::String(String::new())),
        ..Dialog::default()
    });
    assert!(empty.validate().is_ok());

    empty.dialog[0].body = Some(Value::String("not empty".into()));
    assert!(empty.validate().is_err());
}

#[test]
fn mixed_channel_party_and_session_ids_round_trip() {
    let dialog = Dialog {
        kind: DialogKind::Recording,
        start: timestamp(),
        parties: Some(PartyIndices::Channels(vec![
            rvoip_vcon::PartyChannel::One(0),
            rvoip_vcon::PartyChannel::Many(vec![1, 2]),
            rvoip_vcon::PartyChannel::Empty(()),
        ])),
        session_id: Some(SessionIds::Channels(vec![
            SessionIdChannel::One(SessionId {
                local: Some("one".into()),
                ..SessionId::default()
            }),
            SessionIdChannel::Many(vec![
                SessionId {
                    local: Some("two".into()),
                    ..SessionId::default()
                },
                SessionId {
                    remote: Some("three".into()),
                    ..SessionId::default()
                },
            ]),
        ])),
        ..Dialog::default()
    };
    let value = serde_json::to_value(&dialog).unwrap();
    assert_eq!(value["parties"], json!([0, [1, 2], null]));
    let restored: Dialog = serde_json::from_value(value).unwrap();
    assert_eq!(restored, dialog);
}

#[test]
fn validation_enforces_dependencies_references_and_dialog_rules() {
    let mut vcon = VconBuilder::new()
        .with_party(Party::default())
        .recording(timestamp(), 1.0, vec![0], "audio/opus")
        .recording_set(timestamp(), 1.0, vec![0], vec![0])
        .build();
    assert!(vcon.validate().is_ok());

    vcon.dialog[0].recording_set = None;
    assert!(
        vcon.validate().is_ok(),
        "recording_set back-reference is recommended, not required"
    );
    vcon.dialog[0].recording_set = Some(1);

    vcon.dialog[0].parties = Some(PartyIndices::One(99));
    assert!(vcon.validate().is_err(), "party index must be in bounds");
    vcon.dialog[0].parties = Some(PartyIndices::One(0));

    vcon.dialog[0].url = Some("https://media.example/recording".into());
    assert!(vcon.validate().is_err(), "url requires content_hash");
    vcon.dialog[0].content_hash = Some(content_hash(b"recording").into());
    assert!(vcon.validate().is_ok());
    vcon.dialog[0].url = None;
    assert!(vcon.validate().is_err(), "content_hash requires url");

    let invalid_incomplete = VconBuilder::new()
        .with_party(Party::default())
        .with_dialog(Dialog {
            kind: DialogKind::Incomplete,
            start: timestamp(),
            ..Dialog::default()
        })
        .build();
    assert!(invalid_incomplete.validate().is_err());

    let valid_incomplete = VconBuilder::new()
        .with_party(Party::default())
        .incomplete(timestamp(), vec![0], Disposition::Busy)
        .build();
    assert!(valid_incomplete.validate().is_ok());

    let invalid_transfer = VconBuilder::new()
        .with_party(Party::default())
        .with_dialog(Dialog {
            kind: DialogKind::Transfer,
            start: timestamp(),
            parties: Some(PartyIndices::One(0)),
            ..Dialog::default()
        })
        .build();
    assert!(
        invalid_transfer.validate().is_err(),
        "transfer must not carry parties"
    );

    let invalid_recordings = VconBuilder::new()
        .with_party(Party::default())
        .with_dialog(Dialog {
            kind: DialogKind::Incomplete,
            start: timestamp(),
            disposition: Some(Disposition::Failed),
            recordings: vec![0],
            ..Dialog::default()
        })
        .build();
    assert!(
        invalid_recordings.validate().is_err(),
        "recordings is exclusive to recording-set"
    );
}

#[test]
fn validation_enforces_extension_and_analysis_metadata() {
    let mut vcon = Vcon::new_now();
    vcon.critical.push("unknown".into());
    assert!(vcon.validate().is_err());

    vcon.extensions.push("unknown".into());
    assert!(vcon.validate().is_ok());
    assert!(vcon
        .validate_with_supported_extensions(std::iter::empty())
        .is_err());
    assert!(vcon.validate_with_supported_extensions(["unknown"]).is_ok());

    vcon.analysis.push(Analysis::default());
    assert!(vcon.validate().is_err());
}

#[test]
fn extension_parameters_must_be_declared_and_cannot_use_reserved_or_core_names() {
    let mut vcon = Vcon::new_now();
    vcon.parties.push(Party::default());
    vcon.parties[0]
        .extra
        .insert("example_party_parameter".into(), json!(true));
    assert!(
        vcon.validate().is_err(),
        "unknown parameters require a declared extension"
    );

    vcon.extensions.push("example-extension".into());
    assert!(vcon.validate().is_ok());
    let restored: Vcon = serde_json::from_value(serde_json::to_value(&vcon).unwrap()).unwrap();
    assert_eq!(
        restored.parties[0].extra["example_party_parameter"],
        json!(true)
    );

    vcon.extra.insert("group".into(), json!("federation"));
    assert!(
        vcon.validate().is_err(),
        "group is a reserved parameter and must not be emitted"
    );
    vcon.extra.remove("group");

    vcon.parties[0].extra.insert("name".into(), json!("shadow"));
    assert!(
        vcon.validate().is_err(),
        "flattened extension parameters cannot collide with core properties"
    );
}

#[test]
fn validation_rejects_empty_amendment_and_malformed_https_references() {
    let mut vcon = Vcon::new_now();
    vcon.amended = Some(rvoip_vcon::Amended::default());
    assert!(vcon.validate().is_err());

    vcon.amended.as_mut().unwrap().uuid = Some(vcon.uuid);
    assert!(
        vcon.validate().is_err(),
        "amendment lineage cannot directly reference itself"
    );
    vcon.amended.as_mut().unwrap().uuid = None;

    vcon.amended.as_mut().unwrap().url = Some("https://".into());
    vcon.amended.as_mut().unwrap().content_hash = Some(content_hash(b"prior").into());
    assert!(vcon.validate().is_err());

    vcon.amended.as_mut().unwrap().url = Some("https://archive.example/prior".into());
    assert!(vcon.validate().is_ok());

    vcon.amended = None;
    vcon.redacted = Some(rvoip_vcon::Redacted {
        uuid: Some(vcon.uuid),
        kind: "pii".into(),
        ..rvoip_vcon::Redacted::default()
    });
    assert!(
        vcon.validate().is_err(),
        "redaction lineage cannot directly reference itself"
    );
}

#[test]
fn validation_rejects_malformed_dids_and_non_sha512_content_hashes() {
    let mut vcon = VconBuilder::new()
        .with_party(Party {
            did: Some("did:example:alice".into()),
            ..Party::default()
        })
        .recording(timestamp(), 1.0, vec![0], "audio/opus")
        .build();
    assert!(vcon.validate().is_ok());

    for malformed in [
        "https://identity.example/alice",
        "did::alice",
        "did:Example:alice",
        "did:example:",
        "did:example:alice smith",
        "did:example:alice#key-1",
        "did:example:alice/keys",
        "did:example:al!ce",
        "did:example:alice:",
        "did:example:alice%2",
    ] {
        vcon.parties[0].did = Some(malformed.into());
        assert!(
            vcon.validate().is_err(),
            "malformed DID {malformed:?} must fail"
        );
    }

    vcon.parties[0].did = Some("did:example:alice::device-1_%2E".into());
    assert!(vcon.validate().is_ok());
    vcon.dialog[0].url = Some("https://media.example/recording".into());
    vcon.dialog[0].content_hash = Some(ContentHashes::One("sha512-".into()));
    assert!(vcon.validate().is_err(), "empty SHA-512 digest must fail");

    vcon.dialog[0].content_hash = Some(ContentHashes::One(format!(
        "sha256-{}",
        encode_base64url([0_u8; 32])
    )));
    assert!(
        vcon.validate().is_err(),
        "unsupported hash algorithms must fail"
    );

    vcon.dialog[0].content_hash = Some(content_hash(b"recording").into());
    assert!(vcon.validate().is_ok());
}

#[test]
fn validation_correlates_session_ids_and_transfer_dialog_types() {
    let mut vcon = Vcon::new_now();
    vcon.parties = vec![Party::default(), Party::default()];
    vcon.dialog.push(Dialog {
        kind: DialogKind::Recording,
        start: timestamp(),
        parties: Some(PartyIndices::Many(vec![0, 1])),
        session_id: Some(SessionIds::Many(vec![SessionId::default()])),
        ..Dialog::default()
    });
    assert!(vcon.validate().is_err());
    vcon.dialog[0].session_id = Some(SessionIds::Many(vec![
        SessionId::default(),
        SessionId::default(),
    ]));
    assert!(vcon.validate().is_ok());
    vcon.dialog[0].parties = Some(PartyIndices::One(0));
    vcon.dialog[0].session_id = Some(SessionIds::Many(vec![SessionId::default()]));
    assert!(
        vcon.validate().is_err(),
        "a scalar parties value requires a scalar SessionId object"
    );
    vcon.dialog[0].parties = Some(PartyIndices::Many(vec![0, 1]));
    vcon.dialog[0].session_id = Some(SessionIds::Many(vec![
        SessionId::default(),
        SessionId::default(),
    ]));

    vcon.dialog.push(Dialog {
        kind: DialogKind::Transfer,
        start: timestamp(),
        original: Some(IndexReferences::One(1)),
        ..Dialog::default()
    });
    assert!(
        vcon.validate().is_err(),
        "original cannot reference the transfer dialog itself"
    );
    vcon.dialog[1].original = Some(IndexReferences::One(0));
    assert!(vcon.validate().is_ok());
}

#[test]
fn validation_enforces_rfc_7989_session_identifier_syntax() {
    let mut vcon = Vcon::new_now();
    vcon.parties.push(Party::default());
    vcon.dialog.push(Dialog {
        kind: DialogKind::Recording,
        start: timestamp(),
        parties: Some(PartyIndices::One(0)),
        session_id: Some(SessionIds::One(SessionId {
            local: Some("not-a-session-id".into()),
            ..SessionId::default()
        })),
        ..Dialog::default()
    });
    assert!(vcon.validate().is_err());

    vcon.dialog[0].session_id = Some(SessionIds::One(SessionId {
        local: Some("0123456789abcdef0123456789abcdef".into()),
        remote: Some("00000000000000000000000000000000".into()),
        ..SessionId::default()
    }));
    assert!(vcon.validate().is_ok());

    vcon.dialog[0].session_id = Some(SessionIds::One(SessionId {
        local: Some("0123456789ABCDEF0123456789ABCDEF".into()),
        ..SessionId::default()
    }));
    assert!(
        vcon.validate().is_err(),
        "RFC 7989 session identifiers use lowercase hexadecimal"
    );
}

#[test]
fn session_id_channels_use_an_empty_object_for_a_channel_without_a_party() {
    let mut vcon = Vcon::new_now();
    vcon.parties.push(Party::default());
    vcon.dialog.push(Dialog {
        kind: DialogKind::Recording,
        start: timestamp(),
        parties: Some(PartyIndices::Channels(vec![
            rvoip_vcon::PartyChannel::One(0),
            rvoip_vcon::PartyChannel::Empty(()),
        ])),
        session_id: Some(SessionIds::Channels(vec![
            SessionIdChannel::One(SessionId {
                local: Some("0123456789abcdef0123456789abcdef".into()),
                ..SessionId::default()
            }),
            SessionIdChannel::One(SessionId::default()),
        ])),
        ..Dialog::default()
    });
    assert!(vcon.validate().is_ok());
    let round_tripped: Vcon = serde_json::from_value(serde_json::to_value(&vcon).unwrap()).unwrap();
    assert!(
        round_tripped.validate().is_ok(),
        "flat channel session-id arrays must remain valid after untagged deserialization"
    );

    vcon.dialog[0].session_id = Some(SessionIds::Channels(vec![
        SessionIdChannel::One(SessionId {
            local: Some("0123456789abcdef0123456789abcdef".into()),
            ..SessionId::default()
        }),
        SessionIdChannel::Many(Vec::new()),
    ]));
    assert!(
        vcon.validate().is_err(),
        "the draft requires an empty SessionId object, not an empty array"
    );

    vcon.dialog[0].parties = Some(PartyIndices::Channels(vec![
        rvoip_vcon::PartyChannel::Many(vec![0]),
    ]));
    vcon.dialog[0].session_id = Some(SessionIds::Many(vec![SessionId::default()]));
    assert!(
        vcon.validate().is_err(),
        "a nested parties channel requires a nested SessionId channel"
    );
}

#[test]
fn validation_rejects_invalid_media_types() {
    let mut vcon = VconBuilder::new()
        .with_party(Party::default())
        .text(timestamp(), 0, "hello")
        .build();
    vcon.dialog[0].mediatype = Some("not a media type".into());
    assert!(vcon.validate().is_err());
}

#[test]
fn inline_content_requires_mediatype() {
    let mut vcon = VconBuilder::new()
        .with_party(Party::default())
        .text(timestamp(), 0, "hello")
        .build();
    assert!(vcon.validate().is_ok());
    vcon.dialog[0].mediatype = None;
    assert!(vcon.validate().is_err());
}

#[test]
fn sha512_content_hash_matches_known_vector() {
    assert_eq!(
        content_hash([]),
        "sha512-z4PhNX7vuL3xVChQ1m2AB9Yg5AULVxXcg_SpIdNs6c5H0NE8XYXysP-DGNKHfuwvY7kxvUdBeoGlODJ6-SfaPg"
    );
}

#[test]
fn jws_general_json_sign_append_and_verify() {
    let rsa_private_key = EncodingKey::from_rsa_pem(RSA_PRIVATE_KEY).unwrap();
    let rsa_public_key = DecodingKey::from_rsa_pem(RSA_PUBLIC_KEY).unwrap();
    let ed_private_key = EncodingKey::from_ed_pem(ED25519_PRIVATE_KEY).unwrap();
    let ed_public_key = DecodingKey::from_ed_pem(ED25519_PUBLIC_KEY).unwrap();
    let vcon = complete_vcon();
    let mut signed = sign_jws(
        &vcon,
        &rsa_private_key,
        Algorithm::RS256,
        CertificateReference::X5c(vec![TEST_X5C.trim().into()]),
    )
    .unwrap();

    let value = serde_json::to_value(&signed).unwrap();
    assert!(value["payload"].is_string());
    assert!(value["signatures"].is_array());
    assert_eq!(value["signatures"][0]["header"]["x5c"][0], TEST_X5C.trim());
    assert!(value["signatures"][0]["protected"].is_string());
    assert!(value["signatures"][0]["signature"].is_string());

    append_signature(
        &mut signed,
        &ed_private_key,
        Algorithm::EdDSA,
        CertificateReference::X5u("https://keys.example/signer.pem".into()),
    )
    .unwrap();
    assert_eq!(signed.signatures.len(), 2);

    let restored = verify_jws_with(&signed, |protected, _unprotected| match protected.alg {
        Algorithm::RS256 => Ok(TrustedKey::new(rsa_public_key.clone(), Algorithm::RS256)),
        Algorithm::EdDSA => Ok(TrustedKey::new(ed_public_key.clone(), Algorithm::EdDSA)),
        algorithm => Err(VconError::Verify(format!(
            "unexpected test algorithm {algorithm:?}"
        ))),
    })
    .unwrap();
    assert_eq!(restored, vcon);
}

#[test]
fn jws_rejects_hmac_tampering_and_missing_signatures() {
    let vcon = complete_vcon();
    let hmac = sign_jws(
        &vcon,
        &EncodingKey::from_secret(b"not-allowed"),
        Algorithm::HS256,
        CertificateReference::X5u("https://keys.example/hmac.pem".into()),
    );
    assert!(hmac.is_err());

    let private_key = EncodingKey::from_ed_pem(ED25519_PRIVATE_KEY).unwrap();
    let public_key = DecodingKey::from_ed_pem(ED25519_PUBLIC_KEY).unwrap();
    let mut signed = sign_jws(
        &vcon,
        &private_key,
        Algorithm::EdDSA,
        CertificateReference::X5u("https://keys.example/signer.pem".into()),
    )
    .unwrap();
    signed.payload.replace_range(0..1, "A");
    assert!(verify_jws(&signed, &public_key, Algorithm::EdDSA).is_err());

    signed.signatures.clear();
    assert!(verify_jws(&signed, &public_key, Algorithm::EdDSA).is_err());
}

#[test]
fn jws_rejects_invalid_certificate_headers_uuid_and_compressed_payloads() {
    let private_key = EncodingKey::from_ed_pem(ED25519_PRIVATE_KEY).unwrap();
    let public_key = DecodingKey::from_ed_pem(ED25519_PUBLIC_KEY).unwrap();
    let vcon = complete_vcon();

    assert!(sign_jws(
        &vcon,
        &private_key,
        Algorithm::EdDSA,
        CertificateReference::X5c(vec!["not base64!".into()]),
    )
    .is_err());
    assert!(sign_jws(
        &vcon,
        &private_key,
        Algorithm::EdDSA,
        CertificateReference::X5c(vec!["bm90LXgtNTA5".into()]),
    )
    .is_err());
    assert!(sign_jws(
        &vcon,
        &private_key,
        Algorithm::EdDSA,
        CertificateReference::X5c(Vec::new()),
    )
    .is_err());
    assert!(sign_jws(
        &vcon,
        &private_key,
        Algorithm::EdDSA,
        CertificateReference::X5u("http://keys.example/signer.pem".into()),
    )
    .is_err());

    let mut signed = sign_jws(
        &vcon,
        &private_key,
        Algorithm::EdDSA,
        CertificateReference::X5u("https://keys.example/signer.pem".into()),
    )
    .unwrap();
    signed.extra.insert("payload".into(), json!("shadow"));
    assert!(verify_jws(&signed, &public_key, Algorithm::EdDSA).is_err());
    signed.extra.clear();
    signed.signatures[0]
        .extra
        .insert("signature".into(), json!("shadow"));
    assert!(verify_jws(&signed, &public_key, Algorithm::EdDSA).is_err());
    signed.signatures[0].extra.clear();

    signed.signatures[0]
        .header
        .extra
        .insert("alg".into(), json!("EdDSA"));
    assert!(verify_jws(&signed, &public_key, Algorithm::EdDSA).is_err());
    signed.signatures[0].header.extra.clear();

    let critical_header = json!({
        "alg": "EdDSA",
        "uuid": vcon.uuid,
        "crit": ["b64"],
        "b64": false
    });
    signed.signatures[0].protected =
        encode_base64url(serde_json::to_vec(&critical_header).unwrap());
    let signing_input = format!("{}.{}", signed.signatures[0].protected, signed.payload);
    signed.signatures[0].signature =
        jsonwebtoken::crypto::sign(signing_input.as_bytes(), &private_key, Algorithm::EdDSA)
            .unwrap();
    assert!(
        verify_jws(&signed, &public_key, Algorithm::EdDSA).is_err(),
        "unsupported critical and unencoded-payload headers must be rejected"
    );

    let wrong_uuid_header = json!({"alg": "EdDSA", "uuid": Uuid::new_v4()});
    signed.signatures[0].protected =
        encode_base64url(serde_json::to_vec(&wrong_uuid_header).unwrap());
    let signing_input = format!("{}.{}", signed.signatures[0].protected, signed.payload);
    signed.signatures[0].signature =
        jsonwebtoken::crypto::sign(signing_input.as_bytes(), &private_key, Algorithm::EdDSA)
            .unwrap();
    assert!(verify_jws(&signed, &public_key, Algorithm::EdDSA).is_err());

    let protected = json!({"alg": "EdDSA", "uuid": vcon.uuid});
    signed.signatures[0].protected = encode_base64url(serde_json::to_vec(&protected).unwrap());
    signed.payload = encode_base64url([0x1f, 0x8b, 0x08, 0x00]);
    let signing_input = format!("{}.{}", signed.signatures[0].protected, signed.payload);
    signed.signatures[0].signature =
        jsonwebtoken::crypto::sign(signing_input.as_bytes(), &private_key, Algorithm::EdDSA)
            .unwrap();
    assert!(verify_jws(&signed, &public_key, Algorithm::EdDSA).is_err());
}

#[test]
fn schema_core_required_fields_and_enums_match_pinned_commit() {
    let schema: Value =
        serde_json::from_str(include_str!("fixtures/vcon_json_schema.json")).unwrap();
    assert_eq!(schema["required"], json!(["uuid", "created_at"]));
    assert_eq!(schema["properties"]["vcon"]["const"], "0.4.0");
    assert_eq!(
        schema["definitions"]["Dialog"]["properties"]["type"]["enum"],
        json!([
            "recording",
            "text",
            "transfer",
            "incomplete",
            "recording-set"
        ])
    );
    assert_eq!(
        schema["definitions"]["Dialog"]["properties"]["encoding"]["enum"],
        json!(["base64url", "json", "none"])
    );
    assert_eq!(
        schema["definitions"]["Analysis"]["required"],
        json!(["type", "vendor"])
    );
    assert_eq!(
        schema["definitions"]["Attachment"]["required"],
        json!(["start", "party", "dialog"])
    );
}

#[test]
fn generated_and_curated_documents_validate_against_pinned_schema_offline() {
    let schema: Value =
        serde_json::from_str(include_str!("fixtures/vcon_json_schema.json")).unwrap();
    let validator = jsonschema::draft7::new(&schema).unwrap();

    let generated = serde_json::to_value(complete_vcon()).unwrap();
    if let Err(error) = validator.validate(&generated) {
        panic!("generated vCon failed pinned schema: {error}");
    }

    let curated: Value =
        serde_json::from_str(include_str!("fixtures/alice_email_curated.vcon")).unwrap();
    if let Err(error) = validator.validate(&curated) {
        panic!("curated upstream vCon failed pinned schema: {error}");
    }
    let typed: Vcon = serde_json::from_value(curated).unwrap();
    typed.validate().unwrap();
}

#[tokio::test]
async fn memory_store_round_trip_and_overwrite_contract() {
    let store = MemoryVconStore::new();
    let vcon = complete_vcon();
    let uuid = vcon.uuid;
    let expected_hash = content_hash(serde_json::to_vec(&vcon).unwrap());
    store.put(vcon.clone()).await.unwrap();
    assert_eq!(store.get(&uuid).await.unwrap(), vcon);
    assert_eq!(store.content_hash(&uuid).await.unwrap(), expected_hash);
    assert!(matches!(
        store.put(vcon.clone()).await,
        Err(VconStoreError::Backend(_))
    ));
    let mut overwritten = vcon;
    overwritten.subject = Some("updated".into());
    let overwritten_hash = content_hash(serde_json::to_vec(&overwritten).unwrap());
    store.put_overwrite(overwritten).await.unwrap();
    assert_eq!(store.content_hash(&uuid).await.unwrap(), overwritten_hash);
    store.delete(&uuid).await.unwrap();
    store.delete(&uuid).await.unwrap();
    assert!(matches!(
        store.get(&uuid).await,
        Err(VconStoreError::NotFound(_))
    ));
    assert!(matches!(
        store.content_hash(&uuid).await,
        Err(VconStoreError::NotFound(_))
    ));
}

#[tokio::test]
async fn memory_store_rejects_invalid_documents() {
    let store = MemoryVconStore::new();
    let mut invalid = Vcon::new_now();
    invalid.critical.push("undeclared-extension".into());
    assert!(matches!(
        store.put(invalid.clone()).await,
        Err(VconStoreError::Backend(_))
    ));
    assert!(matches!(
        store.put_overwrite(invalid).await,
        Err(VconStoreError::Backend(_))
    ));
    assert_eq!(store.len().await, Some(0));
}

#[test]
fn content_hash_union_serializes_as_scalar_or_array() {
    assert_eq!(
        serde_json::to_value(ContentHashes::One("sha512-a".into())).unwrap(),
        json!("sha512-a")
    );
    assert_eq!(
        serde_json::to_value(ContentHashes::Many(vec![
            "sha512-a".into(),
            "sha384-b".into()
        ]))
        .unwrap(),
        json!(["sha512-a", "sha384-b"])
    );
}
