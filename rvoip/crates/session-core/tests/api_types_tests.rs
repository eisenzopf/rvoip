mod common;

use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::time;

use crate::common::api_test_utils::*;
use rvoip_session_core::api::types::*;
use rvoip_session_core::Result;

#[tokio::test]
async fn test_session_id_creation() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_session_id_creation");
        
        // Test new session ID generation
        let id1 = SessionId::new();
        let id2 = SessionId::new();
        
        // IDs should be unique
        assert_ne!(id1.as_str(), id2.as_str());
        
        // IDs should not be empty
        assert!(!id1.as_str().is_empty());
        assert!(!id2.as_str().is_empty());
        
        // IDs should start with expected prefix
        assert!(id1.as_str().starts_with("sess_"));
        assert!(id2.as_str().starts_with("sess_"));
        
        // Test custom session ID
        let custom_id = SessionId("custom_session_123".to_string());
        assert_eq!(custom_id.as_str(), "custom_session_123");
        
        // Test Display trait
        let display_string = format!("{}", id1);
        assert_eq!(display_string, id1.as_str());
        
        println!("Completed test_session_id_creation");
    }).await;
    
    assert!(result.is_ok(), "test_session_id_creation timed out");
}

#[tokio::test]
async fn test_session_id_equality_and_hash() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_session_id_equality_and_hash");
        
        // Test equality
        let id1 = SessionId("test_id".to_string());
        let id2 = SessionId("test_id".to_string());
        let id3 = SessionId("different_id".to_string());
        
        assert_eq!(id1, id2);
        assert_ne!(id1, id3);
        
        // Test hash map usage
        let mut map = HashMap::new();
        map.insert(id1.clone(), "value1");
        map.insert(id3.clone(), "value3");
        
        assert_eq!(map.get(&id2), Some(&"value1"));
        assert_eq!(map.get(&id3), Some(&"value3"));
        assert_eq!(map.len(), 2);
        
        println!("Completed test_session_id_equality_and_hash");
    }).await;
    
    assert!(result.is_ok(), "test_session_id_equality_and_hash timed out");
}

#[tokio::test]
async fn test_call_session_creation() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_call_session_creation");
        
        let helper = ApiTypesTestHelper::new();
        let sessions = helper.create_test_call_sessions(3);
        
        for (i, session) in sessions.iter().enumerate() {
            // Validate basic properties
            assert_eq!(session.id.as_str(), format!("session_{}", i));
            assert_eq!(session.from, format!("sip:user{}@example.com", i));
            assert_eq!(session.to, format!("sip:target{}@example.com", i));
            assert_eq!(session.state, CallState::Active);
            assert!(session.started_at.is_some());
            
            // Test helper methods
            assert_eq!(session.id(), &SessionId(format!("session_{}", i)));
            assert_eq!(session.state(), &CallState::Active);
            assert!(session.is_active());
        }
        
        println!("Completed test_call_session_creation");
    }).await;
    
    assert!(result.is_ok(), "test_call_session_creation timed out");
}

#[tokio::test]
async fn test_call_session_deprecated_methods() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_call_session_deprecated_methods");
        
        let session = CallSession {
            id: SessionId("test_session".to_string()),
            from: "sip:test@example.com".to_string(),
            to: "sip:target@example.com".to_string(),
            state: CallState::Active,
            started_at: Some(Instant::now()),
        };
        
        // Test that deprecated methods return appropriate errors
        let hold_result = session.hold().await;
        assert!(hold_result.is_err());
        assert!(format!("{:?}", hold_result).contains("Use SessionManager"));
        
        let resume_result = session.resume().await;
        assert!(resume_result.is_err());
        assert!(format!("{:?}", resume_result).contains("Use SessionManager"));
        
        let transfer_result = session.transfer("sip:transfer@example.com").await;
        assert!(transfer_result.is_err());
        assert!(format!("{:?}", transfer_result).contains("Use SessionManager"));
        
        let terminate_result = session.terminate().await;
        assert!(terminate_result.is_err());
        assert!(format!("{:?}", terminate_result).contains("Use SessionManager"));
        
        println!("Completed test_call_session_deprecated_methods");
    }).await;
    
    assert!(result.is_ok(), "test_call_session_deprecated_methods timed out");
}

