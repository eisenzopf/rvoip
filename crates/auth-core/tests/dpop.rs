//! Tests for `DpopValidator` (plan C4 — RFC 9449 foundational layer).
//!
//! Mints DPoP-Proof JWTs with an embedded test EC keypair and runs
//! them through the validator. Verifies signature checking, iat
//! window enforcement, jti replay protection, and RFC 7638 thumbprint
//! computation.

use std::time::Duration;

use base64::Engine;
use chrono::Utc;
use jsonwebtoken::{encode, jwk::Jwk, Algorithm, EncodingKey, Header};
use rvoip_auth_core::dpop::{DpopError, DpopProof, DpopValidator};
use rvoip_auth_core::jwk_thumbprint;
use serde::Serialize;
use serde_json::json;

// --- Test ES256 keypair (P-256) -----------------------------------
// Generated via `openssl ecparam -name prime256v1 -genkey`. Test-only.

// PKCS#8-formatted (what jsonwebtoken expects).
const TEST_PRIVATE_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgBH6K2gwl4mh1D9Bt
lvWsOY7BGSierAtxKT1aYtyjK2ihRANCAAQAcd+bti5jqRhDTF4EZzkmhgvpD33g
0XKWF6natuGoD2tudy5J+jG7RwN9JrRRo5iJKCA4jhpv3EItUwkFgQU/
-----END PRIVATE KEY-----";

/// x / y coordinates of the public key, base64url-encoded, no padding.
const TEST_X: &str = "AHHfm7YuY6kYQ0xeBGc5JoYL6Q994NFylhep2rbhqA8";
const TEST_Y: &str = "a253Lkn6MbtHA30mtFGjmIkoIDiOGm_cQi1TCQWBBT8";

fn test_jwk() -> Jwk {
    let json = json!({
        "kty": "EC",
        "crv": "P-256",
        "x": TEST_X,
        "y": TEST_Y,
    });
    serde_json::from_value(json).expect("parse jwk")
}

#[derive(Serialize)]
struct Claims {
    jti: String,
    htm: String,
    htu: String,
    iat: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    ath: Option<String>,
}

fn mint_proof(jti: &str, iat_offset_secs: i64) -> String {
    let mut header = Header::new(Algorithm::ES256);
    header.typ = Some("dpop+jwt".into());
    header.jwk = Some(test_jwk());
    let claims = Claims {
        jti: jti.into(),
        htm: "POST".into(),
        htu: "https://uctp.example.com/auth".into(),
        iat: Utc::now().timestamp() + iat_offset_secs,
        ath: None,
    };
    encode(
        &header,
        &claims,
        &EncodingKey::from_ec_pem(TEST_PRIVATE_PEM.as_bytes()).expect("ec pem"),
    )
    .expect("encode dpop proof")
}

#[tokio::test]
async fn valid_dpop_proof_returns_thumbprint() {
    let validator = DpopValidator::new();
    let token = mint_proof("jti-1", 0);

    let validated = validator.validate(&token).await.expect("valid");
    assert_eq!(validated.proof.jti, "jti-1");
    assert_eq!(validated.proof.htm, "POST");
    assert_eq!(validated.proof.htu, "https://uctp.example.com/auth");
    // Thumbprint is a base64url-no-pad SHA-256 = 43 chars.
    assert_eq!(validated.jkt.len(), 43);
}

#[tokio::test]
async fn dpop_proof_with_wrong_typ_rejects() {
    // typ must be "dpop+jwt" per RFC 9449 §4.2.
    let mut header = Header::new(Algorithm::ES256);
    header.typ = Some("JWT".into());
    header.jwk = Some(test_jwk());
    let claims = Claims {
        jti: "x".into(),
        htm: "GET".into(),
        htu: "https://example.com".into(),
        iat: Utc::now().timestamp(),
        ath: None,
    };
    let token = encode(
        &header,
        &claims,
        &EncodingKey::from_ec_pem(TEST_PRIVATE_PEM.as_bytes()).unwrap(),
    )
    .unwrap();
    let validator = DpopValidator::new();
    let result = validator.validate(&token).await;
    assert!(matches!(result, Err(DpopError::Header(_))));
}

