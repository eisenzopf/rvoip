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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use chrono::{DateTime, Utc};
use ring::signature::{ED25519, UnparsedPublicKey};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Maximum age of a signed envelope's `ts` field. Envelopes older
/// than this are rejected as stale per spec §5.5.1. Default matches
/// the spec's 5-minute window.
pub const DEFAULT_SIG_REPLAY_TTL: Duration = Duration::from_secs(300);

/// How far ahead of local time a signed envelope's `ts` may sit before it is
/// rejected as unusable.
///
/// A freshness window bounded only from below is not a freshness window: a
/// far-future `ts` produces a negative age, passes an `age > ttl` test, and
/// stays valid for as long as the sender chose — which is exactly the
/// property a replay window exists to deny. Real senders drift by seconds;
/// this allows for that and nothing more.
pub const DEFAULT_SIG_CLOCK_SKEW: Duration = Duration::from_secs(30);

/// Maximum number of envelope IDs the replay cache holds before
/// LRU eviction. Mirrors [`crate::dpop::DEFAULT_JTI_CACHE_CAPACITY`].
pub const DEFAULT_REPLAY_CACHE_CAPACITY: u64 = 100_000;

/// Maximum canonicalized envelope size accepted at the signature boundary.
pub const DEFAULT_MAX_SIGNED_ENVELOPE_BYTES: usize = 1024 * 1024;

pub enum Sig9421Error {
    MissingSignature,
    MalformedSignature(String),
    UnsupportedAlgorithm(String),
    UnknownKeyid(String),
    InvalidSignature,
    ReplayDetected(String),
    StaleTimestamp(String),
    MalformedEnvelope,
    MissingEnvelopeId,
    InvalidEnvelopeTimestamp,
    MissingVerificationContext,
    KeyRejected,
    ReplayStoreUnavailable,
    EnvelopeTooLarge,
}

impl Sig9421Error {
    fn diagnostic_class(&self) -> &'static str {
        match self {
            Self::MissingSignature => "missing-signature",
            Self::MalformedSignature(_) => "malformed-signature",
            Self::UnsupportedAlgorithm(_) => "unsupported-algorithm",
            Self::UnknownKeyid(_) => "unknown-key",
            Self::InvalidSignature => "invalid-signature",
            Self::ReplayDetected(_) => "replay",
            Self::StaleTimestamp(_) => "stale-timestamp",
            Self::MalformedEnvelope => "malformed-envelope",
            Self::MissingEnvelopeId => "missing-envelope-id",
            Self::InvalidEnvelopeTimestamp => "invalid-envelope-timestamp",
            Self::MissingVerificationContext => "missing-verification-context",
            Self::KeyRejected => "key-rejected",
            Self::ReplayStoreUnavailable => "replay-store-unavailable",
            Self::EnvelopeTooLarge => "envelope-too-large",
        }
    }
}

impl fmt::Display for Sig9421Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "message signature validation failed (class={})",
            self.diagnostic_class()
        )
    }
}

impl fmt::Debug for Sig9421Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Sig9421Error")
            .field("class", &self.diagnostic_class())
            .finish()
    }
}

impl std::error::Error for Sig9421Error {}

/// Inline `signature` field on a signed envelope. See
/// CONVERSATION_PROTOCOL.md §5.5.1.
#[derive(Clone, Serialize, Deserialize)]
pub struct EnvelopeSignature {
    pub keyid: String,
    /// JWA algorithm name (e.g. `"EdDSA"`, `"ES256"`).
    pub alg: String,
    /// Base64url-encoded (no padding) signature bytes.
    pub sig: String,
}

impl fmt::Debug for EnvelopeSignature {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnvelopeSignature")
            .field("key_id_present", &!self.keyid.is_empty())
            .field("algorithm_present", &!self.alg.is_empty())
            .field("signature_present", &!self.sig.is_empty())
            .field("signature_bytes", &self.sig.len())
            .finish()
    }
}

/// Ownership boundary supplied by the authenticated transport before key
/// resolution. A bare key ID is not globally meaningful in a multi-tenant
/// deployment and must never select a key across tenant or issuer domains.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SignatureVerificationContext {
    pub tenant: String,
    pub issuer: String,
}

