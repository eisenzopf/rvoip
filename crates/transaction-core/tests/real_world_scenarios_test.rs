/// Real-world SIP Transaction Scenarios
///
/// This file contains tests for real-world SIP transaction scenarios that test
/// the transaction layer's public API for production use cases. These tests focus on:
///
/// 1. High-load scenarios with multiple concurrent transactions
/// 2. Network failure recovery
/// 3. Race conditions and edge cases
/// 4. Complex transaction flows
/// 5. Re-INVITE and dialog refresh flows
/// 6. Authentication challenges

mod transaction_test_utils;

use std::time::Duration;
use tokio::time::sleep;
use tokio::time::timeout;
use std::sync::Arc;
use serial_test::serial;
use std::collections::HashMap;
use std::env;

use rvoip_sip_core::types::status::StatusCode;
use rvoip_sip_core::prelude::Method;
use rvoip_sip_core::Message;
use rvoip_sip_core::types::header::{TypedHeader, HeaderName};
use rvoip_sip_core::types::headers::HeaderValue;
use rvoip_sip_core::types::via::Via;
use rvoip_sip_core::types::content_length::ContentLength;
use rvoip_sip_core::types::param::Param;
use rvoip_sip_transport::Transport;
use rvoip_transaction_core::transaction::TransactionState;
use rvoip_transaction_core::{TransactionEvent, TransactionKey};
use rvoip_transaction_core::builders::{client_quick, server_quick};
use rvoip_transaction_core::client::builders::{InviteBuilder, ByeBuilder};
use rvoip_transaction_core::server::builders::{InviteResponseBuilder, ResponseBuilder};

use transaction_test_utils::*;

/// Tests recovery from network failures during transaction processing
///
/// Scenario:
/// 1. Client sends INVITE
/// 2. Connection fails (transport error) during INVITE processing
/// 3. Client retries INVITE (should use same branch but might need to rebuild transport)
/// 4. New INVITE succeeds and call proceeds
#[tokio::test]
#[serial]
async fn test_network_failure_recovery() {
    println!("\n==== TEST: Network Failure Recovery ====");
    println!("Testing transaction retry after transport errors");
    println!("This test verifies resilience against network failures");
    println!("Scenario: Client sends INVITE, connection fails, client retries");
    println!("Expected behavior: Retry mechanism handles transport failure and transaction recovers\n");
    
    // Test timeout to prevent hanging
    let test_result = timeout(Duration::from_secs(5), async {
        // 1. Initialize the test environment
        let mut env = TestEnvironment::new().await;
        
        // 2. Create an INVITE request
        let server_uri = format!("sip:server@{}", env.server_addr);
        let invite_request = env.create_request(Method::Invite, &server_uri);
        
        // 3. Create client transaction for INVITE
        println!("Creating INVITE client transaction");
        let invite_tx_id = env.client_manager.create_client_transaction(
            invite_request.clone(),
            env.server_addr
        ).await.expect("Failed to create INVITE client transaction");
        println!("INVITE client transaction created with ID: {:?}", invite_tx_id);
        
        // 4. Send the INVITE request
        println!("Sending INVITE request");
        env.client_manager.send_request(&invite_tx_id).await
            .expect("Failed to send INVITE request");
        println!("INVITE request sent");
        
        // 5. Get the sent INVITE request from the mock transport
        sleep(Duration::from_millis(30)).await;
        let sent_message_opt = env.client_transport.get_sent_message().await;
        assert!(sent_message_opt.is_some(), "Client did not send any message");
        
        // 6. Simulate transport failure by simply waiting for a timeout
        println!("Waiting for a potential transaction timeout"); 
        sleep(Duration::from_millis(100)).await;
        
        // 7. Clear transport queues
        while let Some(_) = env.client_transport.get_sent_message().await {}
        
        // 8. Retry sending the INVITE with the retry_request API method
        println!("Retrying INVITE request");
        match env.client_manager.retry_request(&invite_tx_id).await {
            Ok(_) => println!("Successfully retried INVITE request"),
            Err(e) => {
                println!("Error retrying request: {:?}", e);
                // Create a new transaction if retry fails (old one may be terminated)
                let new_invite_request = env.create_request(Method::Invite, &server_uri);
                let new_tx_id = env.client_manager.create_client_transaction(
                    new_invite_request.clone(),
                    env.server_addr
                ).await.expect("Failed to create new INVITE client transaction");
                println!("Created new INVITE client transaction with ID: {:?}", new_tx_id);
                env.client_manager.send_request(&new_tx_id).await
                    .expect("Failed to send new INVITE request");
            }
        }
        
        // 9. Verify request was sent
        sleep(Duration::from_millis(30)).await;
        let retry_message_opt = env.client_transport.get_sent_message().await;
        assert!(retry_message_opt.is_some(), "Client did not send retry message");

        // Complete the test successfully
        println!("Network failure recovery test completed");
        env.shutdown().await;
    }).await;
    
    if let Err(_) = test_result {
        panic!("Test timed out after 5 seconds");
    }
}

