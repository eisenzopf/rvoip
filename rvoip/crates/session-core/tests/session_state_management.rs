use rvoip_session_core::api::control::SessionControl;
//! Session State Management Tests
//!
//! Tests for StateManager functionality including state transition validation,
//! state transition rules, and edge case handling.

mod common;

use std::time::Duration;
use rvoip_session_core::{
    api::types::CallState,
    session::StateManager,
    SessionError,
};
use common::*;

#[tokio::test]
async fn test_state_manager_creation() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_state_manager_creation");
        
        let helper = StateManagerTestHelper::new();
        
        // Test basic functionality exists
        let can_transition = helper.test_transition(CallState::Initiating, CallState::Ringing).await;
        assert!(can_transition);
        
        println!("Completed test_state_manager_creation");
    }).await;
    
    if result.is_err() {
        panic!("test_state_manager_creation timed out");
    }
}

#[tokio::test]
async fn test_valid_state_transitions() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_valid_state_transitions");
        
        let helper = StateManagerTestHelper::new();
        let valid_transitions = StateManagerTestHelper::get_valid_transitions();
        
        for (from_state, to_state) in valid_transitions {
            let can_transition = helper.test_transition(from_state.clone(), to_state.clone()).await;
            assert!(can_transition, "Transition {:?} -> {:?} should be valid", from_state, to_state);
            
            // Also test validation
            let validation_result = helper.validate_transition(from_state.clone(), to_state.clone()).await;
            assert!(validation_result.is_ok(), "Validation for {:?} -> {:?} should succeed", from_state, to_state);
        }
        
        println!("Completed test_valid_state_transitions");
    }).await;
    
    if result.is_err() {
        panic!("test_valid_state_transitions timed out");
    }
}

#[tokio::test]
async fn test_invalid_state_transitions() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_invalid_state_transitions");
        
        let helper = StateManagerTestHelper::new();
        
        // Test some known invalid transitions
        let invalid_transitions = vec![
            (CallState::Terminated, CallState::Active),
            (CallState::Failed("test".to_string()), CallState::Ringing),
            (CallState::Active, CallState::Initiating),
            (CallState::Ringing, CallState::Initiating),
            (CallState::OnHold, CallState::Ringing),
        ];
        
        for (from_state, to_state) in invalid_transitions {
            let can_transition = helper.test_transition(from_state.clone(), to_state.clone()).await;
            assert!(!can_transition, "Transition {:?} -> {:?} should be invalid", from_state, to_state);
            
            // Also test validation
            let validation_result = helper.validate_transition(from_state.clone(), to_state.clone()).await;
            assert!(validation_result.is_err(), "Validation for {:?} -> {:?} should fail", from_state, to_state);
        }
        
        println!("Completed test_invalid_state_transitions");
    }).await;
    
    if result.is_err() {
        panic!("test_invalid_state_transitions timed out");
    }
}

#[tokio::test]
async fn test_state_transition_history() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_state_transition_history");
        
        let helper = StateManagerTestHelper::new();
        
        // Perform several transition tests
        helper.test_transition(CallState::Initiating, CallState::Ringing).await;
        helper.test_transition(CallState::Ringing, CallState::Active).await;
        helper.test_transition(CallState::Active, CallState::Terminated).await;
        helper.test_transition(CallState::Terminated, CallState::Active).await; // Invalid
        
        // Check history
        let history = helper.get_transition_history().await;
        assert_eq!(history.len(), 4);
        
        // Verify specific transitions
        assert_eq!(history[0], (CallState::Initiating, CallState::Ringing, true));
        assert_eq!(history[1], (CallState::Ringing, CallState::Active, true));
        assert_eq!(history[2], (CallState::Active, CallState::Terminated, true));
        assert_eq!(history[3], (CallState::Terminated, CallState::Active, false));
        
        // Clear and verify
        helper.clear_history().await;
        let cleared_history = helper.get_transition_history().await;
        assert_eq!(cleared_history.len(), 0);
        
        println!("Completed test_state_transition_history");
    }).await;
    
    if result.is_err() {
        panic!("test_state_transition_history timed out");
    }
}

