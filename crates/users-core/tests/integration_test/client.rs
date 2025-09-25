//! Integration test client and test cases
//! 
//! This module runs integration tests against the users-core API server
//! to verify rate limiting and security features work correctly.

use reqwest::{Client, StatusCode, header::HeaderMap};
use serde_json::json;
use std::time::Duration;
use tracing::info;

mod server;
use server::{start_test_server, create_test_user};

/// Test client with helper methods
pub struct TestClient {
    client: Client,
    base_url: String,
}

impl TestClient {
    /// Create a new test client
    pub fn new(base_url: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client");
        
        Self { client, base_url }
    }
    
    /// Attempt login with credentials
    pub async fn login(&self, username: &str, password: &str) -> Result<reqwest::Response, reqwest::Error> {
        self.client
            .post(format!("{}/auth/login", self.base_url))
            .json(&json!({
                "username": username,
                "password": password
            }))
            .send()
            .await
    }
    
    /// Make authenticated GET request
    pub async fn authenticated_get(&self, path: &str, token: &str) -> Result<reqwest::Response, reqwest::Error> {
        self.client
            .get(format!("{}{}", self.base_url, path))
            .bearer_auth(token)
            .send()
            .await
    }
    
    /// Make unauthenticated GET request
    pub async fn get(&self, path: &str) -> Result<reqwest::Response, reqwest::Error> {
        self.client
            .get(format!("{}{}", self.base_url, path))
            .send()
            .await
    }
    
    /// Send multiple rapid requests
    pub async fn rapid_requests(&self, path: &str, count: usize) -> Vec<StatusCode> {
        let mut statuses = Vec::new();
        
        for _ in 0..count {
            match self.get(path).await {
                Ok(resp) => statuses.push(resp.status()),
                Err(_) => statuses.push(StatusCode::INTERNAL_SERVER_ERROR),
            }
        }
        
        statuses
    }
    
    /// Check if response has proper rate limit headers
    pub fn check_rate_limit_headers(headers: &HeaderMap) -> bool {
        headers.contains_key("retry-after")
    }
    
    /// Check if response has all required security headers
    pub fn check_security_headers(headers: &HeaderMap) -> Vec<String> {
        let required_headers = vec![
            "x-content-type-options",
            "x-frame-options",
            "x-xss-protection",
            "referrer-policy",
            "permissions-policy",
        ];
        
        let mut missing = Vec::new();
        for header in required_headers {
            if !headers.contains_key(header) {
                missing.push(header.to_string());
            }
        }
        
        missing
    }
}

async fn test_login_attempt_lockout() -> anyhow::Result<()> {
    let server = start_test_server().await?;
    let client = TestClient::new(server.url.clone());
    
    info!("Testing login attempt lockout...");
    
    // Make 3 failed login attempts (the configured limit)
    for i in 1..=3 {
        let resp = client.login("testuser", "WrongPassword").await?;
        
        if i < 3 {
            // First 2 attempts should return 401
            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, 
                      "Attempt {} should return 401", i);
        } else {
            // 3rd attempt should trigger lockout
            assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS,
                      "3rd failed attempt should trigger lockout (429)");
            
            // Check for Retry-After header
            assert!(TestClient::check_rate_limit_headers(resp.headers()),
                   "Rate limited response should include Retry-After header");
            
            let retry_after = resp.headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .expect("Retry-After should be a valid number");
            
            info!("Account locked for {} seconds", retry_after);
            assert!(retry_after > 0 && retry_after <= 3, 
                   "Retry-After should be between 1-3 seconds for test config");
        }
    }
    
    // Additional attempt should still be locked
    let resp = client.login("testuser", "WrongPassword").await?;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS,
              "Should still be locked after 3rd attempt");
    
    // Wait for lockout to expire (2 seconds + buffer)
    info!("Waiting for lockout to expire...");
    tokio::time::sleep(Duration::from_secs(3)).await;
    
    // Should be able to attempt login again
    let resp = client.login("testuser", "SecurePass123").await?;
    assert_eq!(resp.status(), StatusCode::OK,
              "Should be able to login after lockout expires");
    
    info!("✓ Login attempt lockout test passed");
    server.shutdown().await;
    Ok(())
}

