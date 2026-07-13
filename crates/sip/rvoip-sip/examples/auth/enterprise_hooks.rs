//! Enterprise auth hooks example.
//!
//! Shows the provider contracts an enterprise deployment usually wires into
//! UAS authentication:
//!
//! - redacted audit events through `AuthAuditSink`;
//! - rate-limit / lockout checks through `AuthRateLimiter`;
//! - shared SIP Digest nonce and nonce-count replay state through
//!   `DigestReplayStore`.
//!
//! This example keeps the SIP transport out of the way so the hook behavior is
//! easy to read. The same `SipAuthService` can be passed to
//! `IncomingRegister::authenticate_with`, `IncomingRequest::authenticate_with`,
//! or `IncomingCall::authenticate_with`.
//!
//! Run with:
//!
//!   cargo run -p rvoip-sip --example auth_enterprise_hooks

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

use rvoip_sip::{
    AuthAuditEvent, AuthAuditOutcome, AuthAuditSink, AuthRateLimitKey, AuthRateLimitVerdict,
    AuthRateLimiter, CredentialAuthError, DigestAlgorithm, DigestAuth, DigestAuthenticator,
    DigestNonceStatus, DigestReplayStore, DigestSecret, DigestSecretProvider, SipAuthDecision,
    SipAuthScheme, SipAuthService, SipAuthSource,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let audit = Arc::new(PrintAuditSink);
    let rate_limiter = Arc::new(AllowingRateLimiter);
    let replay_store = Arc::new(MemoryDigestReplayStore::default());

    let mut auth = SipAuthService::new()
        .with_basic_realm("legacy")
        .allow_basic_over_cleartext(true)
        .with_digest_provider("pbx.example.com", Arc::new(StaticDigestProvider))
        .with_digest_provider_algorithm(DigestAlgorithm::SHA256)
        .with_digest_replay_store(replay_store)
        .with_audit_sink(audit)
        .with_rate_limiter(rate_limiter);
    auth.add_basic_user("alice", "SecurePass2024");

    let basic = BASE64_STANDARD.encode("alice:SecurePass2024");
    let decision = auth
        .authenticate_authorization(
            Some(&format!("Basic {basic}")),
            "REGISTER",
            "sip:pbx.example.com",
            None,
            SipAuthSource::Origin,
            false,
        )
        .await?;
    print_decision("Basic", decision);

    let challenge = auth
        .challenges_async(SipAuthSource::Origin)
        .await?
        .into_iter()
        .find(|challenge| challenge.scheme == SipAuthScheme::Digest)
        .expect("Digest challenge");
    let parsed = DigestAuthenticator::parse_challenge(&challenge.value)?;
    let computed = DigestAuth::compute_response_with_state(
        "1001",
        "sip-secret",
        &parsed,
        "REGISTER",
        "sip:pbx.example.com",
        1,
        None,
    )?;
    let digest_header = DigestAuth::format_authorization_with_state(
        "1001",
        &parsed,
        "sip:pbx.example.com",
        &computed,
    );
    let decision = auth
        .authenticate_authorization(
            Some(&digest_header),
            "REGISTER",
            "sip:pbx.example.com",
            None,
            SipAuthSource::Origin,
            true,
        )
        .await?;
    print_decision("Digest", decision);

    let replay = auth
        .authenticate_authorization(
            Some(&digest_header),
            "REGISTER",
            "sip:pbx.example.com",
            None,
            SipAuthSource::Origin,
            true,
        )
        .await?;
    print_decision("Digest replay", replay);

    Ok(())
}

fn print_decision(label: &str, decision: SipAuthDecision) {
    match decision {
        SipAuthDecision::Authorized(identity) => {
            println!(
                "{label}: authorized scheme={:?} username={:?} subject={:?}",
                identity.scheme, identity.username, identity.subject
            );
        }
        SipAuthDecision::Rejected { challenges } => {
            println!("{label}: rejected with {} challenge(s)", challenges.len());
        }
    }
}

struct PrintAuditSink;

#[async_trait]
impl AuthAuditSink for PrintAuditSink {
    async fn record_auth_event(
        &self,
        event: AuthAuditEvent,
    ) -> std::result::Result<(), CredentialAuthError> {
        println!(
            "[audit] scheme={:?} outcome={:?} subject={:?} realm={:?} peer={:?}",
            event.scheme, event.outcome, event.subject, event.realm, event.peer
        );
        Ok(())
    }
}

struct AllowingRateLimiter;

#[async_trait]
impl AuthRateLimiter for AllowingRateLimiter {
    async fn check_auth_attempt(
        &self,
        key: &AuthRateLimitKey,
    ) -> std::result::Result<AuthRateLimitVerdict, CredentialAuthError> {
        println!(
            "[rate-limit] check kind={:?} subject={:?} realm={:?}",
            key.kind, key.subject, key.realm
        );
        Ok(AuthRateLimitVerdict::Allowed)
    }

