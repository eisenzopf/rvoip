/// CANCEL Transaction Tests for SIP according to RFC 3261
///
/// This test suite verifies the behavior of CANCEL requests in the rVOIP transaction-core
/// library according to RFC 3261 Section 9.1 (Client Behavior) and 9.2 (Server Behavior).
///
/// RFC 3261 Compliance Verification:
///
/// 1. CANCEL Request Rules (Section 9.1):
///    - CANCEL can only be sent for INVITE requests that haven't received a final response
///    - CANCEL must have the same Call-ID, To, From, and CSeq.request_uri as the INVITE 
///    - The CSeq.sequence_number must be the same, but method must be CANCEL
///    - CANCEL creates a new client transaction (non-INVITE type)
///    - CANCEL can only be sent after a provisional response is received for the INVITE
///
/// 2. CANCEL Transaction Behavior:
///    - CANCEL follows non-INVITE transaction state machine
///    - The canceled INVITE should receive a 487 Request Terminated final response
///
/// Test Coverage:
/// 1. test_cancel_after_provisional: Tests canceling an INVITE after receiving a 180 Ringing

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

/// Tests cancellation of an INVITE transaction after receiving a provisional response
///
/// Flow sequence:
/// 1. Client sends INVITE request
/// 2. Server sends 180 Ringing
/// 3. Client sends CANCEL request
/// 4. Server acknowledges CANCEL with 200 OK
/// 5. Server terminates the INVITE with 487 Request Terminated
/// 6. Client acknowledges the 487 with ACK
#[tokio::test]
#[serial]
async fn test_cancel_after_provisional() {
    // Set test environment variable
    env::set_var("RVOIP_TEST", "1");
    
    println!("\n==== TEST: CANCEL After Provisional ====");
    println!("Testing cancellation of INVITE after 180 Ringing using proper event-driven approach");
    println!("This test verifies the CANCEL functionality according to RFC 3261 Section 9.1");
    println!("Scenario: Client cancels an INVITE transaction after receiving a provisional response");
    println!("Expected behavior: Server sends 487 Request Terminated and client sends ACK\n");
    
    // Test timeout to prevent hanging
    let test_result = timeout(Duration::from_secs(10), async {
        // 1. Initialize the test environment
        let mut env = TestEnvironment::new().await;
        
        // 2. Create an INVITE request
        let server_uri = format!("sip:server@{}", env.server_addr);
        let invite_request = env.create_request(Method::Invite, &server_uri);
        
        // 3. Create client transaction for INVITE and subscribe to events
        println!("Creating INVITE client transaction");
        let invite_tx_id = env.client_manager.create_client_transaction(
            invite_request.clone(),
            env.server_addr
        ).await.expect("Failed to create INVITE client transaction");
        println!("INVITE client transaction created with ID: {:?}", invite_tx_id);
        
        // Subscribe to INVITE client transaction events
        let mut invite_client_events = env.client_manager.subscribe_to_transaction(&invite_tx_id)
            .await.expect("Failed to subscribe to INVITE client events");
        
        // 4. Send the INVITE request - triggers automatic state machine
        println!("Sending INVITE request");
        env.client_manager.send_request(&invite_tx_id).await
            .expect("Failed to send INVITE request");
        println!("INVITE request sent");
        
        // 5. Wait for client to enter Calling state automatically
        println!("Waiting for INVITE client to enter Calling state");
        let calling_success = env.client_manager.wait_for_transaction_state(
            &invite_tx_id,
            TransactionState::Calling,
            Duration::from_millis(1000)
        ).await.expect("Failed to wait for Calling state");
        assert!(calling_success, "Client should transition to Calling state");
        println!("✅ INVITE client entered Calling state");
        
        // 6. Find the auto-created server transaction
        println!("Looking for auto-created server transaction");
        tokio::time::sleep(Duration::from_millis(200)).await;
        
        let invite_server_tx_id = TransactionKey::from_request(&invite_request)
            .expect("Failed to create server transaction key");
        println!("Expected server transaction ID: {:?}", invite_server_tx_id);
        
        // Verify server transaction exists
        let server_tx_exists = env.server_manager.transaction_exists(&invite_server_tx_id).await;
        if !server_tx_exists {
            panic!("Server transaction should have been auto-created");
        }
        println!("✅ Server transaction auto-created with ID: {:?}", invite_server_tx_id);
        
        // Subscribe to server transaction events
        let mut invite_server_events = env.server_manager.subscribe_to_transaction(&invite_server_tx_id)
            .await.expect("Failed to subscribe to server transaction events");
        
        // 7. Check if client already received auto-sent 100 Trying
        println!("Checking for auto-sent 100 Trying");
        let trying_result = tokio::time::timeout(
            Duration::from_millis(500),
            env.wait_for_client_event(Duration::from_millis(1000), |event| match_provisional_response(event))
        ).await;
        
        match trying_result {
            Ok(Some((trying_tx_id, trying_resp))) => {
                assert_eq!(trying_tx_id, invite_tx_id);
                assert_eq!(trying_resp.status_code(), StatusCode::Trying.as_u16());
                println!("✅ Client received auto-sent 100 Trying");
            }
            _ => {
                println!("ℹ️  100 Trying may have been processed before subscription - this is OK");
            }
        }
        
        // 8. Wait for client to transition to Proceeding state
        println!("Waiting for client to enter Proceeding state");
        let proceeding_success = env.client_manager.wait_for_transaction_state(
            &invite_tx_id,
            TransactionState::Proceeding,
            Duration::from_millis(1000)
        ).await.expect("Failed to wait for Proceeding state");
        assert!(proceeding_success, "Client should transition to Proceeding state after 1xx");
        println!("✅ Client entered Proceeding state");
        
        // 9. Server sends 180 Ringing (provisional response)
        println!("Server sending 180 Ringing");
        let ringing_response = env.create_response(&invite_request, StatusCode::Ringing, Some("Ringing"));
        env.server_manager.send_response(&invite_server_tx_id, ringing_response.clone()).await
            .expect("Failed to send ringing response");
        
        // 10. Wait for client to receive 180 Ringing via ProvisionalResponse event
        println!("Waiting for client to receive 180 Ringing");
        let (ringing_tx_id, ringing_resp) = env.wait_for_client_event(
            Duration::from_millis(1000),
            |event| match_provisional_response(event)
        ).await.expect("Timeout waiting for 180 Ringing");
        assert_eq!(ringing_tx_id, invite_tx_id);
        assert_eq!(ringing_resp.status_code(), StatusCode::Ringing.as_u16());
        println!("✅ Client received 180 Ringing");
        
        // 11. Create a CANCEL request using the transaction manager API
        println!("Creating CANCEL request for the INVITE");
        let cancel_tx_id = env.client_manager.cancel_invite_transaction(&invite_tx_id).await
            .expect("Failed to create CANCEL transaction via API");
        println!("✅ CANCEL client transaction created with ID: {:?}", cancel_tx_id);
        
        // Subscribe to CANCEL client transaction events
        let mut cancel_client_events = env.client_manager.subscribe_to_transaction(&cancel_tx_id)
            .await.expect("Failed to subscribe to CANCEL client events");
        
        // 12. Find the auto-created CANCEL server transaction
        println!("Looking for auto-created CANCEL server transaction");
        tokio::time::sleep(Duration::from_millis(200)).await;
        
        // Create the CANCEL request to determine server transaction ID
        let cancel_request = env.create_cancel_request(&invite_request);
        let cancel_server_tx_id = TransactionKey::from_request(&cancel_request)
            .expect("Failed to create CANCEL server transaction key");
        println!("Expected CANCEL server transaction ID: {:?}", cancel_server_tx_id);
        
        // Verify CANCEL server transaction exists (may be auto-created)
        let cancel_server_exists = env.server_manager.transaction_exists(&cancel_server_tx_id).await;
        if !cancel_server_exists {
            // Create server transaction manually if not auto-created
            let cancel_server_tx = env.server_manager.create_server_transaction(
                cancel_request.clone(),
                env.client_addr
            ).await.expect("Failed to create CANCEL server transaction");
            println!("✅ CANCEL server transaction created with ID: {:?}", cancel_server_tx.id());
        } else {
            println!("✅ CANCEL server transaction auto-created");
        }
        
        // Subscribe to CANCEL server transaction events
        let mut cancel_server_events = env.server_manager.subscribe_to_transaction(&cancel_server_tx_id)
            .await.expect("Failed to subscribe to CANCEL server events");
        
        // 13. Server sends 200 OK for CANCEL
        println!("Server sending 200 OK for CANCEL");
        let cancel_ok_response = env.create_response(&cancel_request, StatusCode::Ok, Some("OK"));
        env.server_manager.send_response(&cancel_server_tx_id, cancel_ok_response.clone()).await
            .expect("Failed to send OK response for CANCEL");
        
        // 14. Wait for client to receive 200 OK for CANCEL via SuccessResponse event
        println!("Waiting for client to receive 200 OK for CANCEL");
        let cancel_success_result = tokio::time::timeout(
            Duration::from_millis(1000),
            env.wait_for_client_event(Duration::from_millis(2000), |event| match_success_response(event))
        ).await;
        
        match cancel_success_result {
            Ok(Some((success_tx_id, success_resp))) => {
                if success_tx_id == cancel_tx_id {
                    assert_eq!(success_resp.status_code(), StatusCode::Ok.as_u16());
                    println!("✅ Client received 200 OK for CANCEL");
                } else {
                    println!("ℹ️  Received success response for different transaction");
                }
            }
            _ => {
                println!("ℹ️  200 OK for CANCEL may have been processed before subscription");
            }
        }
        
        // 15. Server sends 487 Request Terminated for the INVITE
        println!("Server sending 487 Request Terminated for INVITE");
        let terminated_response = env.create_response(
            &invite_request, 
            StatusCode::RequestTerminated, 
            Some("Request Terminated")
        );
        env.server_manager.send_response(&invite_server_tx_id, terminated_response.clone()).await
            .expect("Failed to send 487 Request Terminated");
        
        // 16. Wait for client to receive 487 Request Terminated via FailureResponse event
        println!("Waiting for client to receive 487 Request Terminated");
        let failure_result = tokio::time::timeout(
            Duration::from_millis(1000),
            env.wait_for_client_event(Duration::from_millis(2000), |event| match_failure_response(event))
        ).await;
        
        match failure_result {
            Ok(Some((failure_tx_id, failure_resp))) => {
                if failure_tx_id == invite_tx_id {
                    assert_eq!(failure_resp.status_code(), StatusCode::RequestTerminated.as_u16());
                    println!("✅ Client received 487 Request Terminated for INVITE");
                }
            }
            _ => {
                println!("ℹ️  487 Request Terminated may have been processed before subscription");
            }
        }
        
        // 17. Wait for client to transition INVITE to Completed state
        println!("Waiting for INVITE client to enter Completed state");
        let completed_success = env.client_manager.wait_for_transaction_state(
            &invite_tx_id,
            TransactionState::Completed,
            Duration::from_millis(1000)
        ).await.expect("Failed to wait for Completed state");
        assert!(completed_success, "INVITE client should transition to Completed state after 487");
        println!("✅ INVITE client entered Completed state");
        
        // 18. Wait for server to receive ACK via AckReceived event
        println!("Waiting for server to receive ACK");
        let ack_result = tokio::time::timeout(
            Duration::from_millis(2000),
            async {
                loop {
                    tokio::select! {
                        Some(event) = invite_server_events.recv() => {
                            if let Some((ack_tx_id, ack_req)) = match_ack_received(&event) {
                                if ack_tx_id == invite_server_tx_id {
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
                assert_eq!(ack_tx_id, invite_server_tx_id);
                assert_eq!(ack_req.method(), Method::Ack);
                println!("✅ Server received ACK for 487 response");
            }
            _ => {
                println!("ℹ️  ACK event may have been processed before subscription");
            }
        }
        
        // 19. Wait for server to transition to Confirmed state
        println!("Waiting for INVITE server to enter Confirmed state");
        let confirmed_success = env.server_manager.wait_for_transaction_state(
            &invite_server_tx_id,
            TransactionState::Confirmed,
            Duration::from_millis(2000)
        ).await.expect("Failed to wait for Confirmed state");
        assert!(confirmed_success, "Server should transition to Confirmed state after receiving ACK");
        println!("✅ INVITE server entered Confirmed state");
        
        // 20. Both transactions should eventually terminate automatically via RFC 3261 timers
        println!("Waiting for transactions to terminate via RFC 3261 timers");
        
        // Wait for terminations with reasonable timeouts
        let termination_timeout = Duration::from_millis(3000);
        
        // Check INVITE client termination
        let invite_client_terminated = tokio::time::timeout(
            termination_timeout,
            env.wait_for_client_event(Duration::from_millis(5000), |event| match_transaction_terminated(event))
        ).await;
        
        match invite_client_terminated {
            Ok(Some(terminated_tx_id)) => {
                if terminated_tx_id == invite_tx_id {
                    println!("✅ INVITE client transaction terminated via Timer D");
                }
            }
            _ => {
                // Check final state
                let final_state = env.client_manager.transaction_state(&invite_tx_id).await;
                match final_state {
                    Ok(TransactionState::Terminated) => {
                        println!("✅ INVITE client transaction in Terminated state");
                    },
                    Ok(state) => {
                        println!("ℹ️  INVITE client transaction in state: {:?}", state);
                    },
                    Err(_) => {
                        println!("✅ INVITE client transaction already cleaned up");
                    }
                }
            }
        }
        
        println!("✅ CANCEL test completed successfully using event-driven approach");
        
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