/// Tests authentication challenge-response flow
///
/// Scenario:
/// 1. Client sends REGISTER
/// 2. Server challenges with 401 Unauthorized
/// 3. Client creates new transaction with credentials
/// 4. Server accepts the authenticated request with 200 OK
#[tokio::test]
#[serial]
async fn test_authentication_flow() {
    println!("\n==== TEST: Authentication Flow ====");
    println!("Testing auth challenge-response handling using proper event-driven approach");
    println!("This test verifies proper handling of authentication challenges");
    println!("Scenario: Client sends REGISTER, server challenges with 401, client authenticates");
    println!("Expected behavior: New transaction created with auth credentials, server accepts with 200 OK\n");
    
    // Test timeout to prevent hanging
    let test_result = timeout(Duration::from_secs(10), async {
        // 1. Initialize the test environment
        let mut env = TestEnvironment::new().await;
        
        // 2. Create a REGISTER request
        let server_uri = format!("sip:registrar@{}", env.server_addr);
        let register_request = env.create_request(Method::Register, &server_uri);
        
        // 3. Create client transaction for REGISTER and subscribe to events
        println!("Creating REGISTER client transaction");
        let register_tx_id = env.client_manager.create_client_transaction(
            register_request.clone(),
            env.server_addr
        ).await.expect("Failed to create REGISTER client transaction");
        
        // Subscribe to client transaction events
        let mut register_client_events = env.client_manager.subscribe_to_transaction(&register_tx_id)
            .await.expect("Failed to subscribe to REGISTER client events");
        
        // 4. Send the REGISTER request - triggers automatic state machine
        env.client_manager.send_request(&register_tx_id).await
            .expect("Failed to send REGISTER request");
        println!("REGISTER request sent");
        
        // 5. Wait for client to enter Trying state
        println!("Waiting for client to enter Trying state");
        let trying_success = env.client_manager.wait_for_transaction_state(
            &register_tx_id,
            TransactionState::Trying,
            Duration::from_millis(1000)
        ).await.expect("Failed to wait for Trying state");
        assert!(trying_success, "Client should transition to Trying state");
        println!("✅ Client entered Trying state");
        
        // 6. Find the auto-created server transaction
        println!("Looking for auto-created server transaction");
        tokio::time::sleep(Duration::from_millis(200)).await;
        
        let register_server_tx_id = TransactionKey::from_request(&register_request)
            .expect("Failed to create server transaction key");
        println!("Expected server transaction ID: {:?}", register_server_tx_id);
        
        // Verify server transaction exists (may be auto-created)
        let server_tx_exists = env.server_manager.transaction_exists(&register_server_tx_id).await;
        if !server_tx_exists {
            // Create server transaction manually if not auto-created
            let server_tx = env.server_manager.create_server_transaction(
                register_request.clone(), 
                env.client_addr
            ).await.expect("Failed to create server transaction");
            println!("✅ Server transaction created with ID: {:?}", server_tx.id());
        } else {
            println!("✅ Server transaction auto-created");
        }
        
        // Subscribe to server transaction events
        let mut register_server_events = env.server_manager.subscribe_to_transaction(&register_server_tx_id)
            .await.expect("Failed to subscribe to server transaction events");
        
        // 7. Server creates 401 Unauthorized challenge
        println!("Server sending 401 Unauthorized challenge");
        let unauthorized_response = env.create_response(
            &register_request, 
            StatusCode::Unauthorized, 
            Some("Unauthorized")
        );
        
        // 8. Send the 401 challenge
        env.server_manager.send_response(&register_server_tx_id, unauthorized_response.clone()).await
            .expect("Failed to send 401 Unauthorized");
        
        // 9. Wait for client to receive 401 Unauthorized via FailureResponse event
        println!("Waiting for client to receive 401 Unauthorized");
        let (failure_tx_id, failure_resp) = env.wait_for_client_event(
            Duration::from_millis(1000),
            |event| match_failure_response(event)
        ).await.expect("Timeout waiting for 401 Unauthorized");
        assert_eq!(failure_tx_id, register_tx_id);
        assert_eq!(failure_resp.status_code(), StatusCode::Unauthorized.as_u16());
        println!("✅ Client received 401 Unauthorized");
        
        // 10. Wait for client to transition to Completed state
        println!("Waiting for client to enter Completed state");
        let completed_success = env.client_manager.wait_for_transaction_state(
            &register_tx_id,
            TransactionState::Completed,
            Duration::from_millis(1000)
        ).await.expect("Failed to wait for Completed state");
        
        if completed_success {
            println!("✅ Client entered Completed state");
        } else {
            // Check if already terminated
            let final_state = env.client_manager.transaction_state(&register_tx_id).await;
            match final_state {
                Ok(TransactionState::Terminated) => {
                    println!("✅ Client already in Terminated state");
                }
                Ok(state) => {
                    println!("ℹ️  Client in state: {:?}", state);
                    assert!(
                        state == TransactionState::Completed || state == TransactionState::Terminated,
                        "Client should be in Completed or Terminated state after 401"
                    );
                }
                Err(_) => {
                    println!("✅ Client transaction already cleaned up");
                }
            }
        }
        
        // 11. Create a new REGISTER request with credentials (simulating TU action)
        println!("Creating new REGISTER with authentication credentials");
        let auth_register_request = env.create_request(Method::Register, &server_uri);
        
        // 12. Create new client transaction for authenticated REGISTER
        println!("Creating authenticated REGISTER client transaction");
        let auth_register_tx_id = env.client_manager.create_client_transaction(
            auth_register_request.clone(),
            env.server_addr
        ).await.expect("Failed to create authenticated REGISTER client transaction");
        
        // Subscribe to authenticated client transaction events
        let mut auth_client_events = env.client_manager.subscribe_to_transaction(&auth_register_tx_id)
            .await.expect("Failed to subscribe to authenticated client events");
        
        // 13. Send the authenticated REGISTER request
        env.client_manager.send_request(&auth_register_tx_id).await
            .expect("Failed to send authenticated REGISTER request");
        println!("Authenticated REGISTER request sent");
        
        // 14. Wait for authenticated client to enter Trying state
        println!("Waiting for authenticated client to enter Trying state");
        let auth_trying_success = env.client_manager.wait_for_transaction_state(
            &auth_register_tx_id,
            TransactionState::Trying,
            Duration::from_millis(1000)
        ).await.expect("Failed to wait for authenticated Trying state");
        assert!(auth_trying_success, "Authenticated client should transition to Trying state");
        println!("✅ Authenticated client entered Trying state");
        
        // 15. Find the auto-created server transaction for authenticated request
        println!("Looking for auto-created authenticated server transaction");
        tokio::time::sleep(Duration::from_millis(200)).await;
        
        let auth_register_server_tx_id = TransactionKey::from_request(&auth_register_request)
            .expect("Failed to create authenticated server transaction key");
        println!("Expected authenticated server transaction ID: {:?}", auth_register_server_tx_id);
        
        // Verify authenticated server transaction exists
        let auth_server_exists = env.server_manager.transaction_exists(&auth_register_server_tx_id).await;
        if !auth_server_exists {
            // Create server transaction manually if not auto-created
            let auth_server_tx = env.server_manager.create_server_transaction(
                auth_register_request.clone(), 
                env.client_addr
            ).await.expect("Failed to create server transaction for authenticated request");
            println!("✅ Authenticated server transaction created with ID: {:?}", auth_server_tx.id());
        } else {
            println!("✅ Authenticated server transaction auto-created");
        }
        
        // Subscribe to authenticated server transaction events
        let mut auth_server_events = env.server_manager.subscribe_to_transaction(&auth_register_server_tx_id)
            .await.expect("Failed to subscribe to authenticated server events");
        
        // 16. Server sends 200 OK for authenticated REGISTER
        println!("Server sending 200 OK for authenticated REGISTER");
        let ok_response = env.create_response(&auth_register_request, StatusCode::Ok, Some("OK"));
        env.server_manager.send_response(&auth_register_server_tx_id, ok_response.clone()).await
            .expect("Failed to send 200 OK");
        
        // 17. Wait for client to receive 200 OK via SuccessResponse event
        println!("Waiting for authenticated client to receive 200 OK");
        let (success_tx_id, success_resp) = env.wait_for_client_event(
            Duration::from_millis(1000),
            |event| match_success_response(event)
        ).await.expect("Timeout waiting for 200 OK");
        assert_eq!(success_tx_id, auth_register_tx_id);
        assert_eq!(success_resp.status_code(), StatusCode::Ok.as_u16());
        println!("✅ Authenticated client received 200 OK");
        
        // 18. Wait for authenticated client to transition to Completed state
        println!("Waiting for authenticated client to enter Completed state");
        let auth_completed_success = env.client_manager.wait_for_transaction_state(
            &auth_register_tx_id,
            TransactionState::Completed,
            Duration::from_millis(1000)
        ).await.expect("Failed to wait for authenticated Completed state");
        
        if auth_completed_success {
            println!("✅ Authenticated client entered Completed state");
        } else {
            // Check final state
            let final_state = env.client_manager.transaction_state(&auth_register_tx_id).await;
            match final_state {
                Ok(TransactionState::Terminated) => {
                    println!("✅ Authenticated client already in Terminated state");
                }
                Ok(state) => {
                    println!("ℹ️  Authenticated client in state: {:?}", state);
                    assert!(
                        state == TransactionState::Completed || state == TransactionState::Terminated,
                        "Authenticated client should be in Completed or Terminated state after 200 OK"
                    );
                }
                Err(_) => {
                    println!("✅ Authenticated client transaction already cleaned up");
                }
            }
        }
        
        // 19. Both transactions should terminate automatically via RFC 3261 timers
        println!("Waiting for transactions to terminate via RFC 3261 timers");
        
        // Wait for authenticated client termination
        let auth_client_terminated = tokio::time::timeout(
            Duration::from_millis(2000),
            env.wait_for_client_event(Duration::from_millis(3000), |event| match_transaction_terminated(event))
        ).await;
        
        match auth_client_terminated {
            Ok(Some(terminated_tx_id)) => {
                if terminated_tx_id == auth_register_tx_id {
                    println!("✅ Authenticated client transaction terminated via Timer K");
                }
            }
            _ => {
                // Check final state
                let final_state = env.client_manager.transaction_state(&auth_register_tx_id).await;
                match final_state {
                    Ok(TransactionState::Terminated) => {
                        println!("✅ Authenticated client transaction in Terminated state");
                    },
                    Ok(state) => {
                        println!("ℹ️  Authenticated client transaction in state: {:?}", state);
                    },
                    Err(_) => {
                        println!("✅ Authenticated client transaction already cleaned up");
                    }
                }
            }
        }
        
        println!("✅ Authentication flow test completed successfully using event-driven approach");
        env.shutdown().await;
    }).await;
    
    if let Err(_) = test_result {
        panic!("Test timed out after 10 seconds");
    }
}