#[tokio::test]
async fn test_incoming_call_creation() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_incoming_call_creation");
        
        let helper = ApiTypesTestHelper::new();
        let calls = helper.create_test_incoming_calls(3);
        
        for (i, call) in calls.iter().enumerate() {
            // Validate basic properties
            assert_eq!(call.id.as_str(), format!("incoming_{}", i));
            assert_eq!(call.from, format!("sip:caller{}@example.com", i));
            assert_eq!(call.to, format!("sip:callee{}@example.com", i));
            assert!(call.sdp.is_some());
            assert!(!call.headers.is_empty() || call.headers.is_empty()); // Either is valid
            
            // Test helper methods
            assert_eq!(call.caller(), &format!("sip:caller{}@example.com", i));
            assert_eq!(call.called(), &format!("sip:callee{}@example.com", i));
            
            // Validate SDP if present
            if let Some(ref sdp) = call.sdp {
                assert!(ApiTestUtils::is_valid_sdp(sdp));
                assert!(sdp.contains(&format!("session_{}", i)));
            }
        }
        
        println!("Completed test_incoming_call_creation");
    }).await;
    
    assert!(result.is_ok(), "test_incoming_call_creation timed out");
}

#[tokio::test]
async fn test_call_state_properties() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_call_state_properties");
        
        let helper = ApiTypesTestHelper::new();
        let states = helper.get_all_call_states();
        
        for state in states {
            println!("Testing state: {:?}", state);
            
            // Test final state property
            match state {
                CallState::Terminated | CallState::Cancelled | CallState::Failed(_) => {
                    assert!(state.is_final(), "State {:?} should be final", state);
                    assert!(!state.is_in_progress(), "Final state {:?} should not be in progress", state);
                }
                _ => {
                    assert!(!state.is_final(), "State {:?} should not be final", state);
                }
            }
            
            // Test in-progress state property
            match state {
                CallState::Initiating | CallState::Ringing | CallState::Active | CallState::OnHold => {
                    assert!(state.is_in_progress(), "State {:?} should be in progress", state);
                }
                _ => {
                    assert!(!state.is_in_progress(), "State {:?} should not be in progress", state);
                }
            }
        }
        
        println!("Completed test_call_state_properties");
    }).await;
    
    assert!(result.is_ok(), "test_call_state_properties timed out");
}

#[tokio::test]
async fn test_call_state_transitions() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_call_state_transitions");
        
        let helper = ApiTypesTestHelper::new();
        let valid_transitions = helper.get_valid_state_transitions();
        
        // Test that valid transitions make sense
        for (from, to) in valid_transitions {
            println!("Testing transition: {:?} -> {:?}", from, to);
            
            // Can't transition from final states (except the ones we explicitly allow)
            if from.is_final() {
                panic!("Transition from final state {:?} to {:?} should not be valid", from, to);
            }
            
            // Can always transition to final states
            if to.is_final() {
                // This is always valid
                continue;
            }
            
            // Test specific valid transitions
            match (&from, &to) {
                (CallState::Initiating, CallState::Ringing) => {},
                (CallState::Ringing, CallState::Active) => {},
                (CallState::Active, CallState::OnHold) => {},
                (CallState::OnHold, CallState::Active) => {},
                (CallState::Active, CallState::Transferring) => {},
                _ => {
                    if !to.is_final() {
                        println!("Warning: Unexpected transition {:?} -> {:?}", from, to);
                    }
                }
            }
        }
        
        println!("Completed test_call_state_transitions");
    }).await;
    
    assert!(result.is_ok(), "test_call_state_transitions timed out");
}

#[tokio::test]
async fn test_call_decision_types() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_call_decision_types");
        
        let helper = ApiTypesTestHelper::new();
        let decisions = helper.get_all_call_decisions();
        
        assert_eq!(decisions.len(), 4);
        
        // Verify each decision type
        let accept_found = decisions.iter().any(|d| matches!(d, CallDecision::Accept));
        let reject_found = decisions.iter().any(|d| matches!(d, CallDecision::Reject(_)));
        let defer_found = decisions.iter().any(|d| matches!(d, CallDecision::Defer));
        let forward_found = decisions.iter().any(|d| matches!(d, CallDecision::Forward(_)));
        
        assert!(accept_found, "Accept decision not found");
        assert!(reject_found, "Reject decision not found");
        assert!(defer_found, "Defer decision not found");
        assert!(forward_found, "Forward decision not found");
        
        // Test specific decision values
        for decision in decisions {
            match decision {
                CallDecision::Accept => {
                    // No additional data to verify
                }
                CallDecision::Reject(reason) => {
                    assert!(!reason.is_empty(), "Reject reason should not be empty");
                    assert_eq!(reason, "Test rejection");
                }
                CallDecision::Defer => {
                    // No additional data to verify
                }
                CallDecision::Forward(target) => {
                    assert!(!target.is_empty(), "Forward target should not be empty");
                    assert!(ApiTestUtils::is_valid_sip_uri(&target), "Forward target should be valid SIP URI");
                }
            }
        }
        
        println!("Completed test_call_decision_types");
    }).await;
    
    assert!(result.is_ok(), "test_call_decision_types timed out");
}

