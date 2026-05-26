//! P7 — JCS canonicalization + replay cache + IdentityProvider trait
//! addition reachable from rvoip-core.

use rvoip_core::signing::{
    body_digest_hex, canonical_envelope, parse_signature_input, ReplayCache,
};
use std::time::Duration;

#[test]
fn canonical_envelope_sorts_object_keys_lexicographically() {
    let v = serde_json::json!({
        "z": 1,
        "a": 2,
        "m": [3, 2, 1],
    });
    let s = String::from_utf8(canonical_envelope(&v)).unwrap();
    // Keys in lex order; arrays preserve element order.
    assert_eq!(s, r#"{"a":2,"m":[3,2,1],"z":1}"#);
}

#[test]
fn replay_cache_rejects_duplicates_within_ttl() {
    let c = ReplayCache::new(Duration::from_secs(60));
    c.check_and_record("env-1").unwrap();
    assert!(c.check_and_record("env-1").is_err());
    c.check_and_record("env-2").unwrap();
}

#[test]
fn parse_signature_input_extracts_keyid_and_components() {
    let s = parse_signature_input(
        r#"sig1=("@method" "@target-uri" "content-digest");keyid="key-ed25519-1";alg="ed25519";created=1700000000"#,
    );
    assert_eq!(s.key_id.as_deref(), Some("key-ed25519-1"));
    assert_eq!(s.algorithm.as_deref(), Some("ed25519"));
    assert_eq!(s.created, Some(1700000000));
    assert_eq!(
        s.covered_components,
        vec![
            "@method".to_string(),
            "@target-uri".to_string(),
            "content-digest".to_string()
        ]
    );
}

#[test]
fn body_digest_hex_matches_known_sha256() {
    // "" sha256 = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
    assert_eq!(
        body_digest_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}