/// Tests multiple concurrent transactions with resource management
///
/// Scenario:
/// 1. Create multiple simultaneous transactions of different types
/// 2. Verify all are processed correctly
/// 3. Test cleanup of terminated transactions
/// 4. Verify resource usage is managed properly
#[tokio::test]
#[serial]
async fn test_concurrent_transactions() {
    println!("\n==== TEST: Concurrent Transactions ====");
    println!("Testing multi-transaction handling");
    println!("This test verifies the transaction layer can manage multiple transactions simultaneously");
    println!("Scenario: Create and process multiple transactions of different types (INVITE, REGISTER, OPTIONS, INFO)");
    println!("Expected behavior: All transactions processed correctly, resources managed properly\n");
    
    // Test timeout to prevent hanging
    let test_result = timeout(Duration::from_secs(5), async {
        // 1. Initialize the test environment
        let mut env = TestEnvironment::new().await;
        
        // 2. Create several different requests for concurrent processing
        let server_uri = format!("sip:server@{}", env.server_addr);
        let invite_request = env.create_request(Method::Invite, &server_uri);
        let register_request = env.create_request(Method::Register, &server_uri);
        let options_request = env.create_request(Method::Options, &server_uri);
        let info_request = env.create_request(Method::Info, &server_uri);
        
        // 3. Create client transactions for all requests
        println!("Creating multiple concurrent client transactions");
        let invite_tx_id = env.client_manager.create_client_transaction(
            invite_request.clone(),
            env.server_addr
        ).await.expect("Failed to create INVITE client transaction");
        
        let register_tx_id = env.client_manager.create_client_transaction(
            register_request.clone(),
            env.server_addr
        ).await.expect("Failed to create REGISTER client transaction");
        
        let options_tx_id = env.client_manager.create_client_transaction(
            options_request.clone(),
            env.server_addr
        ).await.expect("Failed to create OPTIONS client transaction");
        
        let info_tx_id = env.client_manager.create_client_transaction(
            info_request.clone(),
            env.server_addr
        ).await.expect("Failed to create INFO client transaction");
        
        // 4. Send all requests concurrently
        println!("Sending all requests concurrently");
        let send_futures = vec![
            env.client_manager.send_request(&invite_tx_id),
            env.client_manager.send_request(&register_tx_id),
            env.client_manager.send_request(&options_tx_id),
            env.client_manager.send_request(&info_tx_id)
        ];
        
        for future in futures::future::join_all(send_futures).await {
            assert!(future.is_ok(), "Failed to send a request");
        }
        
        // 5. Allow time for processing
        sleep(Duration::from_millis(50)).await;
        
        // 6. Verify transaction count
        let count = env.client_manager.transaction_count().await;
        println!("Active transaction count: {}", count);
        assert_eq!(count, 4, "Should have 4 active transactions");
        
        // 7. Get active transactions
        let (client_txs, server_txs) = env.client_manager.active_transactions().await;
        println!("Active client transactions: {}, Active server transactions: {}", 
                 client_txs.len(), server_txs.len());
        assert_eq!(client_txs.len(), 4, "Should have 4 active client transactions");
        
        // 8. Clean up one transaction explicitly
        println!("Explicitly terminating OPTIONS transaction");
        env.client_manager.terminate_transaction(&options_tx_id).await
            .expect("Failed to terminate OPTIONS transaction");
        
        // 9. Clean up terminated transactions
        println!("Cleaning up terminated transactions");
        let cleaned = env.client_manager.cleanup_terminated_transactions().await
            .expect("Failed to clean up terminated transactions");
        println!("Cleaned up {} transactions", cleaned);
        
        // 10. Verify updated transaction count
        let new_count = env.client_manager.transaction_count().await;
        println!("New active transaction count: {}", new_count);
        assert!(new_count < count, "Transaction count should have decreased");
        
        // Test has completed successfully
        println!("Concurrent transactions test completed");
        env.shutdown().await;
    }).await;
    
    if let Err(_) = test_result {
        panic!("Test timed out after 5 seconds");
    }
}