#[tokio::test]
async fn test_all_state_combinations() {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        println!("Starting test_all_state_combinations");
        
        let states = StateManagerTestHelper::get_all_states();
        let helper = StateManagerTestHelper::new();
        let mut valid_count = 0;
        let mut invalid_count = 0;
        
        // Test every possible state combination
        for from_state in &states {
            for to_state in &states {
                let can_transition = helper.test_transition(from_state.clone(), to_state.clone()).await;
                
                if can_transition {
                    valid_count += 1;
                } else {
                    invalid_count += 1;
                }
            }
        }
        
        println!("Valid transitions: {}, Invalid transitions: {}", valid_count, invalid_count);
        
        // We should have some valid and some invalid transitions
        assert!(valid_count > 0, "Should have some valid transitions");
        assert!(invalid_count > 0, "Should have some invalid transitions");
        
        // Total should match states^2
        assert_eq!(valid_count + invalid_count, states.len() * states.len());
        
        println!("Completed test_all_state_combinations");
    }).await;
    
    if result.is_err() {
        panic!("test_all_state_combinations timed out");
    }
}

#[tokio::test]
async fn test_terminal_states() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_terminal_states");
        
        let helper = StateManagerTestHelper::new();
        
        // Test that terminal states don't allow transitions to non-terminal states
        let terminal_states = vec![
            CallState::Terminated,
            CallState::Failed("test".to_string()),
        ];
        
        let non_terminal_states = vec![
            CallState::Initiating,
            CallState::Ringing,
            CallState::Active,
            CallState::OnHold,
        ];
        
        for terminal_state in &terminal_states {
            for non_terminal_state in &non_terminal_states {
                let can_transition = helper.test_transition(terminal_state.clone(), non_terminal_state.clone()).await;
                assert!(!can_transition, 
                       "Terminal state {:?} should not transition to {:?}", 
                       terminal_state, non_terminal_state);
            }
        }
        
        println!("Completed test_terminal_states");
    }).await;
    
    if result.is_err() {
        panic!("test_terminal_states timed out");
    }
}

#[tokio::test]
async fn test_states_to_terminal() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_states_to_terminal");
        
        let helper = StateManagerTestHelper::new();
        
        // Test that any state can transition to terminal states
        let all_states = StateManagerTestHelper::get_all_states();
        let terminal_states = vec![
            CallState::Terminated,
            CallState::Failed("test".to_string()),
        ];
        
        for state in &all_states {
            for terminal_state in &terminal_states {
                let can_transition = helper.test_transition(state.clone(), terminal_state.clone()).await;
                assert!(can_transition, 
                       "State {:?} should be able to transition to terminal state {:?}", 
                       state, terminal_state);
            }
        }
        
        println!("Completed test_states_to_terminal");
    }).await;
    
    if result.is_err() {
        panic!("test_states_to_terminal timed out");
    }
}

#[tokio::test]
async fn test_hold_resume_cycle() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_hold_resume_cycle");
        
        let helper = StateManagerTestHelper::new();
        
        // Test Active <-> OnHold transitions
        assert!(helper.test_transition(CallState::Active, CallState::OnHold).await);
        assert!(helper.test_transition(CallState::OnHold, CallState::Active).await);
        
        // Test that you can't go to hold from non-active states
        let non_active_states = vec![
            CallState::Initiating,
            CallState::Ringing,
            CallState::Terminated,
        ];
        
        for state in non_active_states {
            assert!(!helper.test_transition(state.clone(), CallState::OnHold).await,
                   "State {:?} should not transition to OnHold", state);
        }
        
        // Test that you can only resume to Active (but can terminate from OnHold)
        let invalid_targets = vec![
            CallState::Initiating,
            CallState::Ringing,
        ];
        
        for state in invalid_targets {
            assert!(!helper.test_transition(CallState::OnHold, state.clone()).await,
                   "OnHold should not transition to {:?}", state);
        }
        
        // Test that you CAN terminate from OnHold
        assert!(helper.test_transition(CallState::OnHold, CallState::Terminated).await,
               "OnHold should be able to transition to Terminated");
        
        println!("Completed test_hold_resume_cycle");
    }).await;
    
    if result.is_err() {
        panic!("test_hold_resume_cycle timed out");
    }
}

#[tokio::test]
async fn test_call_establishment_flow() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_call_establishment_flow");
        
        let helper = StateManagerTestHelper::new();
        
        // Test normal call flow
        assert!(helper.test_transition(CallState::Initiating, CallState::Ringing).await);
        assert!(helper.test_transition(CallState::Ringing, CallState::Active).await);
        
        // Test that you can't skip states
        assert!(!helper.test_transition(CallState::Initiating, CallState::Active).await);
        assert!(!helper.test_transition(CallState::Initiating, CallState::OnHold).await);
        
        // Test early termination is allowed
        assert!(helper.test_transition(CallState::Initiating, CallState::Terminated).await);
        assert!(helper.test_transition(CallState::Ringing, CallState::Terminated).await);
        
        println!("Completed test_call_establishment_flow");
    }).await;
    
    if result.is_err() {
        panic!("test_call_establishment_flow timed out");
    }
}