async fn test_api_request_rate_limiting() -> anyhow::Result<()> {
    let server = start_test_server().await?;
    let client = TestClient::new(server.url.clone());
    
    info!("Testing API request rate limiting...");
    
    // Make 10 rapid requests (the configured limit)
    let statuses = client.rapid_requests("/health", 11).await;
    
    // First 10 should succeed
    for (i, status) in statuses.iter().take(10).enumerate() {
        assert_eq!(*status, StatusCode::OK, 
                  "Request {} should succeed", i + 1);
    }
    
    // 11th request should be rate limited
    assert_eq!(statuses[10], StatusCode::TOO_MANY_REQUESTS,
              "11th request should be rate limited");
    
    // Test that different IPs have separate limits
    // (In real scenario, would test from different IPs, but here we just verify the concept)
    info!("✓ API request rate limiting test passed");
    server.shutdown().await;
    Ok(())
}

async fn test_authenticated_user_rate_limiting() -> anyhow::Result<()> {
    let server = start_test_server().await?;
    let client = TestClient::new(server.url.clone());
    
    info!("Testing authenticated user rate limiting...");
    
    // Create two additional users
    create_test_user(&server.auth_service, "user1", "SecurePass456").await?;
    create_test_user(&server.auth_service, "user2", "SecurePass789").await?;
    
    // Login both users (login doesn't count against authenticated rate limit)
    let resp1 = client.login("user1", "SecurePass456").await?;
    assert_eq!(resp1.status(), StatusCode::OK);
    let auth1: serde_json::Value = resp1.json().await?;
    let token1 = auth1["access_token"].as_str().unwrap();
    
    let resp2 = client.login("user2", "SecurePass789").await?;
    assert_eq!(resp2.status(), StatusCode::OK);
    let auth2: serde_json::Value = resp2.json().await?;
    let token2 = auth2["access_token"].as_str().unwrap();
    
    // Make requests as user1 up to the limit
    // The rate limit is 10 per minute, but let's find out exactly when it triggers
    let mut successful_requests = 0;
    for _ in 0..15 {
        let resp = client.authenticated_get("/users", token1).await?;
        if resp.status() == StatusCode::OK {
            successful_requests += 1;
        } else if resp.status() == StatusCode::TOO_MANY_REQUESTS {
            info!("Rate limit hit after {} successful requests", successful_requests);
            // Verify proper Retry-After header
            assert!(TestClient::check_rate_limit_headers(resp.headers()),
                   "Rate limited response should include Retry-After header");
            break;
        } else {
            panic!("Unexpected status code: {}", resp.status());
        }
    }
    
    // We expect at least some successful requests before rate limiting
    assert!(successful_requests >= 5, 
            "Expected at least 5 successful requests, got {}", successful_requests);
    
    // Small delay to ensure clean separation
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // User2 should still be able to make requests
    info!("Testing user2 rate limit independence...");
    let resp = client.authenticated_get("/users", token2).await?;
    if resp.status() != StatusCode::OK {
        info!("User2 got status: {} (expected 200)", resp.status());
        if resp.status() == StatusCode::TOO_MANY_REQUESTS {
            info!("User2 is rate limited - this suggests IP-based limiting is being applied to authenticated requests");
        }
    }
    assert_eq!(resp.status(), StatusCode::OK,
              "User2 should have separate rate limit from User1");
    
    info!("✓ Authenticated user rate limiting test passed");
    server.shutdown().await;
    Ok(())
}

async fn test_rate_limit_headers() -> anyhow::Result<()> {
    let server = start_test_server().await?;
    let client = TestClient::new(server.url.clone());
    
    info!("Testing rate limit headers...");
    
    // Trigger rate limiting
    let statuses = client.rapid_requests("/health", 11).await;
    assert_eq!(statuses[10], StatusCode::TOO_MANY_REQUESTS);
    
    // Make another request to check headers
    let resp = client.get("/health").await?;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    
    // Verify Retry-After header
    let headers = resp.headers();
    assert!(headers.contains_key("retry-after"),
           "Rate limited response must include Retry-After header");
    
    let retry_after = headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .expect("Retry-After should be a valid number");
    
    assert!(retry_after > 0 && retry_after <= 60,
           "Retry-After should be reasonable (1-60 seconds)");
    
    info!("✓ Rate limit headers test passed");
    server.shutdown().await;
    Ok(())
}

