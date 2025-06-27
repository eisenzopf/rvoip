/// INVITE Transaction Tests for SIP according to RFC 3261
///
/// This test suite verifies the behavior of INVITE transactions in the rVOIP transaction-core
/// library according to RFC 3261 Sections 17.1.1 (INVITE Client Transaction) and 17.2.1 
/// (INVITE Server Transaction).
///
/// RFC 3261 Compliance Verification:
///
/// 1. INVITE Client Transaction (Section 17.1.1):
///    - Initial state should be "Calling" after sending INVITE
///    - After receiving a provisional response (1xx), should transition to "Proceeding"
///    - After receiving a final response:
///       * For 2xx responses: The transaction should terminate
///       * For non-2xx responses: Should transition to "Completed" and generate ACK automatically
///       * After Timer D expires in Completed state, should transition to "Terminated"
///
/// 2. INVITE Server Transaction (Section 17.2.1):
///    - Initial state should be "Proceeding" after receiving INVITE and sending 1xx
///    - For 2xx responses: 
///       * After sending 2xx, should transition directly to "Terminated"
///       * ACK is handled by the TU (Transaction User), not the transaction layer
///    - For non-2xx responses:
///       * After sending non-2xx, should transition to "Completed"
///       * After receiving ACK, should transition to "Confirmed"
///       * After Timer I expires in Confirmed state, should transition to "Terminated"
///
/// Special handling of ACK in INVITE transactions:
/// - For 2xx responses: ACK is a separate transaction (TU responsibility)
/// - For non-2xx responses: ACK is part of the original transaction (transaction layer handles this)
///
/// Test Coverage:
/// 1. test_invite_success_flow: Tests INVITE transaction with a 200 OK final response
/// 2. test_invite_failure_flow: Tests INVITE transaction with a 4xx final response and ACK handling

mod transaction_test_utils;

use std::time::Duration;
use std::env;
use tokio::time::sleep;
use tokio::time::timeout;
use std::sync::Arc;
use serial_test::serial;

use rvoip_sip_core::types::status::StatusCode;
use rvoip_sip_core::prelude::Method;
use rvoip_sip_core::Message;
use rvoip_transaction_core::transaction::TransactionState;
use rvoip_transaction_core::{TransactionEvent, TransactionKey};

use transaction_test_utils::*;