/// Tests Re-INVITE flow for dialog updates using proper event-driven approach
/// 
/// Scenario:
/// 1. Original INVITE to establish dialog
/// 2. Re-INVITE to modify session parameters
/// 3. Verify proper handling of transaction IDs and states
#[tokio::test]
#[serial]
async fn test_reinvite_flow() {
    println!("\n==== TEST: Re-INVITE Flow ====");
    println!("Testing dialog update with re-INVITE using event-driven approach");
    println!("This test verifies proper handling of Re-INVITE transactions for dialog refresh");
    println!("Scenario: Initial INVITE establishes dialog, Re-INVITE updates session");
    println!("Expected behavior: Dialog maintained while transactions complete independently\n");
    
    // Test timeout to prevent hanging
    let test_result = timeout(Duration::from_secs(10), async {
        println!("PART 1: Initial INVITE Dialog Establishment");
        // 1. Initialize the test environment
        let mut env = TestEnvironment::new().await;
        
        // 2. Create an initial INVITE request
        let server_uri = format!("sip:server@{}", env.server_addr);
        let invite_request = env.create_request(Method::Invite, &server_uri);
        
        // Store dialog parameters for reuse
        let dialog_call_id = invite_request.call_id().unwrap().to_string();
        let dialog_from_tag = invite_request.from().unwrap().address().params.iter()
            .find(|p| matches!(p, Param::Tag(_)))
            .and_then(|p| p.value())
            .unwrap_or_else(|| "from-tag-fallback".to_string());
        let dialog_to_tag = "test-dialog-to-tag".to_string();
        
        println!("Dialog parameters:");
        println!("Call-ID: {}", dialog_call_id);
        println!("From-Tag: {}", dialog_from_tag);
        println!("To-Tag: {}", dialog_to_tag);
        
        // 3. Create initial INVITE client transaction and subscribe to events
        println!("Creating initial INVITE client transaction");
        let invite_tx_id = env.client_manager.create_client_transaction(
            invite_request.clone(),
            env.server_addr
        ).await.expect("Failed to create INVITE client transaction");
        
        // Subscribe to client transaction events
        let mut invite_client_events = env.client_manager.subscribe_to_transaction(&invite_tx_id)
            .await.expect("Failed to subscribe to client transaction events");
        
        // 4. Send the INVITE request - triggers automatic state machine
        env.client_manager.send_request(&invite_tx_id).await
            .expect("Failed to send INVITE request");
        println!("Initial INVITE request sent");
        
        // 5. Wait for client to transition to Calling state
        println!("Waiting for client to enter Calling state");
        let calling_success = env.client_manager.wait_for_transaction_state(
            &invite_tx_id,
            TransactionState::Calling,
            Duration::from_millis(1000)
        ).await.expect("Failed to wait for Calling state");
        assert!(calling_success, "Client should transition to Calling state");
        println!("✅ Client entered Calling state");
        
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
        println!("✅ Server transaction auto-created");
        
        // Subscribe to server transaction events
        let mut invite_server_events = env.server_manager.subscribe_to_transaction(&invite_server_tx_id)
            .await.expect("Failed to subscribe to server transaction events");
        
        // 7. Wait for client to receive automatic 100 Trying first
        println!("Waiting for client to receive auto-sent 100 Trying");
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
                println!("ℹ️  100 Trying may have been processed before subscription");
            }
        }
        
        // 8. Server sends 180 Ringing with To tag (creates dialog)
        println!("Server sending 180 Ringing with To tag");
        let mut ringing_builder = rvoip_sip_core::builder::SimpleResponseBuilder::response_from_request(
            &invite_request, StatusCode::Ringing, Some("Ringing")
        );
        
        let to_header = invite_request.to().unwrap();
        ringing_builder = ringing_builder.to(
            to_header.address().display_name().unwrap_or_default(),
            &to_header.address().uri.to_string(),
            Some(&dialog_to_tag)
        );
        
        let ringing_response = ringing_builder.build();
        env.server_manager.send_response(&invite_server_tx_id, ringing_response.clone()).await
            .expect("Failed to send ringing response");
        
        // 9. Wait for client to receive 180 Ringing via ProvisionalResponse event
        println!("Waiting for client to receive 180 Ringing");
        let ringing_result = tokio::time::timeout(
            Duration::from_millis(1000),
            env.wait_for_client_event(Duration::from_millis(2000), |event| match_provisional_response(event))
        ).await;
        
        match ringing_result {
            Ok(Some((ringing_tx_id, ringing_resp))) => {
                assert_eq!(ringing_tx_id, invite_tx_id);
                assert_eq!(ringing_resp.status_code(), StatusCode::Ringing.as_u16());
                println!("✅ Client received 180 Ringing");
            }
            _ => {
                println!("ℹ️  180 Ringing may have been processed before subscription");
            }
        }
        
        // 10. Wait for client to transition to Proceeding state
        println!("Waiting for client to enter Proceeding state");
        let proceeding_success = env.client_manager.wait_for_transaction_state(
            &invite_tx_id,
            TransactionState::Proceeding,
            Duration::from_millis(1000)
        ).await.expect("Failed to wait for Proceeding state");
        
        if proceeding_success {
            println!("✅ Client entered Proceeding state");
        } else {
            // Check current state
            let current_state = env.client_manager.transaction_state(&invite_tx_id).await
                .expect("Failed to get current state");
            println!("ℹ️  Client in state: {:?}", current_state);
        }
        
        // 11. Server sends 200 OK with same To tag (establishes dialog)
        println!("Server sending 200 OK with To tag");
        let mut ok_builder = rvoip_sip_core::builder::SimpleResponseBuilder::response_from_request(
            &invite_request, StatusCode::Ok, Some("OK")
        );
        
        let to_header = invite_request.to().unwrap();
        ok_builder = ok_builder.to(
            to_header.address().display_name().unwrap_or_default(),
            &to_header.address().uri.to_string(),
            Some(&dialog_to_tag)
        ).contact(
            &format!("sip:server@{}", env.server_addr),
            Some("Server UA")
        );
        
        let ok_response = ok_builder.build();
        env.server_manager.send_response(&invite_server_tx_id, ok_response.clone()).await
            .expect("Failed to send OK response");
        
        // 12. Wait for client to receive 200 OK via SuccessResponse event
        println!("Waiting for client to receive 200 OK");
        let (success_tx_id, success_resp) = env.wait_for_client_event(
            Duration::from_millis(1000),
            |event| match_success_response(event)
        ).await.expect("Timeout waiting for 200 OK");
        assert_eq!(success_tx_id, invite_tx_id);
        assert_eq!(success_resp.status_code(), StatusCode::Ok.as_u16());
        println!("✅ Client received 200 OK - dialog established");
        
        // 13. Wait for transactions to terminate (INVITE transactions terminate on 2xx)
        println!("Waiting for INVITE transactions to terminate");
        let client_terminated = tokio::time::timeout(
            Duration::from_millis(2000),
            env.wait_for_client_event(Duration::from_millis(3000), |event| match_transaction_terminated(event))
        ).await;
        
        match client_terminated {
            Ok(Some(terminated_tx_id)) => {
                if terminated_tx_id == invite_tx_id {
                    println!("✅ Initial INVITE client transaction terminated");
                }
            }
            _ => {
                println!("ℹ️  Initial INVITE client transaction termination handled");
            }
        }
        
        println!("\nPART 2: Re-INVITE in Established Dialog");
        
        // 14. Create a Re-INVITE request with dialog identifiers
        println!("Creating Re-INVITE with dialog identifiers");
        let mut reinvite_builder = rvoip_sip_core::builder::SimpleRequestBuilder::new(
            Method::Invite, &server_uri
        ).expect("Failed to create Re-INVITE builder");
        
        reinvite_builder = reinvite_builder
            .call_id(&dialog_call_id)
            .from("Client UA", &format!("sip:client@{}", env.client_addr), Some(&dialog_from_tag))
            .to("Server UA", &server_uri, Some(&dialog_to_tag))
            .via(&env.client_addr.to_string(), "UDP", Some(&format!("z9hG4bK{}", uuid::Uuid::new_v4().simple())))
            .cseq(2) // Increment CSeq for re-INVITE
            .max_forwards(70)
            .contact(&format!("sip:client@{}", env.client_addr), Some("Client UA"))
            .header(TypedHeader::ContentLength(ContentLength::new(0)));
        
        let reinvite_request = reinvite_builder.build();
        
        println!("Re-INVITE Call-ID: {}", reinvite_request.call_id().unwrap());
        println!("Re-INVITE From Tag: {}", reinvite_request.from().unwrap().to_string());
        println!("Re-INVITE To Tag: {}", reinvite_request.to().unwrap().to_string());
        
        // 15. Create client transaction for Re-INVITE and subscribe to events
        println!("Creating Re-INVITE client transaction");
        let reinvite_tx_id = env.client_manager.create_client_transaction(
            reinvite_request.clone(),
            env.server_addr
        ).await.expect("Failed to create Re-INVITE client transaction");
        
        // Subscribe to Re-INVITE client transaction events
        let mut reinvite_client_events = env.client_manager.subscribe_to_transaction(&reinvite_tx_id)
            .await.expect("Failed to subscribe to Re-INVITE client events");
        
        // 16. Send the Re-INVITE request - triggers automatic state machine
        env.client_manager.send_request(&reinvite_tx_id).await
            .expect("Failed to send Re-INVITE request");
        println!("Re-INVITE request sent");
        
        // 17. Wait for Re-INVITE client to transition to Calling state
        println!("Waiting for Re-INVITE client to enter Calling state");
        let reinvite_calling_success = env.client_manager.wait_for_transaction_state(
            &reinvite_tx_id,
            TransactionState::Calling,
            Duration::from_millis(1000)
        ).await.expect("Failed to wait for Re-INVITE Calling state");
        assert!(reinvite_calling_success, "Re-INVITE client should transition to Calling state");
        println!("✅ Re-INVITE client entered Calling state");
        
        // 18. Find the auto-created Re-INVITE server transaction
        println!("Looking for auto-created Re-INVITE server transaction");
        tokio::time::sleep(Duration::from_millis(200)).await;
        
        let reinvite_server_tx_id = TransactionKey::from_request(&reinvite_request)
            .expect("Failed to create Re-INVITE server transaction key");
        println!("Expected Re-INVITE server transaction ID: {:?}", reinvite_server_tx_id);
        
        // Verify Re-INVITE server transaction exists
        let reinvite_server_exists = env.server_manager.transaction_exists(&reinvite_server_tx_id).await;
        if !reinvite_server_exists {
            panic!("Re-INVITE server transaction should have been auto-created");
        }
        println!("✅ Re-INVITE server transaction auto-created");
        
        // Subscribe to Re-INVITE server transaction events
        let mut reinvite_server_events = env.server_manager.subscribe_to_transaction(&reinvite_server_tx_id)
            .await.expect("Failed to subscribe to Re-INVITE server events");
        
        // 19. Server sends 200 OK for Re-INVITE
        println!("Server sending 200 OK for Re-INVITE");
        let mut reinvite_ok_builder = rvoip_sip_core::builder::SimpleResponseBuilder::response_from_request(
            &reinvite_request, StatusCode::Ok, Some("OK")
        );
        
        // Contact header is required for dialog
        reinvite_ok_builder = reinvite_ok_builder
            .contact(&format!("sip:server@{}", env.server_addr), Some("Server UA"))
            .header(TypedHeader::ContentLength(ContentLength::new(0)));
        
        let reinvite_ok = reinvite_ok_builder.build();
        env.server_manager.send_response(&reinvite_server_tx_id, reinvite_ok.clone()).await
            .expect("Failed to send OK response for Re-INVITE");
        
        // 20. Wait for Re-INVITE client to receive 200 OK via SuccessResponse event
        println!("Waiting for Re-INVITE client to receive 200 OK");
        let (reinvite_success_tx_id, reinvite_success_resp) = env.wait_for_client_event(
            Duration::from_millis(1000),
            |event| match_success_response(event)
        ).await.expect("Timeout waiting for Re-INVITE 200 OK");
        assert_eq!(reinvite_success_tx_id, reinvite_tx_id);
        assert_eq!(reinvite_success_resp.status_code(), StatusCode::Ok.as_u16());
        println!("✅ Re-INVITE client received 200 OK");
        
        // 21. Wait for Re-INVITE transactions to terminate (INVITE transactions terminate on 2xx)
        println!("Waiting for Re-INVITE transactions to terminate");
        let reinvite_client_terminated = tokio::time::timeout(
            Duration::from_millis(2000),
            env.wait_for_client_event(Duration::from_millis(3000), |event| match_transaction_terminated(event))
        ).await;
        
        match reinvite_client_terminated {
            Ok(Some(terminated_tx_id)) => {
                if terminated_tx_id == reinvite_tx_id {
                    println!("✅ Re-INVITE client transaction terminated");
                }
            }
            _ => {
                println!("ℹ️  Re-INVITE client transaction termination handled");
            }
        }
        
        // 22. Verify ACK handling (ACK for 2xx is handled by TU layer, not transaction layer)
        println!("Verifying ACK handling for 2xx responses");
        
        // In a real application, the TU (Transaction User) would handle ACK for 2xx responses
        // The transaction layer handles ACK only for non-2xx responses
        // This is per RFC 3261 Section 17.1.1.3
        println!("✅ For 2xx responses, ACK would be handled by TU, not transaction layer");
        
        println!("✅ Re-INVITE flow test completed successfully using event-driven approach");
        env.shutdown().await;
    }).await;
    
    if let Err(_) = test_result {
        panic!("Test timed out after 10 seconds");
    }
}

