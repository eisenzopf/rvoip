//! Timing Attack Security Tests

use users_core::{AuthenticationService, CreateUserRequest, UsersConfig};
use std::time::{Duration, Instant};
use tempfile::TempDir;

async fn setup_test_auth_service() -> (AuthenticationService, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
    
    let config = UsersConfig {
        database_url: db_url,
        ..Default::default()
    };
    
    let auth_service = users_core::init(config).await.unwrap();
    (auth_service, temp_dir)
}

#[tokio::test]
async fn test_constant_time_authentication() {
    let (auth_service, _temp_dir) = setup_test_auth_service().await;
    
    // Create a real user
    auth_service.create_user(CreateUserRequest {
        username: "realuser".to_string(),
        password: "RealPassword123!".to_string(),
        email: Some("real@example.com".to_string()),
        display_name: None,
        roles: vec!["user".to_string()],
    }).await.unwrap();
    
    // Test scenarios with expected outcomes
    let long_username = "a".repeat(100);
    let scenarios = vec![
        ("realuser", "RealPassword123!", true),      // Valid credentials
        ("realuser", "WrongPassword123!", false),    // Valid user, wrong password
        ("nonexistent", "AnyPassword123!", false),   // Non-existent user
        ("injection'; --", "hack", false),           // SQL injection attempt
        ("", "", false),                              // Empty credentials
        (long_username.as_str(), "pass", false),     // Very long username
    ];
    
    // Warm up to avoid cold start timing differences
    for _ in 0..5 {
        let _ = auth_service.authenticate_password("warmup", "warmup").await;
    }
    
    // Collect timing measurements
    let mut timings = Vec::new();
    
    for (username, password, _should_succeed) in &scenarios {
        let mut scenario_timings = Vec::new();
        
        // Multiple runs per scenario for statistical significance
        for _ in 0..10 {
            let start = Instant::now();
            let _ = auth_service.authenticate_password(username, password).await;
            let duration = start.elapsed();
            scenario_timings.push(duration);
        }
        
        // Use median to avoid outliers
        scenario_timings.sort();
        let median_timing = scenario_timings[scenario_timings.len() / 2];
        timings.push((format!("{}/{}", username, password), median_timing));
    }
    
    // Calculate average duration
    let avg_duration: Duration = timings.iter().map(|(_, d)| *d).sum::<Duration>() / timings.len() as u32;
    
    // Check that all timings are within acceptable variance (20% to account for async operations)
    for (scenario, timing) in &timings {
        let diff = if *timing > avg_duration {
            *timing - avg_duration
        } else {
            avg_duration - *timing
        };
        
        let variance_percent = (diff.as_micros() as f64 / avg_duration.as_micros() as f64) * 100.0;
        
        assert!(
            variance_percent < 20.0,
            "Timing variance too high for {}: {:.2}% ({}µs vs avg {}µs)",
            scenario,
            variance_percent,
            timing.as_micros(),
            avg_duration.as_micros()
        );
    }
    
    println!("Timing attack test results:");
    println!("Average duration: {}µs", avg_duration.as_micros());
    for (scenario, timing) in &timings {
        let diff = if *timing > avg_duration {
            *timing - avg_duration
        } else {
            avg_duration - *timing
        };
        let variance_percent = (diff.as_micros() as f64 / avg_duration.as_micros() as f64) * 100.0;
        println!("  {}: {}µs ({:+.1}%)", scenario, timing.as_micros(), variance_percent);
    }
}

#[tokio::test]
async fn test_password_length_timing_invariance() {
    let (auth_service, _temp_dir) = setup_test_auth_service().await;
    
    // Create user with medium length password
    auth_service.create_user(CreateUserRequest {
        username: "testuser".to_string(),
        password: "Test123Password!".to_string(),
        email: None,
        display_name: None,
        roles: vec!["user".to_string()],
    }).await.unwrap();
    
    // Test with different password lengths
    let long_pass_50 = "a".repeat(50);
    let long_pass_100 = "a".repeat(100);
    let password_tests = vec![
        "a",                                    // 1 char
        "Test123!",                            // 8 chars
        "Test123Password!",                    // 16 chars (correct)
        long_pass_50.as_str(),                 // 50 chars
        long_pass_100.as_str(),                // 100 chars
    ];
    
    let mut timings = Vec::new();
    
    for password in &password_tests {
        let mut pass_timings = Vec::new();
        
        for _ in 0..10 {
            let start = Instant::now();
            let _ = auth_service.authenticate_password("testuser", password).await;
            let duration = start.elapsed();
            pass_timings.push(duration);
        }
        
        pass_timings.sort();
        let median = pass_timings[pass_timings.len() / 2];
        timings.push((password.len(), median));
    }
    
    // Verify no correlation between password length and timing
    let avg_duration: Duration = timings.iter().map(|(_, d)| *d).sum::<Duration>() / timings.len() as u32;
    
    for (length, timing) in &timings {
        let diff = if *timing > avg_duration {
            *timing - avg_duration
        } else {
            avg_duration - *timing
        };
        
        let variance_percent = (diff.as_micros() as f64 / avg_duration.as_micros() as f64) * 100.0;
        
        assert!(
            variance_percent < 25.0,
            "Password length {} chars shows timing variance: {:.2}%",
            length,
            variance_percent
        );
    }
}

#[tokio::test]
async fn test_user_enumeration_prevention() {
    let (auth_service, _temp_dir) = setup_test_auth_service().await;
    
    // Create some users
    for i in 0..5 {
        auth_service.create_user(CreateUserRequest {
            username: format!("user{}", i),
            password: "SecurePass123!".to_string(),
            email: Some(format!("user{}@example.com", i)),
            display_name: None,
            roles: vec!["user".to_string()],
        }).await.unwrap();
    }
    
    // Test timing for existing vs non-existing users with wrong password
    let mut existing_timings = Vec::new();
    let mut nonexisting_timings = Vec::new();
    
    // Test existing users with wrong password
    for i in 0..5 {
        for _ in 0..5 {
            let start = Instant::now();
            let _ = auth_service.authenticate_password(&format!("user{}", i), "WrongPass123!").await;
            existing_timings.push(start.elapsed());
        }
    }
    
    // Test non-existing users
    for i in 100..105 {
        for _ in 0..5 {
            let start = Instant::now();
            let _ = auth_service.authenticate_password(&format!("user{}", i), "WrongPass123!").await;
            nonexisting_timings.push(start.elapsed());
        }
    }
    
    // Calculate medians
    existing_timings.sort();
    nonexisting_timings.sort();
    
    let existing_median = existing_timings[existing_timings.len() / 2];
    let nonexisting_median = nonexisting_timings[nonexisting_timings.len() / 2];
    
    // Verify similar timing
    let diff = if existing_median > nonexisting_median {
        existing_median - nonexisting_median
    } else {
        nonexisting_median - existing_median
    };
    
    let max_timing = existing_median.max(nonexisting_median);
    let variance_percent = (diff.as_micros() as f64 / max_timing.as_micros() as f64) * 100.0;
    
    assert!(
        variance_percent < 20.0,
        "User enumeration possible: existing users {}µs vs non-existing {}µs ({:.2}% difference)",
        existing_median.as_micros(),
        nonexisting_median.as_micros(),
        variance_percent
    );
}