impl SignatureVerificationContext {
    pub fn new(tenant: impl Into<String>, issuer: impl Into<String>) -> Result<Self, Sig9421Error> {
        let context = Self {
            tenant: tenant.into(),
            issuer: issuer.into(),
        };
        if context.tenant.is_empty()
            || context.issuer.is_empty()
            || context.tenant.len() > 256
            || context.issuer.len() > 2048
        {
            return Err(Sig9421Error::MissingVerificationContext);
        }
        Ok(context)
    }

    fn replay_namespace(&self) -> String {
        format!("{}\u{1f}{}", self.tenant, self.issuer)
    }
}

/// Contextual key selected for one signature operation.
#[derive(Clone)]
pub struct ResolvedVerificationKey {
    pub public_key: Vec<u8>,
    pub algorithm: String,
    pub not_before: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked: bool,
}

impl fmt::Debug for ResolvedVerificationKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedVerificationKey")
            .field("public_key_bytes", &self.public_key.len())
            .field("algorithm", &self.algorithm)
            .field("not_before", &self.not_before)
            .field("expires_at", &self.expires_at)
            .field("revoked", &self.revoked)
            .finish()
    }
}

/// Atomic replay-consumption result. Store failures are distinct from a
/// duplicate and fail closed at the verifier boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayConsumeError {
    AlreadyConsumed,
    CapacityExhausted,
    Unavailable,
}

#[async_trait]
pub trait SignatureReplayStore: Send + Sync {
    async fn consume(
        &self,
        context: &SignatureVerificationContext,
        envelope_id: &str,
        expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<(), ReplayConsumeError>;
}

/// Bounded, fail-closed replay store for one-process deployments and tests.
/// The mutex covers lookup and insertion as one operation, so concurrent
/// identical envelopes have exactly one winner.
pub struct InMemorySignatureReplayStore {
    capacity: usize,
    entries: Mutex<HashMap<(String, String), DateTime<Utc>>>,
}

impl InMemorySignatureReplayStore {
    pub fn new(capacity: usize) -> Result<Self, ReplayConsumeError> {
        if capacity == 0 {
            return Err(ReplayConsumeError::CapacityExhausted);
        }
        Ok(Self {
            capacity,
            entries: Mutex::new(HashMap::new()),
        })
    }
}

#[async_trait]
impl SignatureReplayStore for InMemorySignatureReplayStore {
    async fn consume(
        &self,
        context: &SignatureVerificationContext,
        envelope_id: &str,
        expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<(), ReplayConsumeError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| ReplayConsumeError::Unavailable)?;
        entries.retain(|_, expiry| *expiry > now);
        let key = (context.replay_namespace(), envelope_id.to_string());
        if entries.contains_key(&key) {
            return Err(ReplayConsumeError::AlreadyConsumed);
        }
        if entries.len() >= self.capacity {
            return Err(ReplayConsumeError::CapacityExhausted);
        }
        entries.insert(key, expires_at);
        Ok(())
    }
}

pub trait SignatureClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Default)]
pub struct SystemSignatureClock;

impl SignatureClock for SystemSignatureClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Trait the verifier uses to look up the public key bytes for a
/// `keyid`. Production deployments back this with their identity
/// store; tests typically use [`StaticKeyResolver`].
pub trait KeyResolver: Send + Sync {
    /// Returns the raw public key bytes for `keyid`, or `None` if
    /// the keyid is unknown. For Ed25519 the slice is the 32-byte
    /// raw public key.
    fn resolve(&self, keyid: &str) -> Option<Vec<u8>>;

    /// Resolve a key inside an authenticated tenant/issuer boundary. The
    /// compatibility default wraps a legacy global key, while production
    /// resolvers should override this method and reject unknown contexts.
    fn resolve_contextual(
        &self,
        _context: &SignatureVerificationContext,
        keyid: &str,
        algorithm: &str,
        _now: DateTime<Utc>,
    ) -> Option<ResolvedVerificationKey> {
        self.resolve(keyid)
            .map(|public_key| ResolvedVerificationKey {
                public_key,
                algorithm: algorithm.to_string(),
                not_before: None,
                expires_at: None,
                revoked: false,
            })
    }
}