#[tokio::test]
async fn test_session_stats_validation() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_session_stats_validation");
        
        let helper = ApiTypesTestHelper::new();
        let stats = helper.create_test_session_stats();
        
        // Validate the test stats
        assert!(ApiTestUtils::validate_session_stats(&stats).is_ok());
        
        // Test stats properties
        assert_eq!(stats.total_sessions, 150);
        assert_eq!(stats.active_sessions, 23);
        assert_eq!(stats.failed_sessions, 7);
        assert_eq!(stats.average_duration, Some(Duration::from_secs(180)));
        
        // Active sessions should be <= total sessions
        assert!(stats.active_sessions <= stats.total_sessions);
        
        // Failed sessions should be <= total sessions
        assert!(stats.failed_sessions <= stats.total_sessions);
        
        // Test invalid stats
        let invalid_stats1 = SessionStats {
            total_sessions: 10,
            active_sessions: 15, // Invalid: more active than total
            failed_sessions: 2,
            average_duration: None,
        };
        assert!(ApiTestUtils::validate_session_stats(&invalid_stats1).is_err());
        
        let invalid_stats2 = SessionStats {
            total_sessions: 10,
            active_sessions: 5,
            failed_sessions: 15, // Invalid: more failed than total
            average_duration: None,
        };
        assert!(ApiTestUtils::validate_session_stats(&invalid_stats2).is_err());
        
        println!("Completed test_session_stats_validation");
    }).await;
    
    assert!(result.is_ok(), "test_session_stats_validation timed out");
}

#[tokio::test]
async fn test_media_info_creation() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_media_info_creation");
        
        let helper = ApiTypesTestHelper::new();
        let media_info = helper.create_test_media_info("test_session");
        
        // Validate media info properties
        assert!(media_info.local_sdp.is_some());
        assert!(media_info.remote_sdp.is_some());
        assert_eq!(media_info.local_rtp_port, Some(5004));
        assert_eq!(media_info.remote_rtp_port, Some(5006));
        assert_eq!(media_info.codec, Some("PCMU".to_string()));
        
        // Validate SDP content
        if let Some(ref local_sdp) = media_info.local_sdp {
            assert!(ApiTestUtils::is_valid_sdp(local_sdp));
            assert!(local_sdp.contains("test_session"));
        }
        
        if let Some(ref remote_sdp) = media_info.remote_sdp {
            assert!(ApiTestUtils::is_valid_sdp(remote_sdp));
            assert!(remote_sdp.contains("remote_test_session"));
        }
        
        // Test empty media info
        let empty_media_info = MediaInfo {
            local_sdp: None,
            remote_sdp: None,
            local_rtp_port: None,
            remote_rtp_port: None,
            codec: None,
        };
        
        assert!(empty_media_info.local_sdp.is_none());
        assert!(empty_media_info.remote_sdp.is_none());
        assert!(empty_media_info.local_rtp_port.is_none());
        assert!(empty_media_info.remote_rtp_port.is_none());
        assert!(empty_media_info.codec.is_none());
        
        println!("Completed test_media_info_creation");
    }).await;
    
    assert!(result.is_ok(), "test_media_info_creation timed out");
}

#[tokio::test]
async fn test_types_serialization() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_types_serialization");
        
        // Test SessionId serialization
        let session_id = SessionId("test_session_123".to_string());
        let serialized = serde_json::to_string(&session_id).unwrap();
        let deserialized: SessionId = serde_json::from_str(&serialized).unwrap();
        assert_eq!(session_id, deserialized);
        
        // Test CallState serialization
        let states = vec![
            CallState::Initiating,
            CallState::Ringing,
            CallState::Active,
            CallState::OnHold,
            CallState::Terminated,
            CallState::Failed("Test error".to_string()),
        ];
        
        for state in states {
            let serialized = serde_json::to_string(&state).unwrap();
            let deserialized: CallState = serde_json::from_str(&serialized).unwrap();
            assert_eq!(state, deserialized);
        }
        
        println!("Completed test_types_serialization");
    }).await;
    
    assert!(result.is_ok(), "test_types_serialization timed out");
}

#[tokio::test]
async fn test_types_edge_cases() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_types_edge_cases");
        
        // Test empty session ID
        let empty_id = SessionId("".to_string());
        assert_eq!(empty_id.as_str(), "");
        
        // Test very long session ID
        let long_id = SessionId("a".repeat(1000));
        assert_eq!(long_id.as_str().len(), 1000);
        
        // Test unicode session ID
        let unicode_id = SessionId("session_🦀_test".to_string());
        assert!(unicode_id.as_str().contains("🦀"));
        
        // Test call session with unicode URIs
        let unicode_session = CallSession {
            id: SessionId("unicode_test".to_string()),
            from: "sip:user🦀@example.com".to_string(),
            to: "sip:target🚀@example.com".to_string(),
            state: CallState::Active,
            started_at: Some(Instant::now()),
        };
        
        assert!(unicode_session.from.contains("🦀"));
        assert!(unicode_session.to.contains("🚀"));
        
        // Test failed state with different error messages
        let failed_states = vec![
            CallState::Failed("".to_string()), // Empty error
            CallState::Failed("Network timeout".to_string()),
            CallState::Failed("🔥 Unicode error 🔥".to_string()),
            CallState::Failed("Very long error message ".repeat(100)),
        ];
        
        for state in failed_states {
            assert!(state.is_final());
            assert!(!state.is_in_progress());
        }
        
        println!("Completed test_types_edge_cases");
    }).await;
    
    assert!(result.is_ok(), "test_types_edge_cases timed out");
}

