//! Optional Redis-backed enterprise auth hooks example.
//!
//! Run with:
//!
//!   RVOIP_REDIS_URL=redis://127.0.0.1:6379 \
//!     cargo run -p rvoip-sip --example auth_redis_enterprise_hooks

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use rvoip_redis::{RedisAuthConfig, RedisAuthProvider};
use rvoip_sip::{
    AuthAttemptAdmission, AuthAuditOutcome, AuthFailureReason, AuthRateLimitKey, AuthRateLimitKind,
    AuthRateLimiter, CredentialAuthError, DigestAlgorithm, DigestAuth, DigestAuthenticator,
    DigestSecret, DigestSecretProvider, SipAuthDecision, SipAuthScheme, SipAuthService,
    SipAuthSource,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let Some(redis_url) = std::env::var("RVOIP_REDIS_URL").ok() else {
        println!("Skipping Redis example; set RVOIP_REDIS_URL.");
        println!("Local fixture hint: cd ~/Developer/redis && docker compose up -d");
        return Ok(());
    };

    let redis = Arc::new(RedisAuthProvider::from_config(
        RedisAuthConfig::new(redis_url)
            .with_namespace("rvoip:example:auth")
            .with_rate_limit_window(Duration::from_secs(60))
            .with_max_failures_per_window(2)
            .with_max_initial_challenges_per_window(120),
    )?);
    redis.clear_namespace_for_tests().await?;

    let auth = SipAuthService::new()
        .with_digest_provider("pbx.example.com", Arc::new(StaticDigestProvider))
        .with_digest_provider_algorithm(DigestAlgorithm::SHA256)
        .with_digest_replay_store(redis.clone())
        .with_rate_limiter(redis.clone());

    let rate_key = AuthRateLimitKey::new(AuthRateLimitKind::SipRegister)
        .with_subject("1001")
        .with_realm("pbx.example.com")
        .with_peer("198.51.100.10");
    let reservation = match redis.reserve_auth_attempt(&rate_key).await? {
        AuthAttemptAdmission::Reserved(reservation) => reservation,
        AuthAttemptAdmission::Denied { retry_after } => {
            println!("Redis denied the example attempt for {retry_after:?}");
            return Ok(());
        }
    };
    redis
        .complete_auth_attempt(
            &reservation,
            &AuthAuditOutcome::Failure(AuthFailureReason::InvalidCredential),
        )
        .await?;
    println!("Redis atomically reserved and recorded one failed attempt");

    redis
        .revoke_token_id(
            "example-jti",
            Some(SystemTime::now() + Duration::from_secs(300)),
        )
        .await?;
    println!("Redis token revocation marker written for example-jti");

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
    let header = DigestAuth::format_authorization_with_state(
        "1001",
        &parsed,
        "sip:pbx.example.com",
        &computed,
    );

    match auth
        .authenticate_authorization(
            Some(&header),
            "REGISTER",
            "sip:pbx.example.com",
            None,
            SipAuthSource::Origin,
            true,
        )
        .await?
    {
        SipAuthDecision::Authorized(identity) => {
            println!("Digest authorized through Redis replay store: {identity:?}");
        }
        SipAuthDecision::Rejected { challenges } => {
            println!("Digest rejected with {} challenges", challenges.len());
        }
    }

    Ok(())
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
