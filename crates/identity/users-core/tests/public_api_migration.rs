use std::time::Duration;

use users_core::{
    api::rate_limit::{RateLimitCapacity, RateLimitError, TrustedProxyConfig},
    jwt::{JwtConfig, UserClaims},
};

#[test]
fn users_core_02_constructors_replace_extensible_struct_literals() {
    let config = JwtConfig::new("https://users.example", vec!["rvoip-api".into()])
        .with_token_ttls(Duration::from_secs(60), Duration::from_secs(120))
        .with_algorithm("HS256")
        .with_tenant_id("tenant-a")
        .with_signing_key("test-only-key");
    assert_eq!(config.tenant_id.as_deref(), Some("tenant-a"));

    let claims = UserClaims::new(
        "https://users.example",
        "user-a",
        vec!["rvoip-api".into()],
        2,
        1,
        "token-a",
        "alice",
        "openid",
    )
    .with_email(Some("alice@example.test".into()))
    .with_roles(vec!["user".into()])
    .with_tenant_id("tenant-a");
    assert_eq!(claims.tenant_id.as_deref(), Some("tenant-a"));

    let capacity = RateLimitCapacity::new(1_000, 2_000, 500);
    assert_eq!(capacity.ips, 2_000);
    let proxies = TrustedProxyConfig::new(["192.0.2.0/24", "2001:db8::/32"]);
    assert_eq!(proxies.cidrs.len(), 2);
}

#[test]
fn rate_limit_error_is_consumed_as_non_exhaustive() {
    fn classify(error: RateLimitError) -> &'static str {
        match error {
            RateLimitError::TooManyRequests => "requests",
            RateLimitError::AccountLocked(_) => "lockout",
            _ => "future",
        }
    }

    assert_eq!(classify(RateLimitError::TooManyRequests), "requests");
}