/// In-memory key resolver — useful for tests and static deployments.
pub struct StaticKeyResolver {
    keys: std::collections::HashMap<String, Vec<u8>>,
    contextual_keys: std::collections::HashMap<(String, String, String), ResolvedVerificationKey>,
}

impl StaticKeyResolver {
    pub fn new() -> Self {
        Self {
            keys: std::collections::HashMap::new(),
            contextual_keys: std::collections::HashMap::new(),
        }
    }

    pub fn insert(&mut self, keyid: impl Into<String>, public_key: Vec<u8>) {
        self.keys.insert(keyid.into(), public_key);
    }

    pub fn insert_contextual(
        &mut self,
        context: &SignatureVerificationContext,
        keyid: impl Into<String>,
        key: ResolvedVerificationKey,
    ) {
        self.contextual_keys.insert(
            (context.tenant.clone(), context.issuer.clone(), keyid.into()),
            key,
        );
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

    fn resolve_contextual(
        &self,
        context: &SignatureVerificationContext,
        keyid: &str,
        algorithm: &str,
        _now: DateTime<Utc>,
    ) -> Option<ResolvedVerificationKey> {
        self.contextual_keys
            .get(&(
                context.tenant.clone(),
                context.issuer.clone(),
                keyid.to_string(),
            ))
            .cloned()
            .or_else(|| {
                <Self as KeyResolver>::resolve(self, keyid).map(|public_key| {
                    ResolvedVerificationKey {
                        public_key,
                        algorithm: algorithm.to_string(),
                        not_before: None,
                        expires_at: None,
                        revoked: false,
                    }
                })
            })
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
    replay_store: Arc<dyn SignatureReplayStore>,
    clock: Arc<dyn SignatureClock>,
    ttl: Duration,
    clock_skew: Duration,
    max_envelope_bytes: usize,
}

impl Sig9421Verifier {
    pub fn new(resolver: Arc<dyn KeyResolver>) -> Self {
        Self::with_ttl(resolver, DEFAULT_SIG_REPLAY_TTL)
    }

    pub fn with_ttl(resolver: Arc<dyn KeyResolver>, ttl: Duration) -> Self {
        Self::with_ttl_and_skew(resolver, ttl, DEFAULT_SIG_CLOCK_SKEW)
    }

    /// Freshness window with an explicit future tolerance. Deployments whose
    /// senders are tightly synchronized can narrow the skew; nothing should
    /// widen it far, because the skew is exactly how long a forged future
    /// timestamp stays usable.
    pub fn with_ttl_and_skew(
        resolver: Arc<dyn KeyResolver>,
        ttl: Duration,
        clock_skew: Duration,
    ) -> Self {
        Self {
            resolver,
            replay_store: Arc::new(
                InMemorySignatureReplayStore::new(DEFAULT_REPLAY_CACHE_CAPACITY as usize)
                    .expect("non-zero default replay capacity"),
            ),
            clock: Arc::new(SystemSignatureClock),
            ttl,
            clock_skew,
            max_envelope_bytes: DEFAULT_MAX_SIGNED_ENVELOPE_BYTES,
        }
    }

    /// Construct the production verification boundary with an externally
    /// durable atomic replay store and an injectable clock.
    pub fn with_security_dependencies(
        resolver: Arc<dyn KeyResolver>,
        replay_store: Arc<dyn SignatureReplayStore>,
        clock: Arc<dyn SignatureClock>,
        ttl: Duration,
        clock_skew: Duration,
        max_envelope_bytes: usize,
    ) -> Result<Self, Sig9421Error> {
        if max_envelope_bytes == 0 {
            return Err(Sig9421Error::EnvelopeTooLarge);
        }
        Ok(Self {
            resolver,
            replay_store,
            clock,
            ttl,
            clock_skew,
            max_envelope_bytes,
        })
    }

    /// Verify an inline-signed envelope. `envelope` is the parsed
    /// JSON value (as it arrived on the wire — typically via
    /// `serde_json::from_str`). On success the envelope's id is
    /// added to the replay cache so subsequent calls with the same
    /// id are rejected.
    pub async fn verify(&self, envelope: &serde_json::Value) -> Result<(), Sig9421Error> {
        let context = SignatureVerificationContext::new("legacy", "legacy")?;
        self.verify_with_context(envelope, &context).await
    }

    /// Verify an inline-signed envelope within its authenticated ownership
    /// domain. This is the production entry point for multi-tenant callers.
    pub async fn verify_with_context(
        &self,
        envelope: &serde_json::Value,
        context: &SignatureVerificationContext,
    ) -> Result<(), Sig9421Error> {
        if context.tenant.is_empty() || context.issuer.is_empty() {
            return Err(Sig9421Error::MissingVerificationContext);
        }
        let obj = envelope
            .as_object()
            .ok_or(Sig9421Error::MalformedEnvelope)?;

        // 1. Pull the signature field.
        let sig_value = obj.get("signature").ok_or(Sig9421Error::MissingSignature)?;
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
        if env_id.is_empty() || env_id.len() > 256 {
            return Err(Sig9421Error::MissingEnvelopeId);
        }
        let now = self.clock.now();
        let age = now.signed_duration_since(env_ts);
        if age > chrono::Duration::from_std(self.ttl).unwrap_or(chrono::Duration::seconds(300)) {
            return Err(Sig9421Error::StaleTimestamp(env_ts_str.to_string()));
        }
        // Bound the window from above as well: a negative age means the
        // envelope claims the future, and beyond tolerated skew that claim
        // buys the sender an unbounded validity period.
        let skew = chrono::Duration::from_std(self.clock_skew)
            .unwrap_or_else(|_| chrono::Duration::seconds(30));
        if age < -skew {
            return Err(Sig9421Error::StaleTimestamp(env_ts_str.to_string()));
        }

        // 3. Build the canonicalization base: clone the envelope,
        // strip `signature`, JCS-serialize.
        let mut bare = obj.clone();
        bare.remove("signature");
        let canonical = serde_jcs::to_vec(&serde_json::Value::Object(bare))
            .map_err(|_| Sig9421Error::MalformedEnvelope)?;
        if canonical.len() > self.max_envelope_bytes {
            return Err(Sig9421Error::EnvelopeTooLarge);
        }

        // 4. Resolve the signing key and verify.
        let resolved = self
            .resolver
            .resolve_contextual(context, &signature.keyid, &signature.alg, now)
            .ok_or_else(|| Sig9421Error::UnknownKeyid(signature.keyid.clone()))?;
        if resolved.revoked
            || resolved.algorithm != signature.alg
            || resolved
                .not_before
                .is_some_and(|not_before| now < not_before)
            || resolved
                .expires_at
                .is_some_and(|expires_at| now >= expires_at)
        {
            return Err(Sig9421Error::KeyRejected);
        }
        let sig_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(signature.sig.as_bytes())
            .map_err(|_| Sig9421Error::InvalidSignature)?;

        match signature.alg.as_str() {
            "EdDSA" => {
                let key = UnparsedPublicKey::new(&ED25519, &resolved.public_key);
                key.verify(&canonical, &sig_bytes)
                    .map_err(|_| Sig9421Error::InvalidSignature)?;
            }
            other => return Err(Sig9421Error::UnsupportedAlgorithm(other.to_string())),
        }

        // 5. Replay check after signature passes (don't burn cache
        //    slots on rejected signatures).
        let replay_expiry = now
            + chrono::Duration::from_std(self.ttl)
                .unwrap_or_else(|_| chrono::Duration::seconds(300));
        match self
            .replay_store
            .consume(context, &env_id, replay_expiry, now)
            .await
        {
            Ok(()) => {}
            Err(ReplayConsumeError::AlreadyConsumed) => {
                return Err(Sig9421Error::ReplayDetected(env_id));
            }
            Err(ReplayConsumeError::CapacityExhausted | ReplayConsumeError::Unavailable) => {
                return Err(Sig9421Error::ReplayStoreUnavailable);
            }
        }

        Ok(())
    }
}

/// RFC 8785 JSON Canonical Form serializer backed by `serde_jcs`, including
/// ECMAScript number formatting and UTF-16 object-key ordering.
pub fn jcs_canonicalize(value: &serde_json::Value) -> String {
    serde_jcs::to_string(value).expect("serde_json::Value is valid JCS input")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::rand::{SecureRandom, SystemRandom};
    use ring::signature::{Ed25519KeyPair, KeyPair};

    struct FixedClock(DateTime<Utc>);

    impl SignatureClock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            self.0
        }
    }

