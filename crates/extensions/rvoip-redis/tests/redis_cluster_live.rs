use std::time::{Duration, SystemTime};

use rvoip_auth_core::{
    AuthAuditOutcome, AuthFailureReason, AuthRateLimitKey, AuthRateLimitKind, AuthRateLimitVerdict,
    AuthRateLimiter, DigestNonceStatus, DigestReplayStore, TokenRevocationChecker,
    TokenRevocationContext, TokenRevocationStatus,
};
use rvoip_redis::{RedisAuthConfig, RedisAuthConnectionMode, RedisAuthProvider};

fn cluster_provider(test_name: &str) -> Option<RedisAuthProvider> {
    let seed_urls = std::env::var("RVOIP_REDIS_CLUSTER_URLS")
        .ok()?
        .split(',')
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if seed_urls.is_empty() {
        return None;
    }
    let namespace = format!(
        "rvoip:cluster-test:{test_name}:{}",
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()?
            .as_nanos()
    );
    RedisAuthProvider::from_cluster_config(
        RedisAuthConfig::new(seed_urls[0].clone())
            .with_namespace(namespace)
            .with_nonce_stale_retention(Duration::from_secs(30))
            .with_nonce_count_ttl(Duration::from_secs(30))
            .with_token_revocation_ttl(Duration::from_secs(30))
            .with_rate_limit_window(Duration::from_secs(30))
            .with_max_failures_per_window(1),
        seed_urls,
    )
    .ok()
}

#[tokio::test]
async fn redis_cluster_routes_digest_lua_and_single_key_auth_state() {
    let Some(provider) = cluster_provider("auth-state") else {
        return;
    };
    assert_eq!(provider.connection_mode(), RedisAuthConnectionMode::Cluster);

    let nonce = provider
        .admit_nonce("cluster-nonce", SystemTime::now() + Duration::from_secs(20))
        .await
        .unwrap();
    assert_eq!(nonce, "cluster-nonce");
    assert_eq!(
        provider
            .nonce_status(&nonce, SystemTime::now())
            .await
            .unwrap(),
        DigestNonceStatus::Active
    );
    assert!(provider
        .accept_nonce_count("alice", &nonce, "legacy-client", 1)
        .await
        .unwrap());
    assert!(!provider
        .accept_nonce_count("alice", &nonce, "legacy-client", 1)
        .await
        .unwrap());
    assert!(provider
        .accept_nonce_count("alice", &nonce, "other-client", 1)
        .await
        .unwrap());
    assert!(provider
        .accept_client_nonce_count("alice", &nonce, "secure-client", 1, SystemTime::now())
        .await
        .unwrap());

    // Exercise independently hash-tagged tenant namespaces so the test
    // traverses redirects and multiple primaries instead of proving Lua
    // execution for only one cluster slot.
    for shard in 0..12 {
        let shard_provider = cluster_provider(&format!("digest-slot-{shard}"))
            .expect("cluster seed configuration remains available");
        let shard_nonce = format!("cluster-nonce-{shard}");
        let admitted = shard_provider
            .admit_nonce(&shard_nonce, SystemTime::now() + Duration::from_secs(20))
            .await
            .unwrap();
        assert_eq!(admitted, shard_nonce);
        assert!(shard_provider
            .accept_client_nonce_count("alice", &admitted, "secure-client", 1, SystemTime::now(),)
            .await
            .unwrap());
    }

    let token =
        TokenRevocationContext::new("cluster-token").with_issuer("https://issuer.example.test");
    assert_eq!(
        provider.check_token(&token).await.unwrap(),
        TokenRevocationStatus::Active
    );
    provider.revoke_token(&token).await.unwrap();
    assert_eq!(
        provider.check_token(&token).await.unwrap(),
        TokenRevocationStatus::Revoked
    );

    let rate_key = AuthRateLimitKey::new(AuthRateLimitKind::SipRegister)
        .with_subject("alice")
        .with_realm("pbx.example.test")
        .with_peer("198.51.100.42");
    assert_eq!(
        provider.check_auth_attempt(&rate_key).await.unwrap(),
        AuthRateLimitVerdict::Allowed
    );
    provider
        .record_auth_result(
            &rate_key,
            &AuthAuditOutcome::Failure(AuthFailureReason::InvalidCredential),
        )
        .await
        .unwrap();
    assert!(matches!(
        provider.check_auth_attempt(&rate_key).await.unwrap(),
        AuthRateLimitVerdict::Denied { .. }
    ));
}