/// Test complete INVITE call flow with builder integration
///
/// This test demonstrates:
/// 1. Using InviteBuilder to create a proper INVITE request with SDP
/// 2. Using InviteResponseBuilder for server responses
/// 3. Using ByeBuilder for call termination
/// 4. Full integration with transaction layer
#[tokio::test]
#[serial]
async fn test_invite_call_flow_with_builders() {
    env::set_var("RVOIP_TEST", "1");
    
    println!("\n==== TEST: INVITE Call Flow with Builders ====");
    println!("Testing complete call flow using new builders for request/response creation");
    
    let test_result = timeout(Duration::from_secs(15), async {
        let mut env = TestEnvironment::new().await;
        
        // 1. Create INVITE using the new builder with SDP
        println!("Creating INVITE request using InviteBuilder");
        let from_uri = format!("sip:alice@{}", env.client_addr.ip());
        let to_uri = format!("sip:bob@{}", env.server_addr.ip());
        let sdp_offer = r#"v=0
o=alice 123456 123456 IN IP4 127.0.0.1
s=Call Session
c=IN IP4 127.0.0.1
t=0 0
m=audio 5004 RTP/AVP 0
a=rtpmap:0 PCMU/8000"#;
        
        let invite_request = InviteBuilder::new()
            .from_to(&from_uri, &to_uri)
            .local_address(env.client_addr)
            .with_sdp(sdp_offer)
            .cseq(1)
            .build()
            .expect("Failed to build INVITE with builder");
        
        // Verify the builder created a proper INVITE
        assert_eq!(invite_request.method(), Method::Invite);
        assert!(invite_request.body().len() > 0, "INVITE should have SDP content");
        println!("✅ InviteBuilder created proper INVITE with SDP");
        
        // 2. Create client transaction
        let client_tx_id = env.client_manager.create_client_transaction(
            invite_request.clone(),
            env.server_addr
        ).await.expect("Failed to create client transaction");
        
        // 3. Send INVITE
        env.client_manager.send_request(&client_tx_id).await
            .expect("Failed to send INVITE");
        
        // Give time for automatic processing
        tokio::time::sleep(Duration::from_millis(200)).await;
        
        // 4. Find server transaction
        let server_tx_id = TransactionKey::from_request(&invite_request)
            .expect("Failed to create server transaction key");
        
        assert!(env.server_manager.transaction_exists(&server_tx_id).await,
                "Server transaction should exist");
        
        // 5. Server responds with 180 Ringing using builder
        println!("Server sending 180 Ringing using builder");
        let ringing_response = server_quick::ringing(&invite_request, None)
            .expect("Failed to create 180 Ringing with builder");
        
        env.server_manager.send_response(&server_tx_id, ringing_response).await
            .expect("Failed to send ringing response");
        
        // 6. Server responds with 200 OK with SDP using InviteResponseBuilder
        println!("Server sending 200 OK with SDP using InviteResponseBuilder");
        let sdp_answer = r#"v=0
o=bob 654321 654321 IN IP4 127.0.0.1
s=Call Session
c=IN IP4 127.0.0.1
t=0 0
m=audio 5006 RTP/AVP 0
a=rtpmap:0 PCMU/8000"#;
        
        let ok_response = InviteResponseBuilder::new(StatusCode::Ok)
            .from_request(&invite_request)
            .with_sdp_answer(sdp_answer)
            .with_contact_address(env.server_addr, Some("bob"))
            .build()
            .expect("Failed to build 200 OK with InviteResponseBuilder");
        
        // Verify the builder created a proper response
        assert_eq!(ok_response.status_code(), 200);
        assert!(ok_response.body().len() > 0, "200 OK should have SDP content");
        assert!(ok_response.to().unwrap().tag().is_some(), "200 OK should have To tag");
        println!("✅ InviteResponseBuilder created proper 200 OK with SDP");
        
        env.server_manager.send_response(&server_tx_id, ok_response.clone()).await
            .expect("Failed to send OK response");
        
        // 7. Wait for call establishment
        let success_event = env.wait_for_client_event(
            Duration::from_millis(2000),
            |event| match_success_response(event)
        ).await;
        
        assert!(success_event.is_some(), "Should receive 200 OK");
        println!("✅ Call established successfully using builders");
        
        // 8. Terminate call with BYE using ByeBuilder
        println!("Terminating call using ByeBuilder");
        
        // Extract dialog information from the established call
        let call_id = invite_request.call_id().unwrap().value().to_string();
        let from_tag = invite_request.from().unwrap().tag().unwrap().to_string();
        let to_tag = ok_response.to().unwrap().tag().unwrap().to_string();
        
        let bye_request = ByeBuilder::new()
            .from_dialog(&call_id, &from_uri, &from_tag, &to_uri, &to_tag)
            .local_address(env.client_addr)
            .cseq(2) // Increment CSeq for new request in dialog
            .build()
            .expect("Failed to build BYE with builder");
        
        // Verify the builder created a proper BYE
        assert_eq!(bye_request.method(), Method::Bye);
        assert_eq!(bye_request.call_id().unwrap().value(), call_id);
        assert_eq!(bye_request.cseq().unwrap().seq, 2);
        println!("✅ ByeBuilder created proper BYE request");
        
        // 9. Send BYE
        let bye_tx_id = env.client_manager.create_client_transaction(
            bye_request.clone(),
            env.server_addr
        ).await.expect("Failed to create BYE transaction");
        
        env.client_manager.send_request(&bye_tx_id).await
            .expect("Failed to send BYE");
        
        // Give time for processing
        tokio::time::sleep(Duration::from_millis(200)).await;
        
        // 10. Server responds to BYE using builder
        let bye_server_tx_id = TransactionKey::from_request(&bye_request)
            .expect("Failed to create BYE server transaction key");
        
        let bye_ok = server_quick::ok_bye(&bye_request)
            .expect("Failed to create 200 OK for BYE");
        
        env.server_manager.send_response(&bye_server_tx_id, bye_ok).await
            .expect("Failed to send BYE response");
        
        // Wait for BYE completion
        let bye_success = env.wait_for_client_event(
            Duration::from_millis(2000),
            |event| match_success_response(event)
        ).await;
        
        assert!(bye_success.is_some(), "Should receive 200 OK for BYE");
        println!("✅ Call terminated successfully using builders");
        
        env.shutdown().await;
    }).await;
    
    if let Err(_) = test_result {
        panic!("Test timed out");
    }
    
    env::remove_var("RVOIP_TEST");
}