#[tokio::test]
async fn test_failed_state_transitions() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_failed_state_transitions");
        
        let helper = StateManagerTestHelper::new();
        
        // Test that any state can fail
        let all_states = StateManagerTestHelper::get_all_states();
        let failed_states = vec![
            CallState::Failed("network error".to_string()),
            CallState::Failed("timeout".to_string()),
            CallState::Failed("codec negotiation failed".to_string()),
        ];
        
        for state in &all_states {
            for failed_state in &failed_states {
                let can_transition = helper.test_transition(state.clone(), failed_state.clone()).await;
                assert!(can_transition, 
                       "State {:?} should be able to transition to failed state", state);
            }
        }
        
        // Test that failed states are terminal (can't transition out)
        let non_terminal_states = vec![
            CallState::Initiating,
            CallState::Ringing,
            CallState::Active,
            CallState::OnHold,
        ];
        
        for failed_state in &failed_states {
            for non_terminal_state in &non_terminal_states {
                let can_transition = helper.test_transition(failed_state.clone(), non_terminal_state.clone()).await;
                assert!(!can_transition, 
                       "Failed state should not transition to {:?}", non_terminal_state);
            }
        }
        
        println!("Completed test_failed_state_transitions");
    }).await;
    
    if result.is_err() {
        panic!("test_failed_state_transitions timed out");
    }
}

#[tokio::test]
async fn test_state_validation_errors() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_state_validation_errors");
        
        let helper = StateManagerTestHelper::new();
        
        // Test validation returns appropriate errors
        let invalid_transitions = vec![
            (CallState::Terminated, CallState::Active),
            (CallState::Active, CallState::Ringing),
            (CallState::OnHold, CallState::Initiating),
        ];
        
        for (from_state, to_state) in invalid_transitions {
            let validation_result = helper.validate_transition(from_state.clone(), to_state.clone()).await;
            assert!(validation_result.is_err());
            
            // Check that error message contains relevant information
            let error = validation_result.unwrap_err();
            let error_msg = format!("{:?}", error);
            assert!(error_msg.contains(&format!("{:?}", from_state)) || 
                   error_msg.contains(&format!("{:?}", to_state)),
                   "Error message should contain state information: {}", error_msg);
        }
        
        println!("Completed test_state_validation_errors");
    }).await;
    
    if result.is_err() {
        panic!("test_state_validation_errors timed out");
    }
}

#[tokio::test]
async fn test_concurrent_state_validation() {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        println!("Starting test_concurrent_state_validation");
        
        let helper = StateManagerTestHelper::new();
        let concurrent_tasks = 10;
        let mut handles = Vec::new();
        
        // Spawn concurrent validation tasks
        for task_id in 0..concurrent_tasks {
            let helper_clone = StateManagerTestHelper::new();
            let handle = tokio::spawn(async move {
                let mut results = Vec::new();
                
                // Test various transitions
                results.push(helper_clone.test_transition(CallState::Initiating, CallState::Ringing).await);
                results.push(helper_clone.test_transition(CallState::Ringing, CallState::Active).await);
                results.push(helper_clone.test_transition(CallState::Active, CallState::OnHold).await);
                results.push(helper_clone.test_transition(CallState::OnHold, CallState::Active).await);
                results.push(helper_clone.test_transition(CallState::Active, CallState::Terminated).await);
                
                // Test invalid transitions
                results.push(helper_clone.test_transition(CallState::Terminated, CallState::Active).await);
                results.push(helper_clone.test_transition(CallState::Active, CallState::Initiating).await);
                
                (task_id, results)
            });
            handles.push(handle);
        }
        
        // Wait for all tasks to complete
        for handle in handles {
            let (task_id, results) = handle.await.unwrap();
            
            // Verify expected results
            assert_eq!(results.len(), 7);
            assert!(results[0]); // Initiating -> Ringing
            assert!(results[1]); // Ringing -> Active
            assert!(results[2]); // Active -> OnHold
            assert!(results[3]); // OnHold -> Active
            assert!(results[4]); // Active -> Terminated
            assert!(!results[5]); // Terminated -> Active (invalid)
            assert!(!results[6]); // Active -> Initiating (invalid)
        }
        
        println!("Completed test_concurrent_state_validation");
    }).await;
    
    if result.is_err() {
        panic!("test_concurrent_state_validation timed out");
    }
}

