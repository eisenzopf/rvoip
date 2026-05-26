//! RFC 9421 HTTP Message Signatures — UCTP inline envelope variant.
//!
//! Per [CONVERSATION_PROTOCOL.md §5.5.1], signed UCTP envelopes carry
//! an inline `signature: { keyid, alg, sig }` object. Verification:
//!
//! 1. Parse the envelope as JSON.
//! 2. Clone it and remove the `signature` field.
//! 3. Serialize the clone using RFC 8785 JSON Canonical Form.
//! 4. Verify `signature.sig` over the canonicalized bytes using the
//!    public key resolved via `signature.keyid`.
//! 5. Check `envelope.id` is not in the replay cache; add it.
//! 6. Check `envelope.ts` is within the cache TTL.
//!
//! v0 ships [`Sig9421Verifier`] for Ed25519 keys (the recommended
//! algorithm per §5.5.1). Other algorithms (`ES256`, `PS256`, `RS256`)
//! follow the same shape — gated behind future enhancements as
//! deployments need them.
//!
//! Replay protection mirrors [`crate::dpop`]'s moka-based JTI cache:
//! the envelope's `id` is the deduplication key; default TTL is 5
//! minutes per the spec.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use chrono::{DateTime, Utc};
use moka::future::Cache;
use ring::signature::{UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Maximum age of a signed envelope's `ts` field. Envelopes older
/// than this are rejected as stale per spec §5.5.1. Default matches
/// the spec's 5-minute window.
pub const DEFAULT_SIG_REPLAY_TTL: Duration = Duration::from_secs(300);

/// Maximum number of envelope IDs the replay cache holds before
/// LRU eviction. Mirrors [`crate::dpop::DEFAULT_JTI_CACHE_CAPACITY`].
pub const DEFAULT_REPLAY_CACHE_CAPACITY: u64 = 100_000;

#[derive(Debug, Error)]
pub enum Sig9421Error {
    #[error("envelope missing required `signature` field")]
    MissingSignature,

    #[error("malformed signature object: {0}")]
    MalformedSignature(String),

    #[error("unsupported signature algorithm: {0}")]
    UnsupportedAlgorithm(String),

    #[error("signature keyid `{0}` does not resolve to a registered public key")]
    UnknownKeyid(String),

    #[error("signature verification failed")]
    InvalidSignature,

    #[error("envelope replay detected: id `{0}` already seen")]
    ReplayDetected(String),

    #[error("envelope timestamp `{0}` is older than the replay window")]
    StaleTimestamp(String),

    #[error("envelope is not a JSON object")]
    MalformedEnvelope,

    #[error("envelope `id` field missing or not a string")]
    MissingEnvelopeId,

    #[error("envelope `ts` field missing or not a valid RFC 3339 timestamp")]
    InvalidEnvelopeTimestamp,
}

/// Inline `signature` field on a signed envelope. See
/// CONVERSATION_PROTOCOL.md §5.5.1.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnvelopeSignature {
    pub keyid: String,
    /// JWA algorithm name (e.g. `"EdDSA"`, `"ES256"`).
    pub alg: String,
    /// Base64url-encoded (no padding) signature bytes.
    pub sig: String,
}

/// Trait the verifier uses to look up the public key bytes for a
/// `keyid`. Production deployments back this with their identity
/// store; tests typically use [`StaticKeyResolver`].
pub trait KeyResolver: Send + Sync {
    /// Returns the raw public key bytes for `keyid`, or `None` if
    /// the keyid is unknown. For Ed25519 the slice is the 32-byte
    /// raw public key.
    fn resolve(&self, keyid: &str) -> Option<Vec<u8>>;
}

/// In-memory key resolver — useful for tests and static deployments.
pub struct StaticKeyResolver {
    keys: std::collections::HashMap<String, Vec<u8>>,
}

impl StaticKeyResolver {
    pub fn new() -> Self {
        Self {
            keys: std::collections::HashMap::new(),
        }
    }

    pub fn insert(&mut self, keyid: impl Into<String>, public_key: Vec<u8>) {
        self.keys.insert(keyid.into(), public_key);
    }
}