#[tokio::test]
async fn dpop_proof_with_stale_iat_rejects() {
    let validator = DpopValidator::with_leeway(Duration::from_secs(10));
    // 60s in the past — well outside the 10s leeway.
    let token = mint_proof("jti-stale", -60);
    let result = validator.validate(&token).await;
    assert!(matches!(result, Err(DpopError::IatOutOfWindow)));
}

#[tokio::test]
async fn dpop_proof_with_future_iat_rejects() {
    let validator = DpopValidator::with_leeway(Duration::from_secs(10));
    // 60s in the future — also outside the 10s leeway. This catches
    // clock-skew attacks where an attacker mints proofs for future
    // times.
    let token = mint_proof("jti-future", 60);
    let result = validator.validate(&token).await;
    assert!(matches!(result, Err(DpopError::IatOutOfWindow)));
}

#[tokio::test]
async fn dpop_proof_replay_rejects_second_use() {
    let validator = DpopValidator::new();
    let token = mint_proof("jti-once", 0);
    validator.validate(&token).await.expect("first ok");
    // Second use of the same proof — replay detected via jti cache.
    let result = validator.validate(&token).await;
    assert!(matches!(result, Err(DpopError::Replayed)));
}

#[tokio::test]
async fn dpop_proof_with_tampered_signature_rejects() {
    let validator = DpopValidator::new();
    let mut token = mint_proof("jti-tamper", 0);
    // Flip a char in the signature segment (last `.`-delimited part).
    let dot = token.rfind('.').unwrap();
    let bytes = unsafe { token.as_bytes_mut() };
    bytes[dot + 1] = if bytes[dot + 1] == b'A' { b'B' } else { b'A' };
    let result = validator.validate(&token).await;
    assert!(matches!(result, Err(DpopError::Signature(_))));
}

#[test]
fn rfc7638_thumbprint_matches_canonical_form() {
    // Smoke test: same jwk → same thumbprint, different jwks → different.
    let jwk_a = json!({
        "kty": "EC",
        "crv": "P-256",
        "x": TEST_X,
        "y": TEST_Y,
    });
    let jwk_b = json!({
        // Different x value
        "kty": "EC",
        "crv": "P-256",
        "x": "different-x-value-base64url-encoded-here-padded-to-32by",
        "y": TEST_Y,
    });
    let tp_a = jwk_thumbprint(&jwk_a).expect("thumbprint a");
    let tp_a2 = jwk_thumbprint(&jwk_a).expect("thumbprint a2");
    let tp_b = jwk_thumbprint(&jwk_b).expect("thumbprint b");
    assert_eq!(tp_a, tp_a2, "same JWK must produce same thumbprint");
    assert_ne!(tp_a, tp_b, "different JWKs must produce different thumbprints");
    // Length: SHA-256 base64url-no-pad = 43.
    assert_eq!(tp_a.len(), 43);
}

#[test]
fn rfc7638_thumbprint_canonical_ordering_independent_of_input_field_order() {
    // The canonical form sorts fields lexicographically, so the
    // thumbprint is the same regardless of the order fields appear in
    // the input JWK.
    let jwk_in_order_1 = json!({
        "kty": "EC",
        "crv": "P-256",
        "x": TEST_X,
        "y": TEST_Y,
    });
    let jwk_in_order_2 = json!({
        "y": TEST_Y,
        "kty": "EC",
        "x": TEST_X,
        "crv": "P-256",
    });
    let tp1 = jwk_thumbprint(&jwk_in_order_1).unwrap();
    let tp2 = jwk_thumbprint(&jwk_in_order_2).unwrap();
    assert_eq!(tp1, tp2);
}

#[test]
fn rfc7638_thumbprint_handles_rsa_jwk_shape() {
    let jwk = json!({
        "kty": "RSA",
        "n": "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw",
        "e": "AQAB",
    });
    let tp = jwk_thumbprint(&jwk).unwrap();
    assert_eq!(tp.len(), 43, "SHA-256 thumbprint is 43 base64url chars");
}
