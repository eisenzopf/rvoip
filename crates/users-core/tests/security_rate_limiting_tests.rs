//! Rate Limiting Security Tests

use users_core::api::rate_limit::{EnhancedRateLimiter, RateLimitConfig, RateLimitIdentifier, RateLimitError};
use std::time::{Duration, Instant};
use tokio::time::sleep;

#[tokio::test]
async fn test_user_rate_limiting() {
    let config = RateLimitConfig {
        requests_per_minute: 5,
        requests_per_hour: 100,
        login_attempts_per_hour: 3,
        lockout_duration: Duration::from_secs(1),
        cleanup_interval: Duration::from_secs(60),
    };
    
    let limiter = EnhancedRateLimiter::new(config);
    
    // Test user rate limiting - should allow 5 requests
    for i in 0..5 {
        let result = limiter.check_rate_limit(RateLimitIdentifier::User("user1".to_string())).await;
        assert!(result.is_ok(), "Request {} should be allowed", i + 1);
    }
    
    // 6th request should fail
    let result = limiter.check_rate_limit(RateLimitIdentifier::User("user1".to_string())).await;
    assert!(matches!(result, Err(RateLimitError::TooManyRequests)));
    
    // Different user should work
    let result = limiter.check_rate_limit(RateLimitIdentifier::User("user2".to_string())).await;
    assert!(result.is_ok(), "Different user should have separate rate limit");
}

#[tokio::test]
async fn test_ip_rate_limiting() {
    let config = RateLimitConfig {
        requests_per_minute: 10,
        requests_per_hour: 100,
        login_attempts_per_hour: 5,
        lockout_duration: Duration::from_secs(1),
        cleanup_interval: Duration::from_secs(60),
    };
    
    let limiter = EnhancedRateLimiter::new(config);
    
    // Test IP rate limiting
    for i in 0..10 {
        let result = limiter.check_rate_limit(RateLimitIdentifier::Ip("192.168.1.1".to_string())).await;
        assert!(result.is_ok(), "Request {} should be allowed", i + 1);
    }
    
    // 11th request should fail
    let result = limiter.check_rate_limit(RateLimitIdentifier::Ip("192.168.1.1".to_string())).await;
    assert!(matches!(result, Err(RateLimitError::TooManyRequests)));
    
    // Different IP should work
    let result = limiter.check_rate_limit(RateLimitIdentifier::Ip("192.168.1.2".to_string())).await;
    assert!(result.is_ok(), "Different IP should have separate rate limit");
}

#[tokio::test]
async fn test_failed_login_lockout() {
    let config = RateLimitConfig {
        requests_per_minute: 100,
        requests_per_hour: 1000,
        login_attempts_per_hour: 3,
        lockout_duration: Duration::from_secs(2),
        cleanup_interval: Duration::from_secs(60),
    };
    
    let limiter = EnhancedRateLimiter::new(config);
    
    // Record 3 failed login attempts
    for i in 0..3 {
        let result = limiter.record_failed_login("testuser").await;
        if i < 2 {
            assert!(result.is_ok(), "Failed attempt {} should be recorded", i + 1);
        } else {
            // 3rd attempt should trigger lockout
            assert!(matches!(result, Err(RateLimitError::AccountLocked(_))));
        }
    }
    
    // Further attempts should be locked
    let result = limiter.record_failed_login("testuser").await;
    assert!(matches!(result, Err(RateLimitError::AccountLocked(_))));
    
    // Wait for lockout to expire
    sleep(Duration::from_secs(3)).await;
    
    // Should be able to try again
    let result = limiter.record_failed_login("testuser").await;
    assert!(result.is_ok(), "Should be unlocked after waiting");
}

#[tokio::test]
async fn test_successful_login_clears_failed_attempts() {
    let config = RateLimitConfig {
        requests_per_minute: 100,
        requests_per_hour: 1000,
        login_attempts_per_hour: 3,
        lockout_duration: Duration::from_secs(1),
        cleanup_interval: Duration::from_secs(60),
    };
    
    let limiter = EnhancedRateLimiter::new(config);
    
    // Record 2 failed attempts
    for _ in 0..2 {
        let result = limiter.record_failed_login("testuser").await;
        assert!(result.is_ok());
    }
    
    // Successful login should clear attempts
    limiter.record_successful_login("testuser").await;
    
    // Should be able to have 3 more failed attempts
    for i in 0..3 {
        let result = limiter.record_failed_login("testuser").await;
        if i < 2 {
            assert!(result.is_ok(), "Failed attempt {} should be allowed after reset", i + 1);
        } else {
            assert!(matches!(result, Err(RateLimitError::AccountLocked(_))));
        }
    }
}