impl Default for StaticKeyResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyResolver for StaticKeyResolver {
    fn resolve(&self, keyid: &str) -> Option<Vec<u8>> {
        self.keys.get(keyid).cloned()
    }
}

/// Verifier for inline RFC 9421 envelope signatures.
///
/// Owns the replay-protection cache, so a single verifier instance
/// should be shared across all envelopes that should not replay
/// against one another (typically one per Connection or one per
/// process, depending on the threat model).
pub struct Sig9421Verifier {
    resolver: Arc<dyn KeyResolver>,
    replay_cache: Cache<String, ()>,
    ttl: Duration,
}

impl Sig9421Verifier {
    pub fn new(resolver: Arc<dyn KeyResolver>) -> Self {
        Self::with_ttl(resolver, DEFAULT_SIG_REPLAY_TTL)
    }

    pub fn with_ttl(resolver: Arc<dyn KeyResolver>, ttl: Duration) -> Self {
        Self {
            resolver,
            replay_cache: Cache::builder()
                .max_capacity(DEFAULT_REPLAY_CACHE_CAPACITY)
                .time_to_live(ttl)
                .build(),
            ttl,
        }
    }

    /// Verify an inline-signed envelope. `envelope` is the parsed
    /// JSON value (as it arrived on the wire — typically via
    /// `serde_json::from_str`). On success the envelope's id is
    /// added to the replay cache so subsequent calls with the same
    /// id are rejected.
    pub async fn verify(&self, envelope: &serde_json::Value) -> Result<(), Sig9421Error> {
        let obj = envelope
            .as_object()
            .ok_or(Sig9421Error::MalformedEnvelope)?;

        // 1. Pull the signature field.
        let sig_value = obj
            .get("signature")
            .ok_or(Sig9421Error::MissingSignature)?;
        let signature: EnvelopeSignature = serde_json::from_value(sig_value.clone())
            .map_err(|e| Sig9421Error::MalformedSignature(e.to_string()))?;

        // 2. Pull envelope id + ts for replay / freshness checks.
        let env_id = obj
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or(Sig9421Error::MissingEnvelopeId)?
            .to_string();
        let env_ts_str = obj
            .get("ts")
            .and_then(|v| v.as_str())
            .ok_or(Sig9421Error::InvalidEnvelopeTimestamp)?;
        let env_ts: DateTime<Utc> = DateTime::parse_from_rfc3339(env_ts_str)
            .map_err(|_| Sig9421Error::InvalidEnvelopeTimestamp)?
            .with_timezone(&Utc);
        let age = Utc::now().signed_duration_since(env_ts);
        if age > chrono::Duration::from_std(self.ttl).unwrap_or(chrono::Duration::seconds(300)) {
            return Err(Sig9421Error::StaleTimestamp(env_ts_str.to_string()));
        }

        // 3. Build the canonicalization base: clone the envelope,
        // strip `signature`, JCS-serialize.
        let mut bare = obj.clone();
        bare.remove("signature");
        let canonical = jcs_canonicalize(&serde_json::Value::Object(bare));

        // 4. Resolve the signing key and verify.
        let pubkey = self
            .resolver
            .resolve(&signature.keyid)
            .ok_or_else(|| Sig9421Error::UnknownKeyid(signature.keyid.clone()))?;
        let sig_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(signature.sig.as_bytes())
            .map_err(|_| Sig9421Error::InvalidSignature)?;

        match signature.alg.as_str() {
            "EdDSA" => {
                let key = UnparsedPublicKey::new(&ED25519, &pubkey);
                key.verify(canonical.as_bytes(), &sig_bytes)
                    .map_err(|_| Sig9421Error::InvalidSignature)?;
            }
            other => return Err(Sig9421Error::UnsupportedAlgorithm(other.to_string())),
        }

        // 5. Replay check after signature passes (don't burn cache
        //    slots on rejected signatures).
        if self.replay_cache.get(&env_id).await.is_some() {
            return Err(Sig9421Error::ReplayDetected(env_id));
        }
        self.replay_cache.insert(env_id, ()).await;

        Ok(())
    }
}