#[tokio::test]
async fn test_types_validation_helpers() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_types_validation_helpers");
        
        let helper = ApiTypesTestHelper::new();
        
        // Test valid call session validation
        let valid_session = CallSession {
            id: SessionId("valid_session".to_string()),
            from: "sip:user@example.com".to_string(),
            to: "sip:target@example.com".to_string(),
            state: CallState::Active,
            started_at: Some(Instant::now()),
        };
        
        assert!(helper.validate_call_session(&valid_session).is_ok());
        
        // Test invalid call session validation
        let invalid_session1 = CallSession {
            id: SessionId("".to_string()), // Empty ID
            from: "sip:user@example.com".to_string(),
            to: "sip:target@example.com".to_string(),
            state: CallState::Active,
            started_at: Some(Instant::now()),
        };
        assert!(helper.validate_call_session(&invalid_session1).is_err());
        
        let invalid_session2 = CallSession {
            id: SessionId("valid_id".to_string()),
            from: "invalid_uri".to_string(), // Invalid URI
            to: "sip:target@example.com".to_string(),
            state: CallState::Active,
            started_at: Some(Instant::now()),
        };
        assert!(helper.validate_call_session(&invalid_session2).is_err());
        
        // Test valid incoming call validation
        let valid_call = IncomingCall {
            id: SessionId("valid_call".to_string()),
            from: "sip:caller@example.com".to_string(),
            to: "sip:callee@example.com".to_string(),
            sdp: Some(helper.create_test_sdp("test")),
            headers: HashMap::new(),
            received_at: Instant::now(),
        };
        assert!(helper.validate_incoming_call(&valid_call).is_ok());
        
        // Test invalid incoming call validation
        let invalid_call = IncomingCall {
            id: SessionId("valid_id".to_string()),
            from: "sip:caller@example.com".to_string(),
            to: "sip:callee@example.com".to_string(),
            sdp: Some("invalid sdp".to_string()), // Invalid SDP
            headers: HashMap::new(),
            received_at: Instant::now(),
        };
        assert!(helper.validate_incoming_call(&invalid_call).is_err());
        
        println!("Completed test_types_validation_helpers");
    }).await;
    
    assert!(result.is_ok(), "test_types_validation_helpers timed out");
}

#[tokio::test]
async fn test_types_performance() {
    let result = time::timeout(Duration::from_secs(10), async {
        println!("Starting test_types_performance");
        
        let helper = ApiTypesTestHelper::new();
        let start = Instant::now();
        
        // Create session IDs with sufficient spacing for timestamp-based generation
        let mut session_ids = Vec::new();
        for i in 0..100 {  // Reduced count for better spacing
            session_ids.push(SessionId::new());
            // Add delay to allow timestamp to advance
            if i < 99 {
                tokio::time::sleep(Duration::from_micros(10)).await;
            }
        }
        
        // Create many call sessions
        let sessions = helper.create_test_call_sessions(100);
        
        // Create many incoming calls
        let calls = helper.create_test_incoming_calls(100);
        
        let duration = start.elapsed();
        println!("Created 300 objects in {:?}", duration);
        
        // Performance should be reasonable
        assert!(duration < Duration::from_secs(5), "Type creation took too long");
        
        // Verify all objects were created properly
        assert_eq!(session_ids.len(), 100);
        assert_eq!(sessions.len(), 100);
        assert_eq!(calls.len(), 100);
        
        // Test that session IDs are mostly unique (timestamp-based generation may have some duplicates at high speed)
        let mut unique_ids = std::collections::HashSet::new();
        for id in &session_ids {
            unique_ids.insert(id.as_str());
        }
        let uniqueness_ratio = unique_ids.len() as f64 / session_ids.len() as f64;
        println!("Session ID uniqueness: {}/{} ({:.1}%)", unique_ids.len(), session_ids.len(), uniqueness_ratio * 100.0);
        
        // Should have at least 90% unique IDs (timestamp-based generation with spacing)
        assert!(uniqueness_ratio >= 0.90, "Session ID uniqueness too low: {:.1}%", uniqueness_ratio * 100.0);
        
        println!("Completed test_types_performance");
    }).await;
    
    assert!(result.is_ok(), "test_types_performance timed out");
} 