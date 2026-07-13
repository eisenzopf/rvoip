use std::time::{Duration, SystemTime};

use rvoip_auth_core::{
    AuthAuditOutcome, AuthFailureReason, AuthRateLimitKey, AuthRateLimitKind, AuthRateLimitVerdict,
    AuthRateLimiter, DigestNonceStatus, DigestReplayStore, TokenRevocationChecker,
    TokenRevocationContext, TokenRevocationStatus,
};
use rvoip_redis::{RedisAuthConfig, RedisAuthProvider, RedisDigestReplayLimits};

fn live_provider(test_name: &str) -> Option<RedisAuthProvider> {
    let redis_url = std::env::var("RVOIP_REDIS_URL").ok()?;
    let namespace = format!(
        "rvoip:test:{}:{}",
        test_name,
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()?
            .as_nanos()
    );
    RedisAuthProvider::from_config(
        RedisAuthConfig::new(redis_url)
            .with_namespace(namespace)
            .with_nonce_stale_retention(Duration::from_secs(60))
            .with_nonce_count_ttl(Duration::from_secs(60))
            .with_token_revocation_ttl(Duration::from_secs(60))
            .with_rate_limit_window(Duration::from_secs(60))
            .with_max_failures_per_window(2),
    )
    .ok()
}

#[tokio::test]
async fn digest_replay_store_round_trips_against_redis() {
    let Some(provider) = live_provider("digest_replay") else {
        return;
    };
    provider.clear_namespace_for_tests().await.unwrap();

    provider
        .record_nonce("nonce-1", SystemTime::now() + Duration::from_secs(30))
        .await
        .unwrap();
    assert_eq!(
        provider
            .nonce_status("nonce-1", SystemTime::now())
            .await
            .unwrap(),
        DigestNonceStatus::Active
    );
    assert!(provider
        .accept_client_nonce_count("alice", "nonce-1", "client-a", 1, SystemTime::now(),)
        .await
        .unwrap());
    assert!(!provider
        .accept_client_nonce_count("alice", "nonce-1", "client-a", 1, SystemTime::now(),)
        .await
        .unwrap());
    assert!(!provider
        .accept_client_nonce_count("alice", "nonce-1", "client-a", 0, SystemTime::now(),)
        .await
        .unwrap());
    assert!(provider
        .accept_client_nonce_count("alice", "nonce-1", "client-a", 2, SystemTime::now(),)
        .await
        .unwrap());
    assert!(provider
        .accept_client_nonce_count("alice", "nonce-1", "client-b", 1, SystemTime::now(),)
        .await
        .unwrap());

    provider
        .record_nonce("nonce-stale", SystemTime::now() - Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(
        provider
            .nonce_status("nonce-stale", SystemTime::now())
            .await
            .unwrap(),
        DigestNonceStatus::Expired
    );
    assert_eq!(
        provider
            .nonce_status("nonce-unknown", SystemTime::now())
            .await
            .unwrap(),
        DigestNonceStatus::Unknown
    );

    provider.clear_namespace_for_tests().await.unwrap();
}

#[tokio::test]
async fn digest_nonce_count_concurrent_replay_allows_only_one_same_count() {
    let Some(provider) = live_provider("digest_replay_concurrent") else {
        return;
    };
    provider.clear_namespace_for_tests().await.unwrap();
    provider
        .record_nonce(
            "nonce-concurrent",
            SystemTime::now() + Duration::from_secs(30),
        )
        .await
        .unwrap();

    let mut tasks = Vec::new();
    for _ in 0..32 {
        let provider = provider.clone();
        tasks.push(tokio::spawn(async move {
            provider
                .accept_client_nonce_count(
                    "alice",
                    "nonce-concurrent",
                    "same-client",
                    1,
                    SystemTime::now(),
                )
                .await
                .unwrap()
        }));
    }

    let mut accepted = 0;
    for task in tasks {
        if task.await.unwrap() {
            accepted += 1;
        }
    }
    assert_eq!(
        accepted, 1,
        "only one concurrent Digest request can consume the same nonce-count"
    );
    assert!(provider
        .accept_client_nonce_count(
            "alice",
            "nonce-concurrent",
            "same-client",
            2,
            SystemTime::now(),
        )
        .await
        .unwrap());
    assert!(!provider
        .accept_client_nonce_count(
            "alice",
            "nonce-concurrent",
            "same-client",
            2,
            SystemTime::now(),
        )
        .await
        .unwrap());

    provider.clear_namespace_for_tests().await.unwrap();
}

#[tokio::test]
async fn digest_admission_and_sequence_limits_are_bounded_and_fair() {
    let Some(provider) = live_provider("digest_limits") else {
        return;
    };
    let provider = provider
        .with_digest_replay_limits(RedisDigestReplayLimits {
            retained_nonces: 2,
            client_sequences: 4,
            sequences_per_username: 2,
            sequences_per_nonce: 4,
            sequences_per_username_nonce: 2,
        })
        .unwrap();
    provider.clear_namespace_for_tests().await.unwrap();
    let expiry = SystemTime::now() + Duration::from_secs(30);
    assert_eq!(
        provider.admit_nonce("nonce-a", expiry).await.unwrap(),
        "nonce-a"
    );
    assert_eq!(
        provider.admit_nonce("nonce-b", expiry).await.unwrap(),
        "nonce-b"
    );
    let reused = provider.admit_nonce("nonce-c", expiry).await.unwrap();
    assert!(reused == "nonce-a" || reused == "nonce-b");
    assert_eq!(
        provider
            .nonce_status("nonce-c", SystemTime::now())
            .await
            .unwrap(),
        DigestNonceStatus::Unknown
    );

    for client in ["client-a", "client-b"] {
        assert!(provider
            .accept_client_nonce_count("noisy-user", &reused, client, 1, SystemTime::now())
            .await
            .unwrap());
    }
    assert!(provider
        .accept_client_nonce_count("noisy-user", &reused, "client-c", 1, SystemTime::now(),)
        .await
        .is_err());
    assert!(provider
        .accept_client_nonce_count("unrelated-user", &reused, "client-c", 1, SystemTime::now(),)
        .await
        .unwrap());

    provider.clear_namespace_for_tests().await.unwrap();
}

#[tokio::test]
async fn digest_replay_ttl_cannot_expire_before_the_nonce() {
    let Ok(redis_url) = std::env::var("RVOIP_REDIS_URL") else {
        return;
    };
    let namespace = format!(
        "rvoip:test:digest_ttl:{}",
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let provider = RedisAuthProvider::from_config(
        RedisAuthConfig::new(redis_url)
            .with_namespace(namespace)
            .with_nonce_stale_retention(Duration::from_secs(5))
            .with_nonce_count_ttl(Duration::from_secs(1)),
    )
    .unwrap();
    provider.clear_namespace_for_tests().await.unwrap();
    let nonce = provider
        .admit_nonce(
            "nonce-longer-than-count-ttl",
            SystemTime::now() + Duration::from_secs(5),
        )
        .await
        .unwrap();
    assert!(provider
        .accept_client_nonce_count("alice", &nonce, "client-a", 1, SystemTime::now())
        .await
        .unwrap());
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!provider
        .accept_client_nonce_count("alice", &nonce, "client-a", 1, SystemTime::now())
        .await
        .unwrap());
    provider.clear_namespace_for_tests().await.unwrap();
}

#[tokio::test]
async fn token_revocation_checker_round_trips_against_redis() {
    let Some(provider) = live_provider("token_revocation") else {
        return;
    };
    provider.clear_namespace_for_tests().await.unwrap();

    let context = TokenRevocationContext::new("token-1").with_issuer("https://idp.example.test");
    assert_eq!(
        provider.check_token(&context).await.unwrap(),
        TokenRevocationStatus::Active
    );

    provider.revoke_token(&context).await.unwrap();
    assert_eq!(
        provider.check_token(&context).await.unwrap(),
        TokenRevocationStatus::Revoked
    );
    assert_eq!(
        provider
            .check_token(&TokenRevocationContext::new("token-2"))
            .await
            .unwrap(),
        TokenRevocationStatus::Active
    );

    provider.clear_namespace_for_tests().await.unwrap();
}

#[tokio::test]
async fn rate_limiter_concurrent_failures_deny_until_success_resets() {
    let Some(provider) = live_provider("rate_limit_concurrent") else {
        return;
    };
    provider.clear_namespace_for_tests().await.unwrap();

    let key = AuthRateLimitKey::new(AuthRateLimitKind::SipRegister)
        .with_subject("alice")
        .with_realm("pbx.example.test")
        .with_peer("198.51.100.10");
    assert_eq!(
        provider.check_auth_attempt(&key).await.unwrap(),
        AuthRateLimitVerdict::Allowed
    );

    let mut tasks = Vec::new();
    for _ in 0..8 {
        let provider = provider.clone();
        let key = key.clone();
        tasks.push(tokio::spawn(async move {
            provider
                .record_auth_result(
                    &key,
                    &AuthAuditOutcome::Failure(AuthFailureReason::InvalidCredential),
                )
                .await
                .unwrap()
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    assert!(matches!(
        provider.check_auth_attempt(&key).await.unwrap(),
        AuthRateLimitVerdict::Denied { .. }
    ));

    provider
        .record_auth_result(&key, &AuthAuditOutcome::Success)
        .await
        .unwrap();
    assert_eq!(
        provider.check_auth_attempt(&key).await.unwrap(),
        AuthRateLimitVerdict::Allowed
    );

    provider.clear_namespace_for_tests().await.unwrap();
}

#[tokio::test]
async fn rate_limiter_denies_after_configured_failures() {
    let Some(provider) = live_provider("rate_limit") else {
        return;
    };
    provider.clear_namespace_for_tests().await.unwrap();

    let key = AuthRateLimitKey::new(AuthRateLimitKind::SipRegister)
        .with_subject("alice")
        .with_realm("pbx.example.test")
        .with_peer("198.51.100.10");

    assert_eq!(
        provider.check_auth_attempt(&key).await.unwrap(),
        AuthRateLimitVerdict::Allowed
    );
    provider
        .record_auth_result(
            &key,
            &AuthAuditOutcome::Failure(AuthFailureReason::InvalidCredential),
        )
        .await
        .unwrap();
    provider
        .record_auth_result(
            &key,
            &AuthAuditOutcome::Failure(AuthFailureReason::InvalidCredential),
        )
        .await
        .unwrap();
    assert!(matches!(
        provider.check_auth_attempt(&key).await.unwrap(),
        AuthRateLimitVerdict::Denied { .. }
    ));

    provider
        .record_auth_result(&key, &AuthAuditOutcome::Success)
        .await
        .unwrap();
    assert_eq!(
        provider.check_auth_attempt(&key).await.unwrap(),
        AuthRateLimitVerdict::Allowed
    );

    provider.clear_namespace_for_tests().await.unwrap();
}