/// RFC 8785 JSON Canonical Form serializer for the envelope shape
/// (objects of strings/numbers/booleans/null/arrays/sub-objects).
///
/// Key properties: object keys sorted by code unit, no insignificant
/// whitespace, strings escaped per JSON, numbers in the shortest
/// round-trip form. Our envelope payload is bounded to these JSON
/// primitives (no exotic types), so this minimal implementation is
/// sufficient. Production hardening would swap in a fully RFC-8785-
/// compliant crate (e.g. `serde_jcs`).
pub fn jcs_canonicalize(value: &serde_json::Value) -> String {
    let mut out = String::new();
    jcs_write(value, &mut out);
    out
}

fn jcs_write(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::Null => out.push_str("null"),
        serde_json::Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        serde_json::Value::Number(n) => {
            // serde_json's Number Display uses the shortest
            // round-trip representation per the underlying float
            // formatter, which is consistent with JCS for finite
            // numbers we'd see in an envelope.
            out.push_str(&n.to_string());
        }
        serde_json::Value::String(s) => {
            out.push('"');
            for ch in s.chars() {
                match ch {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    '\u{08}' => out.push_str("\\b"),
                    '\u{0c}' => out.push_str("\\f"),
                    c if (c as u32) < 0x20 => {
                        out.push_str(&format!("\\u{:04x}", c as u32));
                    }
                    c => out.push(c),
                }
            }
            out.push('"');
        }
        serde_json::Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                jcs_write(item, out);
            }
            out.push(']');
        }
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                jcs_write(&serde_json::Value::String((*key).clone()), out);
                out.push(':');
                jcs_write(&map[*key], out);
            }
            out.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::rand::{SecureRandom, SystemRandom};
    use ring::signature::{Ed25519KeyPair, KeyPair};

    fn signing_keypair() -> (Ed25519KeyPair, Vec<u8>) {
        let rng = SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let kp = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let pub_bytes = kp.public_key().as_ref().to_vec();
        (kp, pub_bytes)
    }

    fn build_envelope() -> serde_json::Value {
        // Use a fresh "now" so the freshness check passes.
        serde_json::json!({
            "v": 1,
            "type": "session.invite",
            "id": "env_sig_test_1",
            "ts": Utc::now().to_rfc3339(),
            "sid": "sess_abc",
            "cid": "conv_abc",
            "payload": {
                "from": "part_alice",
                "to": ["part_bob"],
                "medium": "voice",
            }
        })
    }

    fn sign_envelope(
        envelope: &mut serde_json::Value,
        keyid: &str,
        kp: &Ed25519KeyPair,
    ) {
        // Strip any existing signature, canonicalize, sign, re-attach.
        let obj = envelope.as_object_mut().unwrap();
        obj.remove("signature");
        let canonical = jcs_canonicalize(envelope);
        let sig = kp.sign(canonical.as_bytes());
        let sig_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sig.as_ref());
        let obj = envelope.as_object_mut().unwrap();
        obj.insert(
            "signature".to_string(),
            serde_json::json!({
                "keyid": keyid,
                "alg": "EdDSA",
                "sig": sig_b64,
            }),
        );
    }

    #[tokio::test]
    async fn round_trip_signed_envelope_verifies() {
        let (kp, pubkey) = signing_keypair();
        let mut resolver = StaticKeyResolver::new();
        resolver.insert("key:agent-1", pubkey);
        let verifier = Sig9421Verifier::new(Arc::new(resolver));

        let mut env = build_envelope();
        sign_envelope(&mut env, "key:agent-1", &kp);

        verifier.verify(&env).await.expect("verify");
    }

    #[tokio::test]
    async fn tampered_payload_fails_verification() {
        let (kp, pubkey) = signing_keypair();
        let mut resolver = StaticKeyResolver::new();
        resolver.insert("key:agent-1", pubkey);
        let verifier = Sig9421Verifier::new(Arc::new(resolver));

        let mut env = build_envelope();
        sign_envelope(&mut env, "key:agent-1", &kp);

        // Mutate the payload after signing.
        env["payload"]["from"] = serde_json::json!("part_mallory");

        let err = verifier.verify(&env).await.unwrap_err();
        assert!(matches!(err, Sig9421Error::InvalidSignature));
    }

    #[tokio::test]
    async fn replay_rejected_on_second_call() {
        let (kp, pubkey) = signing_keypair();
        let mut resolver = StaticKeyResolver::new();
        resolver.insert("key:agent-1", pubkey);
        let verifier = Sig9421Verifier::new(Arc::new(resolver));

        let mut env = build_envelope();
        sign_envelope(&mut env, "key:agent-1", &kp);

        verifier.verify(&env).await.expect("first verify");
        let err = verifier.verify(&env).await.unwrap_err();
        assert!(matches!(err, Sig9421Error::ReplayDetected(_)));
    }

    #[tokio::test]
    async fn unknown_keyid_rejected() {
        let (kp, _pubkey) = signing_keypair();
        // Resolver has no keys.
        let verifier = Sig9421Verifier::new(Arc::new(StaticKeyResolver::new()));
        let mut env = build_envelope();
        sign_envelope(&mut env, "key:agent-unknown", &kp);

        let err = verifier.verify(&env).await.unwrap_err();
        assert!(matches!(err, Sig9421Error::UnknownKeyid(_)));
    }

    #[tokio::test]
    async fn cross_key_tampering_rejected() {
        let (kp_a, pubkey_a) = signing_keypair();
        let (_kp_b, pubkey_b) = signing_keypair();
        let mut resolver = StaticKeyResolver::new();
        // Register pubkey_b under agent-1, but sign with kp_a.
        resolver.insert("key:agent-1", pubkey_b);
        let _ = pubkey_a;
        let verifier = Sig9421Verifier::new(Arc::new(resolver));
        let mut env = build_envelope();
        sign_envelope(&mut env, "key:agent-1", &kp_a);

        let err = verifier.verify(&env).await.unwrap_err();
        assert!(matches!(err, Sig9421Error::InvalidSignature));
    }

    #[tokio::test]
    async fn stale_timestamp_rejected() {
        let (kp, pubkey) = signing_keypair();
        let mut resolver = StaticKeyResolver::new();
        resolver.insert("key:agent-1", pubkey);
        // Very short TTL to make the test deterministic.
        let verifier = Sig9421Verifier::with_ttl(Arc::new(resolver), Duration::from_secs(2));

        let mut env = build_envelope();
        // Backdate the envelope by an hour.
        env["ts"] = serde_json::json!((Utc::now() - chrono::Duration::hours(1)).to_rfc3339());
        sign_envelope(&mut env, "key:agent-1", &kp);

        let err = verifier.verify(&env).await.unwrap_err();
        assert!(matches!(err, Sig9421Error::StaleTimestamp(_)));
    }

    #[tokio::test]
    async fn missing_signature_field_returns_typed_error() {
        let verifier = Sig9421Verifier::new(Arc::new(StaticKeyResolver::new()));
        let env = build_envelope();
        let err = verifier.verify(&env).await.unwrap_err();
        assert!(matches!(err, Sig9421Error::MissingSignature));
    }

    #[test]
    fn jcs_sorts_object_keys() {
        let v = serde_json::json!({ "z": 1, "a": 2, "m": 3 });
        assert_eq!(jcs_canonicalize(&v), r#"{"a":2,"m":3,"z":1}"#);
    }

    #[test]
    fn jcs_escapes_strings() {
        let v = serde_json::json!("a\"b\\c\n");
        assert_eq!(jcs_canonicalize(&v), r#""a\"b\\c\n""#);
    }

    #[test]
    fn jcs_handles_nested() {
        let v = serde_json::json!({ "b": [1, 2, { "y": "z", "x": "w" }], "a": null });
        // Sub-object keys also sorted.
        assert_eq!(
            jcs_canonicalize(&v),
            r#"{"a":null,"b":[1,2,{"x":"w","y":"z"}]}"#
        );
    }

    // Force the `SecureRandom` trait bound import to be exercised so
    // we don't get an unused-import warning on it.
    #[test]
    fn rng_bound_exists() {
        let rng = SystemRandom::new();
        let mut buf = [0u8; 4];
        rng.fill(&mut buf).unwrap();
        assert_ne!(buf, [0u8; 4]);
    }
}
