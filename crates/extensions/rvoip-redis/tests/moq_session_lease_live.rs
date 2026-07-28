#![cfg(feature = "moq")]

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Duration, Utc};
use rvoip_core_traits::PrincipalOwnershipKey;
use rvoip_moq::{
    MoqNamespace, MoqSessionId, MoqSessionLease, MoqSessionLeaseBinding, MoqSessionLeaseClose,
    MoqSessionLeaseError, MoqSessionLeaseStore,
};
use rvoip_redis::{RedisMoqSessionLeaseConfig, RedisMoqSessionLeaseStore};

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn redis_url() -> Option<String> {
    std::env::var("RVOIP_REDIS_URL").ok()
}

fn unique_namespace(test: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("test clock must follow the Unix epoch")
        .as_nanos();
    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!(
        "rvoip:test:moq:{test}:{}:{nanos}:{sequence}",
        std::process::id()
    )
}

fn store(url: String, test: &str, tenant_limit: usize) -> RedisMoqSessionLeaseStore {
    RedisMoqSessionLeaseStore::from_config(
        RedisMoqSessionLeaseConfig::new(url)
            .with_namespace(unique_namespace(test))
            .with_max_active_sessions_per_tenant(tenant_limit),
    )
    .expect("live test store configuration must be valid")
}

fn binding(
    tenant: &str,
    session: &str,
    fingerprint: u8,
    expires_at: chrono::DateTime<Utc>,
) -> MoqSessionLeaseBinding {
    MoqSessionLeaseBinding::new(
        MoqSessionId::new(session).expect("valid test session ID"),
        PrincipalOwnershipKey {
            issuer: Some("https://issuer.test".to_owned()),
            tenant: Some(tenant.to_owned()),
            subject: format!("subject-{tenant}"),
        },
        format!("token-{tenant}-{session}"),
        [fingerprint; 32],
        MoqNamespace::new(tenant, "broadcast").expect("valid test namespace"),
        "broadcast:subscribe:broadcast",
        expires_at,
    )
    .expect("valid test binding")
}

#[tokio::test]
async fn tenant_quota_is_atomic_and_tenants_are_isolated() {
    let Some(url) = redis_url() else {
        return;
    };
    let store = store(url, "quota", 2);
    let now = Utc::now();
    let expiry = now + Duration::minutes(2);

    store
        .acquire(&binding("tenant-a", "session-a1", 1, expiry), now)
        .await
        .expect("first tenant session must acquire");
    store
        .acquire(&binding("tenant-a", "session-a2", 2, expiry), now)
        .await
        .expect("second tenant session must acquire");
    let quota = store
        .acquire(&binding("tenant-a", "session-a3", 3, expiry), now)
        .await
        .expect_err("N+1 session must be rejected");
    assert_eq!(quota, MoqSessionLeaseError::TenantQuotaExceeded);

    store
        .acquire(&binding("tenant-b", "session-b1", 4, expiry), now)
        .await
        .expect("another tenant must have an independent quota");
    let snapshot = store.snapshot(now).await.expect("snapshot must succeed");
    assert_eq!(snapshot.active_sessions, 3);
    assert_eq!(snapshot.tenant_buckets, 2);
    assert_eq!(snapshot.limits.max_active_sessions, usize::MAX);
    assert_eq!(snapshot.limits.max_active_sessions_per_tenant, 2);
}

#[tokio::test]
async fn acquire_retry_is_idempotent_and_cross_session_replay_stays_consumed() {
    let Some(url) = redis_url() else {
        return;
    };
    let store = store(url, "retry-replay", 4);
    let now = Utc::now();
    let expiry = now + Duration::minutes(2);
    let first = binding("tenant-a", "session-one", 9, expiry);

    let lease = store
        .acquire(&first, now)
        .await
        .expect("first acquire must succeed");
    let retried = store
        .acquire(&first, now)
        .await
        .expect("a response-loss retry must be idempotent");
    assert_eq!(retried, lease);

    let replay = store
        .acquire(&binding("tenant-a", "session-two", 9, expiry), now)
        .await
        .expect_err("the same credential cannot move to another session");
    assert_eq!(replay, MoqSessionLeaseError::CrossSessionReplay);

    store
        .close(&lease, MoqSessionLeaseClose::PeerClosed, now)
        .await
        .expect("close must succeed");
    store
        .close(&lease, MoqSessionLeaseClose::PeerClosed, now)
        .await
        .expect("close retry must be idempotent");
    assert_eq!(
        store.verify(&lease, now).await,
        Err(MoqSessionLeaseError::Closed)
    );
    assert_eq!(
        store
            .acquire(&binding("tenant-a", "session-two", 9, expiry), now)
            .await,
        Err(MoqSessionLeaseError::CrossSessionReplay)
    );
}

#[tokio::test]
async fn close_racing_first_acquire_always_leaves_a_tombstone() {
    let Some(url) = redis_url() else {
        return;
    };
    let store = store(url, "close-race", 4);
    let now = Utc::now();
    let binding = binding("tenant-a", "session-race", 12, now + Duration::minutes(2));
    let lease = MoqSessionLease::from_binding(binding.clone());

    let acquire_store = store.clone();
    let close_store = store.clone();
    let acquire_binding = binding.clone();
    let close_lease = lease.clone();
    let (acquire_result, close_result) = tokio::join!(
        acquire_store.acquire(&acquire_binding, now),
        close_store.close(&close_lease, MoqSessionLeaseClose::ActivationFailed, now)
    );
    close_result.expect("close side of race must succeed");
    assert!(matches!(
        acquire_result,
        Ok(_) | Err(MoqSessionLeaseError::Closed)
    ));
    assert_eq!(
        store.verify(&lease, now).await,
        Err(MoqSessionLeaseError::Closed)
    );
    assert_eq!(
        store.acquire(&binding, now).await,
        Err(MoqSessionLeaseError::Closed)
    );
}

#[tokio::test]
async fn redis_ttl_removes_session_token_and_active_index() {
    let Some(url) = redis_url() else {
        return;
    };
    let store = store(url, "ttl", 4);
    let now = Utc::now();
    let binding = binding(
        "tenant-a",
        "session-expiring",
        15,
        now + Duration::milliseconds(350),
    );
    let lease = store
        .acquire(&binding, now)
        .await
        .expect("short lease must acquire");

    tokio::time::sleep(std::time::Duration::from_millis(550)).await;
    let after_expiry = Utc::now();
    assert_eq!(
        store.verify(&lease, after_expiry).await,
        Err(MoqSessionLeaseError::Expired)
    );
    let snapshot = store
        .snapshot(after_expiry)
        .await
        .expect("post-expiry snapshot must succeed");
    assert_eq!(snapshot.retained_sessions, 0);
    assert_eq!(snapshot.retained_tokens, 0);
    assert_eq!(snapshot.active_sessions, 0);
    assert_eq!(snapshot.tenant_buckets, 0);
}