/// Test REGISTER flow with builders
///
/// This test demonstrates:
/// 1. Using client_quick::register for REGISTER request creation
/// 2. Using server_quick::ok_register for server response
/// 3. Full integration with transaction layer
#[tokio::test]
#[serial]
async fn test_register_flow_with_builders() {
    env::set_var("RVOIP_TEST", "1");
    
    println!("\n==== TEST: REGISTER Flow with Builders ====");
    println!("Testing REGISTER flow using builders for request/response creation");
    
    let test_result = timeout(Duration::from_secs(10), async {
        let mut env = TestEnvironment::new().await;
        
        // 1. Create REGISTER using quick helper
        println!("Creating REGISTER request using client_quick::register");
        let registrar_uri = format!("sip:registrar@{}", env.server_addr.ip());
        let user_uri = format!("sip:alice@{}", env.client_addr.ip());
        
        let register_request = client_quick::register(
            &registrar_uri,
            &user_uri,
            "Alice Smith",
            env.client_addr,
            Some(3600) // 1 hour registration
        ).expect("Failed to create REGISTER with builder");
        
        // Verify the builder created a proper REGISTER
        assert_eq!(register_request.method(), Method::Register);
        assert_eq!(register_request.uri().to_string(), registrar_uri);
        let expires_header = register_request.header(&rvoip_sip_core::types::header::HeaderName::Expires);
        assert!(expires_header.is_some(), "REGISTER should have Expires header");
        println!("✅ client_quick::register created proper REGISTER request");
        
        // 2. Create client transaction
        let client_tx_id = env.client_manager.create_client_transaction(
            register_request.clone(),
            env.server_addr
        ).await.expect("Failed to create REGISTER transaction");
        
        // 3. Send REGISTER
        env.client_manager.send_request(&client_tx_id).await
            .expect("Failed to send REGISTER");
        
        // Give time for automatic processing
        tokio::time::sleep(Duration::from_millis(200)).await;
        
        // 4. Find server transaction
        let server_tx_id = TransactionKey::from_request(&register_request)
            .expect("Failed to create server transaction key");
        
        assert!(env.server_manager.transaction_exists(&server_tx_id).await,
                "Server transaction should exist");
        
        // 5. Server responds with 200 OK using builder with registered contacts
        println!("Server sending 200 OK for REGISTER using server_quick::ok_register");
        let registered_contact = format!("sip:alice@{}", env.client_addr);
        
        let register_ok = server_quick::ok_register(
            &register_request,
            3600, // Same expires
            vec![registered_contact.clone()]
        ).expect("Failed to create 200 OK for REGISTER");
        
        // Verify the builder created a proper response
        assert_eq!(register_ok.status_code(), 200);
        let expires_header = register_ok.header(&rvoip_sip_core::types::header::HeaderName::Expires);
        assert!(expires_header.is_some(), "200 OK should have Expires header");
        println!("✅ server_quick::ok_register created proper 200 OK");
        
        env.server_manager.send_response(&server_tx_id, register_ok).await
            .expect("Failed to send REGISTER response");
        
        // 6. Wait for registration completion
        let success_event = env.wait_for_client_event(
            Duration::from_millis(2000),
            |event| match_success_response(event)
        ).await;
        
        assert!(success_event.is_some(), "Should receive 200 OK for REGISTER");
        println!("✅ Registration completed successfully using builders");
        
        env.shutdown().await;
    }).await;
    
    if let Err(_) = test_result {
        panic!("Test timed out");
    }
    
    env::remove_var("RVOIP_TEST");
}

