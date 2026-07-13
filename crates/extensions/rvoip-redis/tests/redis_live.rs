use std::time::{Duration, SystemTime};

use rvoip_auth_core::{
    AuthAttemptAdmission, AuthAuditOutcome, AuthFailureReason, AuthRateLimitKey, AuthRateLimitKind,
    AuthRateLimiter, CredentialAuthError, DigestNonceStatus, DigestReplayStore,
    TokenRevocationChecker, TokenRevocationContext, TokenRevocationStatus,
};
use rvoip_redis::{
    RedisAuthConfig, RedisAuthProvider, RedisAuthRateLimitLimits, RedisDigestReplayLimits,
};

fn live_provider(test_name: &str) -> Option<RedisAuthProvider> {
    let redis_url = std::env::var("RVOIP_REDIS_URL").ok()?;
    let namespace = format!(
        "rvoip:test:{}:{}",
        test_name,
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("system clock must be after the Unix epoch")
            .as_nanos()
    );
    Some(
        RedisAuthProvider::from_config(
            RedisAuthConfig::new(redis_url)
                .with_namespace(namespace)
                .with_nonce_stale_retention(Duration::from_secs(60))
                .with_nonce_count_ttl(Duration::from_secs(60))
                .with_token_revocation_ttl(Duration::from_secs(60))
                .with_rate_limit_window(Duration::from_secs(60))
                .with_max_failures_per_window(2),
        )
        .expect("RVOIP_REDIS_URL must construct a Redis auth provider"),
    )
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
async fn rate_limiter_concurrent_reservations_are_atomic_and_idempotent() {
    let Some(provider) = live_provider("rate_limit_concurrent") else {
        return;
    };
    provider.clear_namespace_for_tests().await.unwrap();

    let key = AuthRateLimitKey::new(AuthRateLimitKind::SipRegister)
        .with_subject("alice")
        .with_realm("pbx.example.test")
        .with_peer("198.51.100.10");
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let provider = provider.clone();
        let key = key.clone();
        tasks.push(tokio::spawn(async move {
            provider.reserve_auth_attempt(&key).await.unwrap()
        }));
    }
    let mut reservations = Vec::new();
    let mut denied = 0;
    for task in tasks {
        match task.await.unwrap() {
            AuthAttemptAdmission::Reserved(reservation) => reservations.push(reservation),
            AuthAttemptAdmission::Denied { .. } => denied += 1,
        }
    }
    assert_eq!(reservations.len(), 2);
    assert_eq!(denied, 6);
    for reservation in &reservations {
        provider
            .complete_auth_attempt(
                reservation,
                &AuthAuditOutcome::Failure(AuthFailureReason::InvalidCredential),
            )
            .await
            .unwrap();
        provider
            .complete_auth_attempt(
                reservation,
                &AuthAuditOutcome::Failure(AuthFailureReason::InvalidCredential),
            )
            .await
            .unwrap();
    }
    assert!(matches!(
        provider.reserve_auth_attempt(&key).await.unwrap(),
        AuthAttemptAdmission::Denied { .. }
    ));

    let success_key = AuthRateLimitKey::new(AuthRateLimitKind::SipRegister)
        .with_subject("successful-user")
        .with_realm("pbx.example.test")
        .with_peer("198.51.100.11");
    let AuthAttemptAdmission::Reserved(success) =
        provider.reserve_auth_attempt(&success_key).await.unwrap()
    else {
        panic!("fresh peer and subject must reserve capacity");
    };
    provider
        .complete_auth_attempt(&success, &AuthAuditOutcome::Success)
        .await
        .unwrap();
    provider
        .complete_auth_attempt(&success, &AuthAuditOutcome::Success)
        .await
        .unwrap();
    assert!(matches!(
        provider.reserve_auth_attempt(&success_key).await.unwrap(),
        AuthAttemptAdmission::Reserved(_)
    ));

    assert!(matches!(
        provider.check_auth_attempt(&key).await,
        Err(CredentialAuthError::PolicyRejected(_))
    ));

    provider.clear_namespace_for_tests().await.unwrap();
}