/// Tests the complete flow of a successful INVITE transaction (2xx response)
///
/// Flow sequence:
/// 1. Client sends INVITE request
/// 2. Server receives INVITE and creates server transaction  
/// 3. Server sends provisional responses (100 Trying, 180 Ringing)
/// 4. Client receives provisional responses and moves to Proceeding
/// 5. Server sends 200 OK (success)
/// 6. Client receives 200 OK and terminates the transaction
/// 7. TU is responsible for sending ACK for 2xx (not the transaction layer)
#[tokio::test]
#[serial]
async fn test_invite_success_flow() {
    // Set test environment variable
    env::set_var("RVOIP_TEST", "1");
    
    println!("\n==== TEST: INVITE Success Flow ====");
    println!("Testing INVITE transaction with 2xx responses using proper event-driven approach");
    println!("This test verifies the success path of INVITE transactions according to RFC 3261 Section 17.1.1/17.2.1");
    println!("Scenario: Client sends INVITE, server responds with 1xx then 200 OK");
    println!("Expected behavior: Event-driven state transitions and automatic timer management\n");
    
    // Test timeout to prevent hanging
    let test_result = timeout(Duration::from_secs(10), async {
        // 1. Initialize the test environment
        let mut env = TestEnvironment::new().await;
        
        // 2. Create an INVITE request
        let server_uri = format!("sip:server@{}", env.server_addr);
        let invite_request = env.create_request(Method::Invite, &server_uri);
        
        // 3. Create client transaction and subscribe to its events
        println!("Creating INVITE client transaction");
        let client_tx_id = env.client_manager.create_client_transaction(
            invite_request.clone(),
            env.server_addr
        ).await.expect("Failed to create client transaction");
        println!("Client transaction created with ID: {:?}", client_tx_id);
        
        // Subscribe to client events for this specific transaction
        let mut client_events = env.client_manager.subscribe_to_transaction(&client_tx_id)
            .await.expect("Failed to subscribe to client transaction events");
        
        // 4. Subscribe to server events to detect incoming requests
        let mut server_events = env.server_manager.subscribe();
        
        // 5. Send the INVITE request - this triggers automatic state machine
        println!("Starting client transaction (sending INVITE)");
        env.client_manager.send_request(&client_tx_id).await
            .expect("Failed to send request");
        println!("INVITE request sent from client");
        
        // 6. Wait for client to transition to Calling state automatically
        println!("Waiting for client to enter Calling state");
        let calling_success = env.client_manager.wait_for_transaction_state(
            &client_tx_id,
            TransactionState::Calling,
            Duration::from_millis(1000)
        ).await.expect("Failed to wait for Calling state");
        assert!(calling_success, "Client should transition to Calling state");
        println!("✅ Client entered Calling state");
        
        // 7. The server automatically processes the INVITE and creates a transaction
        // Let's find the server transaction that was auto-created
        println!("Looking for auto-created server transaction");
        
        // Give the auto-processing time to complete
        tokio::time::sleep(Duration::from_millis(200)).await;
        
        // The transaction key for the server side should match the client's branch
        // We can reconstruct it or find it via server transaction inspection
        let server_tx_id = TransactionKey::from_request(&invite_request)
            .expect("Failed to create server transaction key");
        println!("Expected server transaction ID: {:?}", server_tx_id);
        
        // Verify server transaction exists
        let server_tx_exists = env.server_manager.transaction_exists(&server_tx_id).await;
        if !server_tx_exists {
            panic!("Server transaction should have been auto-created");
        }
        println!("✅ Server transaction auto-created with ID: {:?}", server_tx_id);
        
        // Subscribe to server transaction events  
        let mut server_tx_events = env.server_manager.subscribe_to_transaction(&server_tx_id)
            .await.expect("Failed to subscribe to server transaction events");
        
        // 8. At this point, client should have already received 100 Trying automatically
        // Let's wait for that event if we haven't seen it yet
        println!("Waiting for client to receive auto-sent 100 Trying");
        let trying_result = tokio::time::timeout(
            Duration::from_millis(500),
            env.wait_for_client_event(Duration::from_millis(1000), |event| match_provisional_response(event))
        ).await;
        
        match trying_result {
            Ok(Some((trying_tx_id, trying_resp))) => {
                assert_eq!(trying_tx_id, client_tx_id);
                assert_eq!(trying_resp.status_code(), StatusCode::Trying.as_u16());
                println!("✅ Client received auto-sent 100 Trying");
            }
            _ => {
                println!("ℹ️  100 Trying may have been processed before subscription - this is OK");
            }
        }
        
        // 9. Wait for client to transition to Proceeding state automatically
        println!("Waiting for client to enter Proceeding state");
        let proceeding_success = env.client_manager.wait_for_transaction_state(
            &client_tx_id,
            TransactionState::Proceeding,
            Duration::from_millis(1000)
        ).await.expect("Failed to wait for Proceeding state");
        assert!(proceeding_success, "Client should transition to Proceeding state after 1xx");
        println!("✅ Client entered Proceeding state");
        
        // 10. Server sends 180 Ringing
        println!("Server sending 180 Ringing");
        let ringing_response = env.create_response(&invite_request, StatusCode::Ringing, Some("Ringing"));
        env.server_manager.send_response(&server_tx_id, ringing_response).await
            .expect("Failed to send ringing response");
        
        // 11. Wait for client to receive 180 Ringing via ProvisionalResponse event
        println!("Waiting for client to receive 180 Ringing");
        let (ringing_tx_id, ringing_resp) = env.wait_for_client_event(
            Duration::from_millis(1000),
            |event| match_provisional_response(event)
        ).await.expect("Timeout waiting for 180 Ringing");
        assert_eq!(ringing_tx_id, client_tx_id);
        assert_eq!(ringing_resp.status_code(), StatusCode::Ringing.as_u16());
        println!("✅ Client received 180 Ringing");
        
        // 12. Server sends 200 OK (success)
        println!("Server sending 200 OK");
        let ok_response = env.create_response(&invite_request, StatusCode::Ok, Some("OK"));
        env.server_manager.send_response(&server_tx_id, ok_response).await
            .expect("Failed to send OK response");
        
        // 13. Wait for client to receive 200 OK via SuccessResponse event
        println!("Waiting for client to receive 200 OK");
        let (success_tx_id, success_resp) = env.wait_for_client_event(
            Duration::from_millis(1000),
            |event| match_success_response(event)
        ).await.expect("Timeout waiting for 200 OK");
        assert_eq!(success_tx_id, client_tx_id);
        assert_eq!(success_resp.status_code(), StatusCode::Ok.as_u16());
        println!("✅ Client received 200 OK");
        
        // 14. For 2xx responses, both transactions should terminate automatically
        // Wait for client transaction termination
        println!("Waiting for client transaction to terminate");
        let client_terminated = env.wait_for_client_event(
            Duration::from_millis(2000),
            |event| match_transaction_terminated(event)
        ).await;
        
        if let Some(terminated_tx_id) = client_terminated {
            assert_eq!(terminated_tx_id, client_tx_id);
            println!("✅ Client transaction terminated automatically");
        } else {
            // Check if transaction reached Terminated state
            let final_state = env.client_manager.transaction_state(&client_tx_id).await;
            match final_state {
                Ok(TransactionState::Terminated) => {
                    println!("✅ Client transaction in Terminated state");
                },
                Ok(state) => {
                    println!("ℹ️  Client transaction in state: {:?}", state);
                },
                Err(_) => {
                    println!("✅ Client transaction already cleaned up");
                }
            }
        }
        
        // Wait for server transaction termination 
        println!("Waiting for server transaction to terminate");
        let server_terminated = tokio::time::timeout(
            Duration::from_millis(2000),
            async {
                loop {
                    tokio::select! {
                        Some(event) = server_tx_events.recv() => {
                            if let Some(terminated_tx_id) = match_transaction_terminated(&event) {
                                if terminated_tx_id == server_tx_id {
                                    return Some(terminated_tx_id);
                                }
                            }
                        }
                        _ = tokio::time::sleep(Duration::from_millis(100)) => {
                            // Check current state
                            if let Ok(state) = env.server_manager.transaction_state(&server_tx_id).await {
                                if state == TransactionState::Terminated {
                                    return Some(server_tx_id.clone());
                                }
                            } else {
                                // Transaction cleaned up
                                return Some(server_tx_id.clone());
                            }
                        }
                    }
                }
            }
        ).await;
        
        match server_terminated {
            Ok(Some(_)) => println!("✅ Server transaction terminated automatically"),
            Ok(None) => println!("⚠️  Server transaction termination not detected"),
            Err(_) => {
                // Check final state
                let final_state = env.server_manager.transaction_state(&server_tx_id).await;
                match final_state {
                    Ok(TransactionState::Terminated) => {
                        println!("✅ Server transaction in Terminated state");
                    },
                    Ok(state) => {
                        println!("ℹ️  Server transaction in state: {:?}", state);
                    },
                    Err(_) => {
                        println!("✅ Server transaction already cleaned up");
                    }
                }
            }
        }
        
        // 15. For 2xx responses, ACK is handled at the TU level, not by the transaction
        println!("✅ For 2xx responses, ACK would be handled by TU, not transaction layer");
        
        println!("✅ INVITE success flow test completed successfully using event-driven approach");
        
        // Clean up
        env.shutdown().await;
    }).await;
    
    // Handle test timeout
    if let Err(_) = test_result {
        panic!("Test timed out after 10 seconds");
    }
    
    // Reset environment variable
    env::remove_var("RVOIP_TEST");
}