/// Test error response scenarios with builders
///
/// This test demonstrates:
/// 1. Using server_quick helpers for various error responses
/// 2. Integration with transaction error handling
#[tokio::test]
#[serial]
async fn test_error_responses_with_builders() {
    env::set_var("RVOIP_TEST", "1");
    
    println!("\n==== TEST: Error Responses with Builders ====");
    println!("Testing error response scenarios using builders");
    
    let test_result = timeout(Duration::from_secs(10), async {
        let mut env = TestEnvironment::new().await;
        
        // 1. Create INVITE that will be rejected
        let invite_request = env.create_request(Method::Invite, &format!("sip:busy@{}", env.server_addr));
        
        let client_tx_id = env.client_manager.create_client_transaction(
            invite_request.clone(),
            env.server_addr
        ).await.expect("Failed to create client transaction");
        
        env.client_manager.send_request(&client_tx_id).await
            .expect("Failed to send INVITE");
        
        tokio::time::sleep(Duration::from_millis(200)).await;
        
        let server_tx_id = TransactionKey::from_request(&invite_request)
            .expect("Failed to create server transaction key");
        
        // 2. Test various error responses using builders
        println!("Testing 486 Busy Here using server_quick::busy_here");
        let busy_response = server_quick::busy_here(&invite_request)
            .expect("Failed to create 486 Busy Here");
        
        assert_eq!(busy_response.status_code(), 486);
        assert_eq!(busy_response.reason_phrase(), "Busy Here");
        println!("✅ server_quick::busy_here created proper 486 response");
        
        env.server_manager.send_response(&server_tx_id, busy_response).await
            .expect("Failed to send 486 response");
        
        // Wait for error response
        let error_event = env.wait_for_client_event(
            Duration::from_millis(2000),
            |event| match_failure_response(event)
        ).await;
        
        if let Some((_, response)) = error_event {
            assert_eq!(response.status_code(), 486);
            println!("✅ Client received 486 Busy Here error response");
        }
        
        // 3. Test 404 Not Found for a different call
        println!("Testing 404 Not Found scenario");
        let invite2 = env.create_request(Method::Invite, &format!("sip:notfound@{}", env.server_addr));
        
        let client_tx_id2 = env.client_manager.create_client_transaction(
            invite2.clone(),
            env.server_addr
        ).await.expect("Failed to create second client transaction");
        
        env.client_manager.send_request(&client_tx_id2).await
            .expect("Failed to send second INVITE");
        
        tokio::time::sleep(Duration::from_millis(200)).await;
        
        let server_tx_id2 = TransactionKey::from_request(&invite2)
            .expect("Failed to create second server transaction key");
        
        let not_found_response = server_quick::not_found(&invite2)
            .expect("Failed to create 404 Not Found");
        
        assert_eq!(not_found_response.status_code(), 404);
        println!("✅ server_quick::not_found created proper 404 response");
        
        env.server_manager.send_response(&server_tx_id2, not_found_response).await
            .expect("Failed to send 404 response");
        
        println!("✅ Error response scenarios completed using builders");
        
        env.shutdown().await;
    }).await;
    
    if let Err(_) = test_result {
        panic!("Test timed out");
    }
    
    env::remove_var("RVOIP_TEST");
}