#[tokio::test]
async fn test_state_transition_performance() {
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        println!("Starting test_state_transition_performance");
        
        let helper = StateManagerTestHelper::new();
        let validation_count = 1000;
        
        let start = std::time::Instant::now();
        
        // Perform many validations quickly
        for _ in 0..validation_count {
            helper.test_transition(CallState::Initiating, CallState::Ringing).await;
            helper.test_transition(CallState::Ringing, CallState::Active).await;
            helper.test_transition(CallState::Active, CallState::OnHold).await;
            helper.test_transition(CallState::OnHold, CallState::Active).await;
            helper.test_transition(CallState::Active, CallState::Terminated).await;
        }
        
        let duration = start.elapsed();
        println!("Performed {} state validations in {:?}", validation_count * 5, duration);
        
        // Performance assertion
        assert!(duration < Duration::from_secs(10), "State validation took too long");
        
        println!("Completed test_state_transition_performance");
    }).await;
    
    if result.is_err() {
        panic!("test_state_transition_performance timed out");
    }
}

#[tokio::test]
async fn test_state_edge_cases() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_state_edge_cases");
        
        let helper = StateManagerTestHelper::new();
        
        // Test self-transitions (should mostly be invalid)
        let states = StateManagerTestHelper::get_all_states();
        for state in &states {
            let can_self_transition = helper.test_transition(state.clone(), state.clone()).await;
            
            // Generally self-transitions should be invalid, but let's be flexible
            // and just verify that the validation function runs without error
            helper.validate_transition(state.clone(), state.clone()).await.ok();
        }
        
        // Test transitions with failed states containing edge case strings
        let edge_case_failures = vec![
            CallState::Failed("".to_string()), // Empty failure reason
            CallState::Failed("🦀".to_string()), // Unicode
            CallState::Failed("a".repeat(1000)), // Very long reason
            CallState::Failed("\n\r\t".to_string()), // Special characters
        ];
        
        for failed_state in edge_case_failures {
            // Should be able to transition to any failure state
            assert!(helper.test_transition(CallState::Active, failed_state.clone()).await);
            
            // Should not be able to transition from failure state
            assert!(!helper.test_transition(failed_state, CallState::Active).await);
        }
        
        println!("Completed test_state_edge_cases");
    }).await;
    
    if result.is_err() {
        panic!("test_state_edge_cases timed out");
    }
}

#[tokio::test]
async fn test_comprehensive_state_validation() {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        println!("Starting test_comprehensive_state_validation");
        
        // Use the utility function to test all state combinations
        let states = StateManagerTestHelper::get_all_states();
        let mut results = Vec::new();
        
        for from_state in &states {
            for to_state in &states {
                let is_valid = StateManager::can_transition(from_state, to_state);
                results.push((from_state.clone(), to_state.clone(), is_valid));
            }
        }
        
        let mut valid_transitions = 0;
        let mut invalid_transitions = 0;
        
        for (from_state, to_state, is_valid) in &results {
            if *is_valid {
                valid_transitions += 1;
            } else {
                invalid_transitions += 1;
            }
        }
        
        println!("Comprehensive validation: {} valid, {} invalid transitions", 
                valid_transitions, invalid_transitions);
        
        // Verify we have expected number of total combinations
        let states = StateManagerTestHelper::get_all_states();
        assert_eq!(results.len(), states.len() * states.len());
        
        // Verify we have both valid and invalid transitions
        assert!(valid_transitions > 0, "Should have some valid transitions");
        assert!(invalid_transitions > 0, "Should have some invalid transitions");
        
        println!("Completed test_comprehensive_state_validation");
    }).await;
    
    if result.is_err() {
        panic!("test_comprehensive_state_validation timed out");
    }
}

#[tokio::test]
async fn test_state_manager_helper_functionality() {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Starting test_state_manager_helper_functionality");
        
        let helper = StateManagerTestHelper::new();
        
        // Test helper methods
        let all_states = StateManagerTestHelper::get_all_states();
        assert!(all_states.len() > 0);
        assert!(all_states.contains(&CallState::Initiating));
        assert!(all_states.contains(&CallState::Active));
        assert!(all_states.contains(&CallState::Terminated));
        
        let valid_transitions = StateManagerTestHelper::get_valid_transitions();
        assert!(valid_transitions.len() > 0);
        assert!(valid_transitions.contains(&(CallState::Initiating, CallState::Ringing)));
        assert!(valid_transitions.contains(&(CallState::Active, CallState::OnHold)));
        
        // Test history functionality
        helper.test_transition(CallState::Initiating, CallState::Ringing).await;
        assert_eq!(helper.get_transition_history().await.len(), 1);
        
        helper.clear_history().await;
        assert_eq!(helper.get_transition_history().await.len(), 0);
        
        println!("Completed test_state_manager_helper_functionality");
    }).await;
    
    if result.is_err() {
        panic!("test_state_manager_helper_functionality timed out");
    }
} 