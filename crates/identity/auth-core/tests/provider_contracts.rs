use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use rvoip_auth_core::{
    AuthAuditEvent, AuthAuditOutcome, AuthAuditScheme, AuthAuditSink, AuthFailureReason,
    AuthRateLimitKey, AuthRateLimitKind, AuthRateLimitVerdict, AuthRateLimiter,
    CredentialAuthError, DigestNonceStatus, DigestReplayStore,
};

#[derive(Default)]
struct MemoryDigestReplayStore {
    nonces: Mutex<HashMap<String, SystemTime>>,
    counts: Mutex<HashMap<(String, String, String), u32>>,
}

#[async_trait::async_trait]
impl DigestReplayStore for MemoryDigestReplayStore {
    async fn record_nonce(
        &self,
        nonce: &str,
        expires_at: SystemTime,
    ) -> Result<(), CredentialAuthError> {
        self.nonces
            .lock()
            .expect("nonce lock")
            .insert(nonce.to_string(), expires_at);
        Ok(())
    }

    async fn nonce_status(
        &self,
        nonce: &str,
        now: SystemTime,
    ) -> Result<DigestNonceStatus, CredentialAuthError> {
        let nonces = self.nonces.lock().expect("nonce lock");
        Ok(match nonces.get(nonce).copied() {
            Some(expires_at) if expires_at > now => DigestNonceStatus::Active,
            Some(_) => DigestNonceStatus::Expired,
            None => DigestNonceStatus::Unknown,
        })
    }

    async fn accept_nonce_count(
        &self,
        username: &str,
        nonce: &str,
        cnonce: &str,
        nonce_count: u32,
    ) -> Result<bool, CredentialAuthError> {
        let key = (username.to_string(), nonce.to_string(), cnonce.to_string());
        let mut counts = self.counts.lock().expect("count lock");
        if counts.get(&key).is_some_and(|last| nonce_count <= *last) {
            return Ok(false);
        }
        counts.insert(key, nonce_count);
        Ok(true)
    }
}

#[derive(Default)]
struct MemoryAuditSink {
    events: Mutex<Vec<AuthAuditEvent>>,
}

#[async_trait::async_trait]
impl AuthAuditSink for MemoryAuditSink {
    async fn record_auth_event(&self, event: AuthAuditEvent) -> Result<(), CredentialAuthError> {
        self.events.lock().expect("audit lock").push(event);
        Ok(())
    }
}

struct DenyRateLimiter;

#[async_trait::async_trait]
impl AuthRateLimiter for DenyRateLimiter {
    async fn check_auth_attempt(
        &self,
        _key: &AuthRateLimitKey,
    ) -> Result<AuthRateLimitVerdict, CredentialAuthError> {
        Ok(AuthRateLimitVerdict::Denied {
            retry_after: Some(Duration::from_secs(30)),
        })
    }

    async fn record_auth_result(
        &self,
        _key: &AuthRateLimitKey,
        _outcome: &AuthAuditOutcome,
    ) -> Result<(), CredentialAuthError> {
        Ok(())
    }
}

#[tokio::test]
async fn digest_replay_store_rejects_same_or_lower_nonce_count() {
    let store = MemoryDigestReplayStore::default();
    let expires_at = SystemTime::now() + Duration::from_secs(60);
    store.record_nonce("n1", expires_at).await.unwrap();

    assert_eq!(
        store.nonce_status("n1", SystemTime::now()).await.unwrap(),
        DigestNonceStatus::Active
    );
    assert!(store
        .accept_nonce_count("alice", "n1", "client-a", 1)
        .await
        .unwrap());
    assert!(!store
        .accept_nonce_count("alice", "n1", "client-a", 1)
        .await
        .unwrap());
    assert!(!store
        .accept_nonce_count("alice", "n1", "client-a", 0)
        .await
        .unwrap());
    assert!(store
        .accept_nonce_count("alice", "n1", "client-a", 2)
        .await
        .unwrap());
    assert!(store
        .accept_nonce_count("alice", "n1", "client-b", 1)
        .await
        .unwrap());
}

#[tokio::test]
async fn audit_sink_records_redacted_events() {
    let sink = MemoryAuditSink::default();
    let event = AuthAuditEvent::new(
        AuthAuditScheme::Bearer,
        AuthAuditOutcome::Failure(AuthFailureReason::TokenRevoked),
    )
    .with_subject("token-jti-123")
    .with_realm("https://idp.example.com")
    .with_peer("192.0.2.10")
    .with_metadata("client_id", "rvoip-sip");

    sink.record_auth_event(event.clone()).await.unwrap();

    assert_eq!(sink.events.lock().expect("audit lock").as_slice(), &[event]);
}

#[tokio::test]
async fn rate_limiter_contract_returns_retry_after() {
    let limiter = DenyRateLimiter;
    let key = AuthRateLimitKey::new(AuthRateLimitKind::SipRegister)
        .with_subject("1001")
        .with_realm("pbx.example.com")
        .with_peer("198.51.100.7");

    let verdict = limiter.check_auth_attempt(&key).await.unwrap();
    assert_eq!(
        verdict,
        AuthRateLimitVerdict::Denied {
            retry_after: Some(Duration::from_secs(30))
        }
    );
}