    fn context(tenant: &str) -> SignatureVerificationContext {
        SignatureVerificationContext::new(tenant, "https://issuer.example").unwrap()
    }

    fn resolved_key(public_key: Vec<u8>) -> ResolvedVerificationKey {
        ResolvedVerificationKey {
            public_key,
            algorithm: "EdDSA".into(),
            not_before: None,
            expires_at: None,
            revoked: false,
        }
    }

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

    fn sign_envelope(envelope: &mut serde_json::Value, keyid: &str, kp: &Ed25519KeyPair) {
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
    async fn concurrent_replay_consumption_has_exactly_one_winner() {
        let (kp, pubkey) = signing_keypair();
        let mut resolver = StaticKeyResolver::new();
        resolver.insert("key:agent-1", pubkey);
        let verifier = Arc::new(Sig9421Verifier::new(Arc::new(resolver)));
        let mut env = build_envelope();
        sign_envelope(&mut env, "key:agent-1", &kp);
        let env = Arc::new(env);

        let mut tasks = Vec::new();
        for _ in 0..32 {
            let verifier = Arc::clone(&verifier);
            let env = Arc::clone(&env);
            tasks.push(tokio::spawn(async move { verifier.verify(&env).await }));
        }
        let mut accepted = 0;
        let mut replayed = 0;
        for task in tasks {
            match task.await.unwrap() {
                Ok(()) => accepted += 1,
                Err(Sig9421Error::ReplayDetected(_)) => replayed += 1,
                Err(error) => panic!("unexpected verification result: {error:?}"),
            }
        }
        assert_eq!(accepted, 1);
        assert_eq!(replayed, 31);
    }

    #[tokio::test]
    async fn contextual_resolution_prevents_cross_tenant_key_id_collision() {
        let (kp_a, pubkey_a) = signing_keypair();
        let (_kp_b, pubkey_b) = signing_keypair();
        let tenant_a = context("tenant-a");
        let tenant_b = context("tenant-b");
        let mut resolver = StaticKeyResolver::new();
        resolver.insert_contextual(&tenant_a, "shared-key-id", resolved_key(pubkey_a));
        resolver.insert_contextual(&tenant_b, "shared-key-id", resolved_key(pubkey_b));
        let verifier = Sig9421Verifier::new(Arc::new(resolver));
        let mut env = build_envelope();
        sign_envelope(&mut env, "shared-key-id", &kp_a);

        verifier
            .verify_with_context(&env, &tenant_a)
            .await
            .expect("tenant-a key verifies");
        assert!(matches!(
            verifier.verify_with_context(&env, &tenant_b).await,
            Err(Sig9421Error::InvalidSignature)
        ));
    }

    #[tokio::test]
    async fn revoked_and_expired_contextual_keys_fail_closed() {
        let (kp, pubkey) = signing_keypair();
        let context = context("tenant-a");
        let now = Utc::now();
        let mut key = resolved_key(pubkey);
        key.revoked = true;
        key.expires_at = Some(now - chrono::Duration::seconds(1));
        let mut resolver = StaticKeyResolver::new();
        resolver.insert_contextual(&context, "revoked-key", key);
        let verifier = Sig9421Verifier::with_security_dependencies(
            Arc::new(resolver),
            Arc::new(InMemorySignatureReplayStore::new(8).unwrap()),
            Arc::new(FixedClock(now)),
            DEFAULT_SIG_REPLAY_TTL,
            DEFAULT_SIG_CLOCK_SKEW,
            DEFAULT_MAX_SIGNED_ENVELOPE_BYTES,
        )
        .unwrap();
        let mut env = build_envelope();
        env["ts"] = serde_json::json!(now.to_rfc3339());
        sign_envelope(&mut env, "revoked-key", &kp);

        assert!(matches!(
            verifier.verify_with_context(&env, &context).await,
            Err(Sig9421Error::KeyRejected)
        ));
    }

    #[tokio::test]
    async fn replay_capacity_pressure_fails_closed_without_evicting_live_ids() {
        let now = Utc::now();
        let store = InMemorySignatureReplayStore::new(1).unwrap();
        let context = context("tenant-a");
        let expiry = now + chrono::Duration::minutes(5);
        store.consume(&context, "first", expiry, now).await.unwrap();
        assert_eq!(
            store.consume(&context, "second", expiry, now).await,
            Err(ReplayConsumeError::CapacityExhausted)
        );
        assert_eq!(
            store.consume(&context, "first", expiry, now).await,
            Err(ReplayConsumeError::AlreadyConsumed)
        );
    }

    #[tokio::test]
    async fn shared_store_rejects_replay_across_two_verifier_instances() {
        let (kp, pubkey) = signing_keypair();
        let mut resolver = StaticKeyResolver::new();
        resolver.insert("key:agent-1", pubkey);
        let resolver: Arc<dyn KeyResolver> = Arc::new(resolver);
        let replay_store: Arc<dyn SignatureReplayStore> =
            Arc::new(InMemorySignatureReplayStore::new(8).unwrap());
        let now = Utc::now();
        let make_verifier = || {
            Sig9421Verifier::with_security_dependencies(
                Arc::clone(&resolver),
                Arc::clone(&replay_store),
                Arc::new(FixedClock(now)),
                DEFAULT_SIG_REPLAY_TTL,
                DEFAULT_SIG_CLOCK_SKEW,
                DEFAULT_MAX_SIGNED_ENVELOPE_BYTES,
            )
            .unwrap()
        };
        let first_server = make_verifier();
        let second_server = make_verifier();
        let mut env = build_envelope();
        env["ts"] = serde_json::json!(now.to_rfc3339());
        sign_envelope(&mut env, "key:agent-1", &kp);

        first_server
            .verify(&env)
            .await
            .expect("first server accepts");
        assert!(matches!(
            second_server.verify(&env).await,
            Err(Sig9421Error::ReplayDetected(_))
        ));
    }

    #[tokio::test]
    async fn rotated_key_accepts_current_and_rejects_revoked_generation() {
        let (old_kp, old_public_key) = signing_keypair();
        let (current_kp, current_public_key) = signing_keypair();
        let verification_context = context("tenant-a");
        let mut revoked = resolved_key(old_public_key);
        revoked.revoked = true;
        let mut resolver = StaticKeyResolver::new();
        resolver.insert_contextual(&verification_context, "signing-key:old", revoked);
        resolver.insert_contextual(
            &verification_context,
            "signing-key:current",
            resolved_key(current_public_key),
        );
        let verifier = Sig9421Verifier::new(Arc::new(resolver));

        let mut old_envelope = build_envelope();
        sign_envelope(&mut old_envelope, "signing-key:old", &old_kp);
        assert!(matches!(
            verifier
                .verify_with_context(&old_envelope, &verification_context)
                .await,
            Err(Sig9421Error::KeyRejected)
        ));

        let mut current_envelope = build_envelope();
        current_envelope["id"] = serde_json::json!("env_sig_test_current");
        sign_envelope(&mut current_envelope, "signing-key:current", &current_kp);
        verifier
            .verify_with_context(&current_envelope, &verification_context)
            .await
            .expect("current key generation verifies");
    }

    #[tokio::test]
    async fn malformed_and_oversized_envelopes_fail_closed() {
        let verifier = Sig9421Verifier::new(Arc::new(StaticKeyResolver::new()));
        assert!(matches!(
            verifier
                .verify(&serde_json::json!(["not", "an", "object"]))
                .await,
            Err(Sig9421Error::MalformedEnvelope)
        ));

        let (kp, pubkey) = signing_keypair();
        let mut resolver = StaticKeyResolver::new();
        resolver.insert("key:agent-1", pubkey);
        let verifier = Sig9421Verifier::with_security_dependencies(
            Arc::new(resolver),
            Arc::new(InMemorySignatureReplayStore::new(8).unwrap()),
            Arc::new(SystemSignatureClock),
            DEFAULT_SIG_REPLAY_TTL,
            DEFAULT_SIG_CLOCK_SKEW,
            128,
        )
        .unwrap();
        let mut envelope = build_envelope();
        envelope["payload"]["oversized"] = serde_json::json!("x".repeat(1024));
        sign_envelope(&mut envelope, "key:agent-1", &kp);
        assert!(matches!(
            verifier.verify(&envelope).await,
            Err(Sig9421Error::EnvelopeTooLarge)
        ));
    }

    #[tokio::test]
    async fn published_cross_language_fixture_is_byte_exact() {
        const SEED_HEX: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
        const PUBLIC_HEX: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
        const CANONICAL: &str = "{\"id\":\"env_cross_language_1\",\"payload\":{\"a\":2,\"admin\":false,\"emoji\":\"😀\",\"z\":1},\"ts\":\"2030-01-02T03:04:05Z\",\"type\":\"tool.invoke\",\"v\":1}";
        const SIGNATURE: &str = "Mfs4cn9KID7Aj8dleUs_97zDyANiuvMZ6wuvPWv9N4QZxTALhQs7r5G6OIhyMOGPFDsYONwUCcI2sQe5D6gWBg";
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/sig9421-cross-language-v1.json"
        ))
        .unwrap();
        assert_eq!(fixture["private_seed_hex"], SEED_HEX);
        assert_eq!(fixture["public_key_hex"], PUBLIC_HEX);
        assert_eq!(fixture["canonical_utf8"], CANONICAL);
        assert_eq!(fixture["signature_base64url_no_pad"], SIGNATURE);

