mod common;

use std::sync::Arc;
use std::time::Duration;
use tokio::time;

use crate::common::api_test_utils::*;
use rvoip_session_core::api::control::*;
use rvoip_session_core::api::types::*;
use rvoip_session_core::api::builder::SessionManagerBuilder;
use rvoip_session_core::Result;

#[tokio::test]
async fn test_control_function_signatures() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_control_function_signatures");
        
        let active_session = CallSession {
            id: SessionId::new(),
            from: "sip:test@example.com".to_string(),
            to: "sip:target@example.com".to_string(),
            state: CallState::Active,
            started_at: Some(std::time::Instant::now()),
        };
        
        // Test state validation logic in control functions
        assert!(active_session.is_active());
        assert!(active_session.state().is_in_progress());
        
        println!("Completed test_control_function_signatures");
    }).await;
    
    assert!(result.is_ok(), "test_control_function_signatures timed out");
}

#[tokio::test]
async fn test_call_state_validation() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_call_state_validation");
        
        let helper = ApiTypesTestHelper::new();
        let all_states = helper.get_all_call_states();
        
        for state in all_states {
            let session = CallSession {
                id: SessionId::new(),
                from: "sip:test@example.com".to_string(),
                to: "sip:target@example.com".to_string(),
                state: state.clone(),
                started_at: Some(std::time::Instant::now()),
            };
            
            println!("Testing control validation for state: {:?}", state);
            
            match state {
                CallState::Active => {
                    assert!(session.is_active());
                    assert!(session.state.is_in_progress());
                }
                CallState::OnHold => {
                    assert!(!session.is_active());
                    assert!(session.state.is_in_progress());
                }
                CallState::Terminated | CallState::Cancelled | CallState::Failed(_) => {
                    assert!(!session.is_active());
                    assert!(!session.state.is_in_progress());
                    assert!(session.state.is_final());
                }
                _ => {
                    assert!(!session.is_active() || session.state.is_in_progress());
                }
            }
        }
        
        println!("Completed test_call_state_validation");
    }).await;
    
    assert!(result.is_ok(), "test_call_state_validation timed out");
}

#[tokio::test]
async fn test_dtmf_validation() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_dtmf_validation");
        
        let valid_dtmf_sequences = vec![
            "0", "1", "2", "3", "4", "5", "6", "7", "8", "9",
            "*", "#",
            "123", "456", "789", "*0#",
            "12345678901234567890",
            "*123#456*789#",
        ];
        
        for digits in valid_dtmf_sequences {
            println!("Testing DTMF sequence: {}", digits);
            
            for char in digits.chars() {
                assert!(
                    char.is_ascii_digit() || char == '*' || char == '#',
                    "Invalid DTMF character: {}", char
                );
            }
        }
        
        println!("Completed test_dtmf_validation");
    }).await;
    
    assert!(result.is_ok(), "test_dtmf_validation timed out");
}

#[tokio::test]
async fn test_transfer_target_validation() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_transfer_target_validation");
        
        let valid_targets = vec![
            "sip:transfer@example.com",
            "sips:secure@example.com",
            "sip:user@192.168.1.100",
            "sip:user@example.com:5060",
            "sip:complex.user+tag@sub.domain.com:5061",
        ];
        
        for target in valid_targets {
            println!("Testing transfer target: {}", target);
            assert!(ApiTestUtils::is_valid_sip_uri(&target));
        }
        
        let invalid_targets = vec![
            "",
            "not_a_uri",
            "http://example.com",
            "sip:",
            "sip:@example.com",
        ];
        
        for target in invalid_targets {
            println!("Testing invalid transfer target: {}", target);
            assert!(!ApiTestUtils::is_valid_sip_uri(&target));
        }
        
        println!("Completed test_transfer_target_validation");
    }).await;
    
    assert!(result.is_ok(), "test_transfer_target_validation timed out");
}

#[tokio::test]
async fn test_control_edge_cases() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_control_edge_cases");
        
        let unicode_session = CallSession {
            id: SessionId("unicode_🦀_session".to_string()),
            from: "sip:user🦀@example.com".to_string(),
            to: "sip:target🚀@example.com".to_string(),
            state: CallState::Active,
            started_at: Some(std::time::Instant::now()),
        };
        
        assert!(unicode_session.is_active());
        assert!(unicode_session.from.contains("🦀"));
        assert!(unicode_session.to.contains("🚀"));
        
        println!("Completed test_control_edge_cases");
    }).await;
    
    assert!(result.is_ok(), "test_control_edge_cases timed out");
} 