#[tokio::test]
async fn rate_limiter_aggregates_peers_and_subjects_without_cross_release() {
    let Some(provider) = live_provider("rate_limit_aggregates") else {
        return;
    };
    provider.clear_namespace_for_tests().await.unwrap();

    let peer_key = |subject: &str, realm: &str| {
        AuthRateLimitKey::new(AuthRateLimitKind::SipRegister)
            .with_subject(subject)
            .with_realm(realm)
            .with_peer("198.51.100.20")
    };
    let AuthAttemptAdmission::Reserved(peer_failure) = provider
        .reserve_auth_attempt(&peer_key("alice", "pbx-a.example.test"))
        .await
        .unwrap()
    else {
        panic!("first peer aggregate attempt must reserve");
    };
    provider
        .complete_auth_attempt(
            &peer_failure,
            &AuthAuditOutcome::Failure(AuthFailureReason::InvalidCredential),
        )
        .await
        .unwrap();
    let AuthAttemptAdmission::Reserved(peer_success) = provider
        .reserve_auth_attempt(&peer_key("bob", "pbx-b.example.test"))
        .await
        .unwrap()
    else {
        panic!("second peer aggregate attempt must reserve");
    };
    provider
        .complete_auth_attempt(&peer_success, &AuthAuditOutcome::Success)
        .await
        .unwrap();
    let AuthAttemptAdmission::Reserved(peer_second_failure) = provider
        .reserve_auth_attempt(&peer_key("carol", "pbx-c.example.test"))
        .await
        .unwrap()
    else {
        panic!("success must release only its own peer reservation");
    };
    provider
        .complete_auth_attempt(
            &peer_second_failure,
            &AuthAuditOutcome::Failure(AuthFailureReason::InvalidCredential),
        )
        .await
        .unwrap();
    assert!(matches!(
        provider
            .reserve_auth_attempt(&peer_key("rotated-user", "rotated-realm.example.test"))
            .await
            .unwrap(),
        AuthAttemptAdmission::Denied { .. }
    ));

    let subject_key = |peer: &str, realm: &str| {
        AuthRateLimitKey::new(AuthRateLimitKind::Digest)
            .with_subject("shared-subject")
            .with_realm(realm)
            .with_peer(peer)
    };
    for (peer, realm) in [
        ("198.51.100.30", "pbx-a.example.test"),
        ("198.51.100.31", "pbx-b.example.test"),
    ] {
        let AuthAttemptAdmission::Reserved(reservation) = provider
            .reserve_auth_attempt(&subject_key(peer, realm))
            .await
            .unwrap()
        else {
            panic!("first two subject aggregate attempts must reserve");
        };
        provider
            .complete_auth_attempt(
                &reservation,
                &AuthAuditOutcome::Failure(AuthFailureReason::InvalidCredential),
            )
            .await
            .unwrap();
    }
    assert!(matches!(
        provider
            .reserve_auth_attempt(&subject_key("198.51.100.32", "rotated-realm.example.test",))
            .await
            .unwrap(),
        AuthAttemptAdmission::Denied { .. }
    ));

    provider.clear_namespace_for_tests().await.unwrap();
}