        let seed = hex::decode(SEED_HEX).unwrap();
        let keypair = Ed25519KeyPair::from_seed_unchecked(&seed).unwrap();
        assert_eq!(hex::encode(keypair.public_key().as_ref()), PUBLIC_HEX);
        let envelope: serde_json::Value = serde_json::from_str(CANONICAL).unwrap();
        assert_eq!(jcs_canonicalize(&envelope), CANONICAL);
        assert_eq!(
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(keypair.sign(CANONICAL.as_bytes()).as_ref()),
            SIGNATURE
        );
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
    async fn future_timestamp_beyond_skew_rejected() {
        let (kp, pubkey) = signing_keypair();
        let mut resolver = StaticKeyResolver::new();
        resolver.insert("key:agent-1", pubkey);
        let verifier = Sig9421Verifier::new(Arc::new(resolver));

        // A far-future `ts` yields a negative age. Bounded only from below,
        // that envelope would verify — and keep verifying for an hour.
        let mut env = build_envelope();
        env["ts"] = serde_json::json!((Utc::now() + chrono::Duration::hours(1)).to_rfc3339());
        sign_envelope(&mut env, "key:agent-1", &kp);

        let err = verifier.verify(&env).await.unwrap_err();
        assert!(matches!(err, Sig9421Error::StaleTimestamp(_)));
    }