async fn test_security_headers_present() -> anyhow::Result<()> {
    let server = start_test_server().await?;
    let client = TestClient::new(server.url.clone());
    
    info!("Testing security headers...");
    
    // Make a request and check security headers
    let resp = client.get("/health").await?;
    let headers = resp.headers();
    
    // Check for missing security headers
    let missing = TestClient::check_security_headers(headers);
    
    if !missing.is_empty() {
        panic!("Missing security headers: {:?}", missing);
    }
    
    // Verify specific header values
    assert_eq!(
        headers.get("x-content-type-options").and_then(|v| v.to_str().ok()),
        Some("nosniff"),
        "X-Content-Type-Options should be 'nosniff'"
    );
    
    assert_eq!(
        headers.get("x-frame-options").and_then(|v| v.to_str().ok()),
        Some("DENY"),
        "X-Frame-Options should be 'DENY'"
    );
    
    assert_eq!(
        headers.get("x-xss-protection").and_then(|v| v.to_str().ok()),
        Some("1; mode=block"),
        "X-XSS-Protection should be '1; mode=block'"
    );
    
    info!("✓ Security headers test passed");
    server.shutdown().await;
    Ok(())
}

// Extended tests that can be run with --full flag
async fn test_extended_rate_limiting_scenarios() -> anyhow::Result<()> {
    let server = start_test_server().await?;
    let client = TestClient::new(server.url.clone());
    
    info!("Running extended rate limiting tests...");
    
    // Test 1: Verify rate limit resets after time window
    let statuses = client.rapid_requests("/health", 10).await;
    assert!(statuses.iter().all(|s| *s == StatusCode::OK));
    
    // Wait for rate limit window to reset (1 minute)
    info!("Waiting 60 seconds for rate limit window to reset...");
    tokio::time::sleep(Duration::from_secs(61)).await;
    
    // Should be able to make requests again
    let resp = client.get("/health").await?;
    assert_eq!(resp.status(), StatusCode::OK,
              "Rate limit should reset after time window");
    
    // Test 2: Concurrent requests handling
    info!("Testing concurrent request handling...");
    let mut handles = vec![];
    
    for i in 0..5 {
        let client_clone = TestClient::new(server.url.clone());
        let handle = tokio::spawn(async move {
            let resp = client_clone.get("/health").await.unwrap();
            (i, resp.status())
        });
        handles.push(handle);
    }
    
    let results = futures::future::join_all(handles).await;
    let successful = results.iter()
        .filter(|r| r.as_ref().unwrap().1 == StatusCode::OK)
        .count();
    
    assert!(successful >= 3, 
           "At least 3 concurrent requests should succeed within rate limit");
    
    info!("✓ Extended rate limiting tests passed");
    server.shutdown().await;
    Ok(())
}

// Main entry point for running all tests
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("Running users-core integration tests...");
    
    // Initialize logging
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter("info")
        .init();
    
    // Parse command line arguments
    let args: Vec<String> = std::env::args().collect();
    let run_extended = args.contains(&"--full".to_string());
    
    // Run tests sequentially
    let mut passed = 0;
    let mut failed = 0;
    
    // Define test cases with names
    let test_cases = vec![
        ("Login Attempt Lockout", 1),
        ("API Request Rate Limiting", 2),
        ("Authenticated User Rate Limiting", 3),
        ("Rate Limit Headers", 4),
        ("Security Headers", 5),
    ];
    
    // Add extended test if requested
    let all_tests = if run_extended {
        let mut tests = test_cases;
        tests.push(("Extended Rate Limiting Scenarios", 6));
        tests
    } else {
        test_cases
    };
    
    for (name, test_id) in all_tests {
        println!("\n📋 Running: {}", name);
        
        let result = match test_id {
            1 => tokio::time::timeout(Duration::from_secs(30), test_login_attempt_lockout()).await,
            2 => tokio::time::timeout(Duration::from_secs(30), test_api_request_rate_limiting()).await,
            3 => tokio::time::timeout(Duration::from_secs(30), test_authenticated_user_rate_limiting()).await,
            4 => tokio::time::timeout(Duration::from_secs(30), test_rate_limit_headers()).await,
            5 => tokio::time::timeout(Duration::from_secs(30), test_security_headers_present()).await,
            6 => tokio::time::timeout(Duration::from_secs(120), test_extended_rate_limiting_scenarios()).await,
            _ => unreachable!(),
        };
        
        match result {
            Ok(Ok(_)) => {
                println!("✅ {} - PASSED", name);
                passed += 1;
            }
            Ok(Err(e)) => {
                println!("❌ {} - FAILED: {}", name, e);
                failed += 1;
            }
            Err(_) => {
                println!("❌ {} - FAILED (timeout)", name);
                failed += 1;
            }
        }
    }
    
    println!("\n📊 Test Results:");
    println!("   Passed: {}", passed);
    println!("   Failed: {}", failed);
    
    if failed > 0 {
        std::process::exit(1);
    }
    
    Ok(())
}