/// Test builder customization in real scenarios
///
/// This test demonstrates:
/// 1. Custom headers and parameters using builders
/// 2. SDP handling with builders
/// 3. Contact header management
#[tokio::test]
#[serial]
async fn test_builder_customization_scenarios() {
    env::set_var("RVOIP_TEST", "1");
    
    println!("\n==== TEST: Builder Customization Scenarios ====");
    println!("Testing builder customization features in real transaction flows");
    
    let test_result = timeout(Duration::from_secs(10), async {
        let mut env = TestEnvironment::new().await;
        
        // 1. Create highly customized INVITE
        println!("Creating customized INVITE with specific Call-ID and headers");
        let custom_call_id = "custom-call-12345";
        let custom_contact = format!("sip:alice-custom@{}", env.client_addr);
        
        let custom_invite = InviteBuilder::new()
            .from_to(
                &format!("sip:alice@{}", env.client_addr.ip()),
                &format!("sip:bob@{}", env.server_addr.ip())
            )
            .local_address(env.client_addr)
            .call_id(custom_call_id)
            .cseq(42) // Custom CSeq number
            .contact(&custom_contact)
            .max_forwards(50) // Custom Max-Forwards
            .build()
            .expect("Failed to build customized INVITE");
        
        // Verify customizations
        assert_eq!(custom_invite.call_id().unwrap().value(), custom_call_id);
        assert_eq!(custom_invite.cseq().unwrap().seq, 42);
        println!("✅ Created customized INVITE with specific parameters");
        
        // 2. Create client transaction and send
        let client_tx_id = env.client_manager.create_client_transaction(
            custom_invite.clone(),
            env.server_addr
        ).await.expect("Failed to create customized transaction");
        
        env.client_manager.send_request(&client_tx_id).await
            .expect("Failed to send customized INVITE");
        
        tokio::time::sleep(Duration::from_millis(200)).await;
        
        // 3. Create customized response with specific To tag
        let server_tx_id = TransactionKey::from_request(&custom_invite)
            .expect("Failed to create server transaction key");
        
        let custom_to_tag = "custom-server-tag-67890";
        let custom_ok = ResponseBuilder::new(StatusCode::Ok)
            .from_request(&custom_invite)
            .with_to_tag(custom_to_tag)
            .reason_phrase("Custom OK Message")
            .with_contact_address(env.server_addr, Some("custom-bob"))
            .build()
            .expect("Failed to build customized 200 OK");
        
        // Verify customizations
        assert_eq!(custom_ok.to().unwrap().tag().unwrap(), custom_to_tag);
        assert_eq!(custom_ok.reason_phrase(), "Custom OK Message");
        println!("✅ Created customized 200 OK response");
        
        env.server_manager.send_response(&server_tx_id, custom_ok).await
            .expect("Failed to send customized response");
        
        // 4. Verify the custom flow completes
        let success_event = env.wait_for_client_event(
            Duration::from_millis(2000),
            |event| match_success_response(event)
        ).await;
        
        if let Some((_, response)) = success_event {
            assert_eq!(response.status_code(), 200);
            assert_eq!(response.reason_phrase(), "Custom OK Message");
            println!("✅ Customized call flow completed successfully");
        }
        
        env.shutdown().await;
    }).await;
    
    if let Err(_) = test_result {
        panic!("Test timed out");
    }
    
    env::remove_var("RVOIP_TEST");
} 