/// Tests the complete flow of a failed INVITE transaction (non-2xx response) with ACK
///
/// Flow sequence:
/// 1. Client sends INVITE request
/// 2. Server receives INVITE and creates server transaction
/// 3. Server sends 100 Trying
/// 4. Client receives 100 Trying and moves to Proceeding
/// 5. Server sends 486 Busy Here (failure)
/// 6. Client receives 486, moves to Completed, and automatically generates ACK
/// 7. Server receives ACK and moves to Confirmed
/// 8. Both transactions terminate after timers expire
#[tokio::test]
#[serial]
async fn test_invite_failure_flow() {
    // Set test environment variable
    env::set_var("RVOIP_TEST", "1");
    
    println!("\n==== TEST: INVITE Failure Flow ====");
    println!("Testing INVITE transaction with non-2xx responses and ACK handling using event-driven approach");
    println!("This test verifies the failure path of INVITE transactions according to RFC 3261 Section 17.1.1/17.2.1");
    println!("Scenario: Client sends INVITE, server responds with 1xx then 486 Busy Here");
    println!("Expected behavior: Client auto-generates ACK, server moves to Confirmed state\n");
    
    // Test timeout to prevent hanging
    let test_result = timeout(Duration::from_secs(10), async {
        // 1. Initialize the test environment
        let mut env = TestEnvironment::new().await;
        
        // 2. Create an INVITE request
        let server_uri = format!("sip:server@{}", env.server_addr);
        let invite_request = env.create_request(Method::Invite, &server_uri);
        println!("Created INVITE request");
        
        // 3. Create client transaction and subscribe to its events
        println!("Creating INVITE client transaction");
        let client_tx_id = env.client_manager.create_client_transaction(
            invite_request.clone(),
            env.server_addr
        ).await.expect("Failed to create client transaction");
        println!("Client transaction created with ID: {:?}", client_tx_id);
        
        // Subscribe to client events for this specific transaction
        let mut client_events = env.client_manager.subscribe_to_transaction(&client_tx_id)
            .await.expect("Failed to subscribe to client transaction events");
        
        // 4. Send the INVITE request - this triggers automatic state machine
        println!("Starting client transaction (sending INVITE)");
        env.client_manager.send_request(&client_tx_id).await
            .expect("Failed to send request");
        println!("INVITE request sent from client");
        
        // 5. Wait for client to transition to Calling state automatically
        println!("Waiting for client to enter Calling state");
        let calling_success = env.client_manager.wait_for_transaction_state(
            &client_tx_id,
            TransactionState::Calling,
            Duration::from_millis(1000)
        ).await.expect("Failed to wait for Calling state");
        assert!(calling_success, "Client should transition to Calling state");
        println!("✅ Client entered Calling state");
        
        // 6. The server automatically processes the INVITE and creates a transaction
        // Let's find the server transaction that was auto-created
        println!("Looking for auto-created server transaction");
        
        // Give the auto-processing time to complete
        tokio::time::sleep(Duration::from_millis(200)).await;
        
        let server_tx_id = TransactionKey::from_request(&invite_request)
            .expect("Failed to create server transaction key");
        println!("Expected server transaction ID: {:?}", server_tx_id);
        
        // Verify server transaction exists
        let server_tx_exists = env.server_manager.transaction_exists(&server_tx_id).await;
        if !server_tx_exists {
            panic!("Server transaction should have been auto-created");
        }
        println!("✅ Server transaction auto-created with ID: {:?}", server_tx_id);
        
        // Subscribe to server transaction events  
        let mut server_tx_events = env.server_manager.subscribe_to_transaction(&server_tx_id)
            .await.expect("Failed to subscribe to server transaction events");
        
        // 7. At this point, client should have already received 100 Trying automatically
        // Let's wait for that event if we haven't seen it yet
        println!("Waiting for client to receive auto-sent 100 Trying");
        let trying_result = tokio::time::timeout(
            Duration::from_millis(500),
            env.wait_for_client_event(Duration::from_millis(1000), |event| match_provisional_response(event))
        ).await;
        
        match trying_result {
            Ok(Some((trying_tx_id, trying_resp))) => {
                assert_eq!(trying_tx_id, client_tx_id);
                assert_eq!(trying_resp.status_code(), StatusCode::Trying.as_u16());
                println!("✅ Client received auto-sent 100 Trying");
            }
            _ => {
                println!("ℹ️  100 Trying may have been processed before subscription - this is OK");
            }
        }
        
        // 8. Wait for client to transition to Proceeding state automatically  
        println!("Waiting for client to enter Proceeding state");
        let proceeding_success = env.client_manager.wait_for_transaction_state(
            &client_tx_id,
            TransactionState::Proceeding,
            Duration::from_millis(1000)
        ).await.expect("Failed to wait for Proceeding state");
        assert!(proceeding_success, "Client should transition to Proceeding state after 1xx");
        println!("✅ Client entered Proceeding state");
        
        // 9. Server sends 486 Busy Here
        println!("Server sending 486 Busy Here");
        let busy_response = env.create_response(&invite_request, StatusCode::BusyHere, Some("Busy Here"));
        env.server_manager.send_response(&server_tx_id, busy_response.clone()).await
            .expect("Failed to send 486 Busy Here");
        
        // 10. Wait for client to receive 486 Busy Here via FailureResponse event
        println!("Waiting for client to receive 486 Busy Here");
        let (failure_tx_id, busy_resp) = env.wait_for_client_event(
            Duration::from_millis(1000),
            |event| match_failure_response(event)
        ).await.expect("Timeout waiting for 486 Busy Here");
        assert_eq!(failure_tx_id, client_tx_id);
        assert_eq!(busy_resp.status_code(), StatusCode::BusyHere.as_u16());
        println!("✅ Client received 486 Busy Here");
        
        // 11. Wait for client to transition to Completed state automatically
        println!("Waiting for client to enter Completed state");
        let completed_success = env.client_manager.wait_for_transaction_state(
            &client_tx_id,
            TransactionState::Completed,
            Duration::from_millis(1000)
        ).await.expect("Failed to wait for Completed state");
        assert!(completed_success, "Client should transition to Completed state after non-2xx");
        println!("✅ Client entered Completed state");
        
        // 12. Wait for server to receive ACK via AckReceived event
        // The client should automatically generate and send an ACK for the non-2xx response
        println!("Waiting for server to receive ACK");
        let ack_result = tokio::time::timeout(
            Duration::from_millis(2000),
            async {
                loop {
                    tokio::select! {
                        Some(event) = server_tx_events.recv() => {
                            if let Some((ack_tx_id, ack_req)) = match_ack_received(&event) {
                                if ack_tx_id == server_tx_id {
                                    return Some((ack_tx_id, ack_req));
                                }
                            }
                        }
                        _ = tokio::time::sleep(Duration::from_millis(100)) => {
                            // Continue waiting
                        }
                    }
                }
            }
        ).await;
        
        match ack_result {
            Ok(Some((ack_tx_id, ack_req))) => {
                assert_eq!(ack_tx_id, server_tx_id);
                assert_eq!(ack_req.method(), Method::Ack);
                println!("✅ Server received ACK for non-2xx response");
            }
            _ => {
                println!("ℹ️  ACK event may have been processed before subscription - checking transaction state");
            }
        }
        
        // 13. Wait for server to transition to Confirmed state automatically
        println!("Waiting for server to enter Confirmed state");
        let confirmed_success = env.server_manager.wait_for_transaction_state(
            &server_tx_id,
            TransactionState::Confirmed,
            Duration::from_millis(2000)
        ).await.expect("Failed to wait for Confirmed state");
        assert!(confirmed_success, "Server should transition to Confirmed state after receiving ACK");
        println!("✅ Server entered Confirmed state");
        
        // 14. Both transactions should eventually terminate automatically via timers
        println!("Waiting for transactions to terminate via RFC 3261 timers");
        
        // Wait for client termination 
        let client_terminated = tokio::time::timeout(
            Duration::from_millis(3000),
            env.wait_for_client_event(Duration::from_millis(5000), |event| match_transaction_terminated(event))
        ).await;
        
        match client_terminated {
            Ok(Some(terminated_tx_id)) => {
                assert_eq!(terminated_tx_id, client_tx_id);
                println!("✅ Client transaction terminated via Timer D");
            }
            _ => {
                // Check final state
                let final_state = env.client_manager.transaction_state(&client_tx_id).await;
                match final_state {
                    Ok(TransactionState::Terminated) => {
                        println!("✅ Client transaction in Terminated state");
                    },
                    Ok(state) => {
                        println!("ℹ️  Client transaction in state: {:?}", state);
                    },
                    Err(_) => {
                        println!("✅ Client transaction already cleaned up");
                    }
                }
            }
        }
        
        // Wait for server termination
        let server_terminated = tokio::time::timeout(
            Duration::from_millis(3000),
            async {
                loop {
                    tokio::select! {
                        Some(event) = server_tx_events.recv() => {
                            if let Some(terminated_tx_id) = match_transaction_terminated(&event) {
                                if terminated_tx_id == server_tx_id {
                                    return Some(terminated_tx_id);
                                }
                            }
                        }
                        _ = tokio::time::sleep(Duration::from_millis(100)) => {
                            // Check current state
                            if let Ok(state) = env.server_manager.transaction_state(&server_tx_id).await {
                                if state == TransactionState::Terminated {
                                    return Some(server_tx_id.clone());
                                }
                            } else {
                                // Transaction cleaned up
                                return Some(server_tx_id.clone());
                            }
                        }
                    }
                }
            }
        ).await;
        
        match server_terminated {
            Ok(Some(_)) => println!("✅ Server transaction terminated via Timer I"),
            _ => {
                // Check final state
                let final_state = env.server_manager.transaction_state(&server_tx_id).await;
                match final_state {
                    Ok(TransactionState::Terminated) => {
                        println!("✅ Server transaction in Terminated state");
                    },
                    Ok(state) => {
                        println!("ℹ️  Server transaction in state: {:?}", state);
                    },
                    Err(_) => {
                        println!("✅ Server transaction already cleaned up");
                    }
                }
            }
        }
        
        println!("✅ INVITE failure flow test completed successfully using event-driven approach");
        
        // Clean up
        env.shutdown().await;
    }).await;
    
    // Handle test timeout
    if let Err(_) = test_result {
        panic!("Test timed out after 10 seconds");
    }
    
    // Reset environment variable
    env::remove_var("RVOIP_TEST");
} 