    #[tokio::test]
    async fn future_timestamp_within_skew_accepted() {
        let (kp, pubkey) = signing_keypair();
        let mut resolver = StaticKeyResolver::new();
        resolver.insert("key:agent-1", pubkey);
        let verifier = Sig9421Verifier::new(Arc::new(resolver));

        // Ordinary sender drift must still verify.
        let mut env = build_envelope();
        env["ts"] = serde_json::json!((Utc::now() + chrono::Duration::seconds(5)).to_rfc3339());
        sign_envelope(&mut env, "key:agent-1", &kp);

        verifier.verify(&env).await.expect("skew within tolerance");
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

    #[test]
    fn jcs_uses_utf16_key_order_for_non_bmp_names() {
        let value: serde_json::Value =
            serde_json::from_str(r#"{"\ue000":1,"\ud800\udc00":2}"#).expect("valid surrogate pair");
        assert_eq!(jcs_canonicalize(&value), "{\"𐀀\":2,\"\":1}");
    }

    #[test]
    fn jcs_uses_ecmascript_number_serialization() {
        let value: serde_json::Value = serde_json::from_str(
            "[333333333.33333329,1E30,4.50,2e-3,0.000000000000000000000000001]",
        )
        .unwrap();
        assert_eq!(
            jcs_canonicalize(&value),
            "[333333333.3333333,1e+30,4.5,0.002,1e-27]"
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