#[tokio::test]
async fn rate_limiter_bounds_incomplete_reservations_under_concurrency() {
    let Ok(redis_url) = std::env::var("RVOIP_REDIS_URL") else {
        return;
    };
    let namespace = format!(
        "rvoip:test:rate-reservations:{}",
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let provider = RedisAuthProvider::from_config(
        RedisAuthConfig::new(redis_url)
            .with_namespace(namespace)
            .with_rate_limit_window(Duration::from_secs(60))
            .with_max_failures_per_window(100),
    )
    .unwrap()
    .with_auth_rate_limit_limits(RedisAuthRateLimitLimits {
        peer_cohorts: 16,
        subject_cohorts: 16,
        reservations: 2,
    })
    .unwrap();
    provider.clear_namespace_for_tests().await.unwrap();

    let mut tasks = Vec::new();
    for index in 0..16 {
        let provider = provider.clone();
        tasks.push(tokio::spawn(async move {
            let key = AuthRateLimitKey::new(AuthRateLimitKind::BearerToken)
                .with_subject(format!("subject-{index}"))
                .with_realm("tenant-a")
                .with_peer(format!("198.51.100.{index}"));
            provider.reserve_auth_attempt(&key).await.unwrap()
        }));
    }
    let mut reservations = Vec::new();
    let mut denied = 0;
    for task in tasks {
        match task.await.unwrap() {
            AuthAttemptAdmission::Reserved(reservation) => reservations.push(reservation),
            AuthAttemptAdmission::Denied { .. } => denied += 1,
        }
    }
    assert_eq!(reservations.len(), 2);
    assert_eq!(denied, 14);
    assert_eq!(
        provider
            .auth_rate_limit_cardinality()
            .await
            .unwrap()
            .reservations,
        2
    );
    for reservation in reservations {
        provider
            .complete_auth_attempt(&reservation, &AuthAuditOutcome::Success)
            .await
            .unwrap();
    }
    assert_eq!(
        provider
            .auth_rate_limit_cardinality()
            .await
            .unwrap()
            .reservations,
        0
    );

    provider.clear_namespace_for_tests().await.unwrap();
}

#[tokio::test]
async fn expired_completion_cannot_release_a_newer_peer_or_subject_cohort() {
    let Ok(redis_url) = std::env::var("RVOIP_REDIS_URL") else {
        return;
    };
    let namespace = format!(
        "rvoip:test:rate-stale-completion:{}",
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let limits = RedisAuthRateLimitLimits {
        peer_cohorts: 512,
        subject_cohorts: 512,
        reservations: 512,
    };
    let provider_with_window = |window| {
        RedisAuthProvider::from_config(
            RedisAuthConfig::new(redis_url.clone())
                .with_namespace(namespace.clone())
                .with_rate_limit_window(window)
                .with_max_failures_per_window(1),
        )
        .unwrap()
        .with_auth_rate_limit_limits(limits)
        .unwrap()
    };
    let target_provider = provider_with_window(Duration::from_secs(3));
    let filler_provider = provider_with_window(Duration::from_secs(1));
    target_provider.clear_namespace_for_tests().await.unwrap();

    let target_key = AuthRateLimitKey::new(AuthRateLimitKind::Digest)
        .with_subject("target-subject")
        .with_realm("tenant-a")
        .with_peer("198.51.100.200");
    let AuthAttemptAdmission::Reserved(stale) = target_provider
        .reserve_auth_attempt(&target_key)
        .await
        .unwrap()
    else {
        panic!("target reservation must be admitted");
    };

    // More than one cleanup batch expires before the target. This leaves the
    // target's reservation record behind while a new cohort with the same
    // peer/subject is admitted, reproducing the dangerous stale-completion
    // ordering deterministically.
    let mut fillers = Vec::new();
    for index in 0..300 {
        let provider = filler_provider.clone();
        fillers.push(tokio::spawn(async move {
            let key = AuthRateLimitKey::new(AuthRateLimitKind::Digest)
                .with_subject(format!("filler-subject-{index}"))
                .with_realm("tenant-a")
                .with_peer(format!("198.51.{}.{}", 101 + index / 250, index % 250));
            provider.reserve_auth_attempt(&key).await.unwrap()
        }));
    }
    for filler in fillers {
        assert!(matches!(
            filler.await.unwrap(),
            AuthAttemptAdmission::Reserved(_)
        ));
    }
    tokio::time::sleep(Duration::from_secs(4)).await;

    let AuthAttemptAdmission::Reserved(current) = target_provider
        .reserve_auth_attempt(&target_key)
        .await
        .unwrap()
    else {
        panic!("expired target cohort must admit a new reservation");
    };
    target_provider
        .complete_auth_attempt(&stale, &AuthAuditOutcome::Success)
        .await
        .unwrap();
    assert!(matches!(
        target_provider
            .reserve_auth_attempt(&target_key)
            .await
            .unwrap(),
        AuthAttemptAdmission::Denied { .. }
    ));
    target_provider
        .complete_auth_attempt(&current, &AuthAuditOutcome::Success)
        .await
        .unwrap();

    target_provider.clear_namespace_for_tests().await.unwrap();
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

    for _ in 0..2 {
        let AuthAttemptAdmission::Reserved(reservation) =
            provider.reserve_auth_attempt(&key).await.unwrap()
        else {
            panic!("first two attempts must reserve capacity");
        };
        provider
            .complete_auth_attempt(
                &reservation,
                &AuthAuditOutcome::Failure(AuthFailureReason::InvalidCredential),
            )
            .await
            .unwrap();
    }
    assert!(matches!(
        provider.reserve_auth_attempt(&key).await.unwrap(),
        AuthAttemptAdmission::Denied { .. }
    ));

    provider.clear_namespace_for_tests().await.unwrap();
}

#[tokio::test]
async fn rate_limiter_bounds_rotating_peer_and_subject_cohorts() {
    let Ok(redis_url) = std::env::var("RVOIP_REDIS_URL") else {
        return;
    };
    let namespace = format!(
        "rvoip:test:rate-cardinality:{}",
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let provider = RedisAuthProvider::from_config(
        RedisAuthConfig::new(redis_url)
            .with_namespace(namespace)
            .with_rate_limit_window(Duration::from_secs(60))
            .with_max_failures_per_window(100),
    )
    .unwrap()
    .with_auth_rate_limit_limits(RedisAuthRateLimitLimits {
        peer_cohorts: 3,
        subject_cohorts: 4,
        reservations: 8,
    })
    .unwrap();
    provider.clear_namespace_for_tests().await.unwrap();

    for index in 0..128 {
        let key = AuthRateLimitKey::new(AuthRateLimitKind::SipRegister)
            .with_subject(format!("rotating-user-{index}"))
            .with_realm("pbx.example.test")
            .with_peer(format!("198.51.100.{}", index % 250));
        if let AuthAttemptAdmission::Reserved(reservation) =
            provider.reserve_auth_attempt(&key).await.unwrap()
        {
            provider
                .complete_auth_attempt(
                    &reservation,
                    &AuthAuditOutcome::Failure(AuthFailureReason::InvalidCredential),
                )
                .await
                .unwrap();
        }
    }

    let cardinality = provider.auth_rate_limit_cardinality().await.unwrap();
    assert!(cardinality.peer_cohorts <= 3, "{cardinality:?}");
    assert!(cardinality.subject_cohorts <= 4, "{cardinality:?}");
    assert_eq!(cardinality.reservations, 0, "{cardinality:?}");
    provider.clear_namespace_for_tests().await.unwrap();
}