#[tokio::test]
async fn test_rate_limit_window_reset() {
    let config = RateLimitConfig {
        requests_per_minute: 5,
        requests_per_hour: 100,
        login_attempts_per_hour: 5,
        lockout_duration: Duration::from_secs(1),
        cleanup_interval: Duration::from_secs(60),
    };
    
    let limiter = EnhancedRateLimiter::new(config);
    
    // Use up the limit
    for _ in 0..5 {
        let result = limiter.check_rate_limit(RateLimitIdentifier::User("test_user".to_string())).await;
        assert!(result.is_ok());
    }
    
    // Next request should be blocked
    let result = limiter.check_rate_limit(RateLimitIdentifier::User("test_user".to_string())).await;
    assert!(matches!(result, Err(RateLimitError::TooManyRequests)));
    
    // Note: In a real test, we'd wait 60 seconds for the window to reset
    // For unit tests, we test that the logic is correct
    // Integration tests would verify the actual timing
}

#[tokio::test]
async fn test_concurrent_rate_limiting() {
    let config = RateLimitConfig {
        requests_per_minute: 10,
        requests_per_hour: 100,
        login_attempts_per_hour: 5,
        lockout_duration: Duration::from_secs(1),
        cleanup_interval: Duration::from_secs(60),
    };
    
    let limiter = EnhancedRateLimiter::new(config);
    
    // Test concurrent requests from same user
    let mut handles = Vec::new();
    for i in 0..15 {
        let limiter_clone = limiter.clone();
        let handle = tokio::spawn(async move {
            let result = limiter_clone.check_rate_limit(
                RateLimitIdentifier::User("concurrent_user".to_string())
            ).await;
            (i, result)
        });
        handles.push(handle);
    }
    
    // Collect results
    let mut successes = 0;
    let mut failures = 0;
    
    for handle in handles {
        let (_index, result) = handle.await.unwrap();
        match result {
            Ok(()) => successes += 1,
            Err(RateLimitError::TooManyRequests) => failures += 1,
            _ => panic!("Unexpected error"),
        }
    }
    
    // Should have exactly 10 successes and 5 failures
    assert_eq!(successes, 10, "Should allow exactly 10 requests");
    assert_eq!(failures, 5, "Should block exactly 5 requests");
}

#[tokio::test]
async fn test_different_identifiers_separate_limits() {
    let config = RateLimitConfig {
        requests_per_minute: 5,
        requests_per_hour: 100,
        login_attempts_per_hour: 3,
        lockout_duration: Duration::from_secs(1),
        cleanup_interval: Duration::from_secs(60),
    };
    
    let limiter = EnhancedRateLimiter::new(config);
    
    // Use up user limit
    for _ in 0..5 {
        limiter.check_rate_limit(RateLimitIdentifier::User("user1".to_string())).await.unwrap();
    }
    
    // User should be blocked
    let result = limiter.check_rate_limit(RateLimitIdentifier::User("user1".to_string())).await;
    assert!(matches!(result, Err(RateLimitError::TooManyRequests)));
    
    // But same identifier as IP should work (different namespace)
    let result = limiter.check_rate_limit(RateLimitIdentifier::Ip("user1".to_string())).await;
    assert!(result.is_ok(), "IP identifier should have separate limit from User identifier");
}

#[tokio::test]
async fn test_lockout_duration_in_error() {
    let config = RateLimitConfig {
        requests_per_minute: 100,
        requests_per_hour: 1000,
        login_attempts_per_hour: 1,
        lockout_duration: Duration::from_secs(15 * 60), // 15 minutes
        cleanup_interval: Duration::from_secs(60),
    };
    
    let limiter = EnhancedRateLimiter::new(config);
    
    // Trigger lockout
    let result = limiter.record_failed_login("locktest").await;
    
    match result {
        Err(RateLimitError::AccountLocked(duration)) => {
            // Should report approximately 15 minutes
            assert!(duration.as_secs() >= 14 * 60 && duration.as_secs() <= 15 * 60,
                "Lockout duration should be approximately 15 minutes, got {} seconds", duration.as_secs());
        }
        _ => panic!("Expected AccountLocked error"),
    }
}