    async fn record_auth_result(
        &self,
        _key: &AuthRateLimitKey,
        outcome: &AuthAuditOutcome,
    ) -> std::result::Result<(), CredentialAuthError> {
        println!("[rate-limit] outcome={outcome:?}");
        Ok(())
    }
}

struct StaticDigestProvider;

#[async_trait]
impl DigestSecretProvider for StaticDigestProvider {
    async fn lookup_digest_secret(
        &self,
        username: &str,
        realm: &str,
        _algorithm: DigestAlgorithm,
    ) -> std::result::Result<Option<DigestSecret>, CredentialAuthError> {
        if username == "1001" && realm == "pbx.example.com" {
            Ok(Some(DigestSecret::PlaintextPassword("sip-secret".into())))
        } else {
            Ok(None)
        }
    }
}

#[derive(Default)]
struct MemoryDigestReplayStore {
    nonces: Mutex<HashMap<String, SystemTime>>,
    nonce_counts: Mutex<HashMap<(String, String, String), u32>>,
}

const MAX_EXAMPLE_NONCES: usize = 4_096;
const MAX_EXAMPLE_SEQUENCES: usize = 16_384;
const MAX_EXAMPLE_SEQUENCES_PER_USERNAME: usize = 4_096;

#[async_trait]
impl DigestReplayStore for MemoryDigestReplayStore {
    async fn record_nonce(
        &self,
        nonce: &str,
        expires_at: SystemTime,
    ) -> std::result::Result<(), CredentialAuthError> {
        let admitted = self.admit_nonce(nonce, expires_at).await?;
        if admitted == nonce {
            Ok(())
        } else {
            Err(CredentialAuthError::PolicyRejected(
                "legacy nonce admission reached capacity".to_string(),
            ))
        }
    }

    async fn nonce_status(
        &self,
        nonce: &str,
        now: SystemTime,
    ) -> std::result::Result<DigestNonceStatus, CredentialAuthError> {
        match self.nonces.lock().unwrap().get(nonce).copied() {
            Some(expires_at) if expires_at > now => Ok(DigestNonceStatus::Active),
            Some(_) => Ok(DigestNonceStatus::Expired),
            None => Ok(DigestNonceStatus::Unknown),
        }
    }

    async fn accept_nonce_count(
        &self,
        username: &str,
        nonce: &str,
        cnonce: &str,
        nonce_count: u32,
    ) -> std::result::Result<bool, CredentialAuthError> {
        self.accept_client_nonce_count(username, nonce, cnonce, nonce_count, SystemTime::now())
            .await
    }

    async fn admit_nonce(
        &self,
        proposed_nonce: &str,
        expires_at: SystemTime,
    ) -> std::result::Result<String, CredentialAuthError> {
        let now = SystemTime::now();
        let mut nonces = self.nonces.lock().unwrap();
        let expired = nonces
            .iter()
            .filter(|(_, expiry)| **expiry <= now)
            .map(|(nonce, _)| nonce.clone())
            .collect::<HashSet<_>>();
        nonces.retain(|nonce, _| !expired.contains(nonce));
        let admitted = if nonces.contains_key(proposed_nonce) {
            nonces.insert(proposed_nonce.to_string(), expires_at);
            proposed_nonce.to_string()
        } else if nonces.len() >= MAX_EXAMPLE_NONCES {
            nonces
                .iter()
                .max_by_key(|(_, expiry)| **expiry)
                .map(|(nonce, _)| nonce.clone())
                .ok_or_else(|| {
                    CredentialAuthError::PolicyRejected(
                        "Digest nonce admission unavailable".to_string(),
                    )
                })?
        } else {
            nonces.insert(proposed_nonce.to_string(), expires_at);
            proposed_nonce.to_string()
        };
        drop(nonces);
        if !expired.is_empty() {
            self.nonce_counts
                .lock()
                .unwrap()
                .retain(|(_, nonce, _), _| !expired.contains(nonce));
        }
        Ok(admitted)
    }

    async fn accept_client_nonce_count(
        &self,
        username: &str,
        nonce: &str,
        cnonce: &str,
        nonce_count: u32,
        now: SystemTime,
    ) -> std::result::Result<bool, CredentialAuthError> {
        if self.nonce_status(nonce, now).await? != DigestNonceStatus::Active {
            return Ok(false);
        }
        let key = (username.to_string(), nonce.to_string(), cnonce.to_string());
        let mut counts = self.nonce_counts.lock().unwrap();
        if let Some(previous) = counts.get_mut(&key) {
            if nonce_count <= *previous {
                println!("[replay] rejected nonce-count replay for {username}");
                return Ok(false);
            }
            *previous = nonce_count;
            return Ok(true);
        }
        let username_sequences = counts
            .keys()
            .filter(|(recorded, _, _)| recorded == username)
            .count();
        if counts.len() >= MAX_EXAMPLE_SEQUENCES
            || username_sequences >= MAX_EXAMPLE_SEQUENCES_PER_USERNAME
        {
            return Err(CredentialAuthError::PolicyRejected(
                "Digest replay capacity exhausted".to_string(),
            ));
        }
        counts.insert(key, nonce_count);
        Ok(true)
    }
}
