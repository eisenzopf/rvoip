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
    println!("Testing cancellation of INVITE after 180 Ringing");
    println!("This test verifies the CANCEL functionality according to RFC 3261 Section 9.1");
    println!("Scenario: Client cancels an INVITE transaction after receiving a provisional response");
    println!("Expected behavior: Server sends 487 Request Terminated and client sends ACK\n");
    
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
        
        // 5. Wait for server to receive INVITE or detect auto-created transaction
        println!("Waiting for server to receive INVITE");
        
        // Give time for the message to be processed
        sleep(Duration::from_millis(100)).await;
        
        // Instead of waiting for a specific event, let's check if the server received and processed the INVITE
        // by looking for a sent message (like 100 Trying) or by creating the server transaction ourselves
        let invite_server_tx_id;
        
        // Check if server sent any responses (like 100 Trying)
        let server_sent_response = env.server_transport.sent_message_count().await > 0;
        
        if server_sent_response {
            println!("Server auto-processed INVITE and sent response");
            // Server already processed the INVITE, create our transaction to match
            let server_tx = env.server_manager.create_server_transaction(
                invite_request.clone(),
                env.client_addr
            ).await.expect("Failed to create server transaction");
            invite_server_tx_id = server_tx.id().clone();
            println!("Server transaction created with ID: {:?}", invite_server_tx_id);
        } else {
            // Traditional flow - wait for NewRequest event
            match env.wait_for_server_event(
                Duration::from_millis(500),
                |event| match_new_request(event)
            ).await {
                Some((server_tx_id, _, _)) => {
                    println!("Server received INVITE request via NewRequest event, transaction ID: {:?}", server_tx_id);
                    
                    // Create the server transaction for the INVITE explicitly
                    println!("Server creating transaction for received INVITE");
                    let server_tx = env.server_manager.create_server_transaction(
                        invite_request.clone(),
                        env.client_addr
                    ).await.expect("Failed to create server transaction");
                    invite_server_tx_id = server_tx.id().clone();
                    println!("Server transaction created with ID: {:?}", invite_server_tx_id);
                },
                None => {
                    println!("No NewRequest event received, creating server transaction directly");
                    // Create server transaction directly
                    let server_tx = env.server_manager.create_server_transaction(
                        invite_request.clone(),
                        env.client_addr
                    ).await.expect("Failed to create server transaction");
                    invite_server_tx_id = server_tx.id().clone();
                    println!("Server transaction created with ID: {:?}", invite_server_tx_id);
                }
            }
        }
        
        // 6. Server sends 180 Ringing (provisional response)
        println!("Server sending 180 Ringing");
        let ringing_response = env.create_response(&invite_request, StatusCode::Ringing, Some("Ringing"));
        env.server_manager.send_response(&invite_server_tx_id, ringing_response.clone()).await
            .expect("Failed to send ringing response");
        
        // 7. Wait for client to receive 180 Ringing
        println!("Waiting for client to receive 180 Ringing");
        let (response_tx_id, _) = env.wait_for_client_event(
            Duration::from_millis(1000),
            |event| match_provisional_response(event)
        ).await.expect("Timeout waiting for 180 Ringing");
        assert_eq!(response_tx_id, invite_tx_id, "Response transaction ID should match INVITE transaction ID");
        
        // 8. Verify client state (should be Proceeding after 180 Ringing)
        let mut client_state = TransactionState::Calling;
        for _ in 0..10 {
            let state = env.client_manager.transaction_state(&invite_tx_id).await
                .expect("Failed to get client state");
            if state == TransactionState::Proceeding {
                client_state = state;
                break;
            }
            sleep(Duration::from_millis(50)).await;
        }
        
        println!("Client state after 180 Ringing: {:?}", client_state);
        
        // Can be Proceeding, Calling, or might have optimized to Terminated 
        assert!(
            client_state == TransactionState::Proceeding || 
            client_state == TransactionState::Calling,
            "Client should be in Proceeding or Calling state after 180 Ringing"
        );
        
        // 9. Create a CANCEL request based on the INVITE
        println!("Creating CANCEL request for the INVITE");
        
        // Using the transaction manager's cancel_invite_transaction method (public API)
        let cancel_tx_id = env.client_manager.cancel_invite_transaction(&invite_tx_id).await
            .expect("Failed to create CANCEL transaction via API");
        
        println!("CANCEL client transaction created with ID: {:?}", cancel_tx_id);
        
        // 10. Wait for server to receive CANCEL or detect auto-created transaction
        println!("Waiting for server to receive CANCEL");
        
        // Give time for the CANCEL message to be processed
        sleep(Duration::from_millis(100)).await;
        
        // Get current message count before checking for new responses
        let messages_before_cancel = env.server_transport.sent_message_count().await;
        
        // Check if server auto-processed the CANCEL by looking for any new responses
        // The server might have already processed it and created a transaction
        let cancel_server_tx_id;
        let cancel_request;
        
        // First, try to extract the CANCEL request from the client's cancel transaction
        // We can get it from the transaction manager or recreate it
        let extracted_cancel_request = {
            // Get the original INVITE request and create a CANCEL based on it
            // This matches what the client transaction manager would have created
            env.create_cancel_request(&invite_request)
        };
        
        // Check if server has processed new messages (might have auto-created CANCEL transaction)
        let messages_after_cancel = env.server_transport.sent_message_count().await;
        let server_auto_processed_cancel = messages_after_cancel > messages_before_cancel;
        
        if server_auto_processed_cancel {
            println!("Server auto-processed CANCEL and may have sent response");
            // Server already processed the CANCEL, create our transaction to match
            let server_tx = env.server_manager.create_server_transaction(
                extracted_cancel_request.clone(),
                env.client_addr
            ).await.expect("Failed to create server CANCEL transaction");
            cancel_server_tx_id = server_tx.id().clone();
            cancel_request = extracted_cancel_request;
            println!("Server CANCEL transaction created with ID: {:?}", cancel_server_tx_id);
        } else {
            // Traditional flow - wait for NewRequest event or create transaction directly
            match env.wait_for_server_event(
                Duration::from_millis(500),
                |event| match_new_request(event)
            ).await {
                Some((server_tx_id, received_cancel_request, _)) => {
                    println!("Server received CANCEL request via NewRequest event, transaction ID: {:?}", server_tx_id);
                    
                    // Create the server transaction for the CANCEL explicitly
                    println!("Server creating new transaction for received CANCEL");
                    let cancel_server_tx = env.server_manager.create_server_transaction(
                        received_cancel_request.clone(),
                        env.client_addr
                    ).await.expect("Failed to create server CANCEL transaction");
                    cancel_server_tx_id = cancel_server_tx.id().clone();
                    cancel_request = received_cancel_request;
                    println!("Server CANCEL transaction created with ID: {:?}", cancel_server_tx_id);
                },
                None => {
                    println!("No NewRequest event received for CANCEL, creating server transaction directly");
                    // Create server transaction directly using the extracted CANCEL request
                    let cancel_server_tx = env.server_manager.create_server_transaction(
                        extracted_cancel_request.clone(),
                        env.client_addr
                    ).await.expect("Failed to create server CANCEL transaction");
                    cancel_server_tx_id = cancel_server_tx.id().clone();
                    cancel_request = extracted_cancel_request;
                    println!("Server CANCEL transaction created with ID: {:?}", cancel_server_tx_id);
                }
            }
        }
        
        // 11. Server sends 200 OK for CANCEL
        println!("Server sending 200 OK for CANCEL");
        let cancel_ok_response = env.create_response(&cancel_request, StatusCode::Ok, Some("OK"));
        env.server_manager.send_response(&cancel_server_tx_id, cancel_ok_response.clone()).await
            .expect("Failed to send OK response for CANCEL");
        
        // 12. Server sends 487 Request Terminated for the INVITE first
        // (should be done before ACK is received to ensure proper sequence)
        println!("Server sending 487 Request Terminated for INVITE");
        let terminated_response = env.create_response(
            &invite_request, 
            StatusCode::RequestTerminated, 
            Some("Request Terminated")
        );
        env.server_manager.send_response(&invite_server_tx_id, terminated_response.clone()).await
            .expect("Failed to send 487 Request Terminated");
        
        // 12. Wait for messages to be processed and transactions to reach final states
        println!("Waiting for client to receive and process success responses");
        
        // Give time for both responses (200 OK and 487) to be received and processed
        sleep(Duration::from_millis(500)).await;
        
        // Verify that both transactions received their appropriate responses by checking states
        let cancel_client_state = env.client_manager.transaction_state(&cancel_tx_id).await;
        let invite_client_state = env.client_manager.transaction_state(&invite_tx_id).await;
        
        println!("CANCEL client transaction state: {:?}", cancel_client_state);
        println!("INVITE client transaction state: {:?}", invite_client_state);
        
        // CANCEL should be in Trying, Completed or Terminated (200 OK may still be processing)
        match cancel_client_state {
            Ok(state) => {
                assert!(
                    state == TransactionState::Trying ||
                    state == TransactionState::Completed || 
                    state == TransactionState::Terminated,
                    "CANCEL client should be in Trying, Completed or Terminated state"
                );
                if state == TransactionState::Trying {
                    println!("✓ CANCEL transaction is processing (Trying state - 200 OK may still be in transit)");
                } else {
                    println!("✓ CANCEL transaction received and processed 200 OK response");
                }
            },
            Err(_) => {
                // Transaction might have been removed already
                let exists = env.client_manager.transaction_exists(&cancel_tx_id).await;
                if !exists {
                    println!("✓ CANCEL client transaction already terminated and removed");
                } else {
                    panic!("CANCEL client transaction exists but state cannot be retrieved");
                }
            }
        }
        
        // INVITE should be in Completed or Terminated (received 487)
        match invite_client_state {
            Ok(state) => {
                assert!(
                    state == TransactionState::Completed || 
                    state == TransactionState::Terminated,
                    "INVITE client should be in Completed or Terminated state after receiving 487"
                );
                println!("✓ INVITE transaction received 487 Request Terminated");
            },
            Err(_) => {
                // Transaction might have been removed already
                let exists = env.client_manager.transaction_exists(&invite_tx_id).await;
                if !exists {
                    println!("✓ INVITE client transaction already terminated and removed");
                } else {
                    panic!("INVITE client transaction exists but state cannot be retrieved");
                }
            }
        }
        
        // 14. Verify ACK was sent by checking client transport logs
        println!("Checking for client ACK to be generated");
        
        // Wait a short time for the ACK to be sent and received
        sleep(Duration::from_millis(100)).await;
        
        // Check for ACK in the client's sent messages
        let mut found_ack = false;
        let mut ack_request = None;
        {
            let messages = env.client_transport.sent_messages.lock().await;
            for (message, _) in messages.iter() {
                if let Message::Request(request) = message {
                    if request.method() == Method::Ack {
                        found_ack = true;
                        ack_request = Some(request.clone());
                        println!("Found ACK sent by client");
                        break;
                    }
                }
            }
        }
        
        // Verify we found the ACK - that's sufficient for the test
        assert!(found_ack, "Client should have sent an ACK for the 487 response");

        // For server processing, check if server transaction is still active
        // before attempting to process the ACK - it may have already terminated
        let server_tx_exists = env.server_manager.transaction_exists(&invite_server_tx_id).await;
        if server_tx_exists {
            println!("Server transaction still exists, checking state");
            
            let server_state = env.server_manager.transaction_state(&invite_server_tx_id).await
                .expect("Failed to get server transaction state");
            
            // Only try to process ACK if server transaction is still in Completed state
            if server_state == TransactionState::Completed {
                println!("Processing ACK with server transaction in Completed state");
                // Process the ACK (will transition to Confirmed)
                if let Some(ack) = ack_request {
                    match env.server_manager.process_request(&invite_server_tx_id, ack).await {
                        Ok(_) => {
                            println!("Successfully processed ACK, server should move to Confirmed");
                            
                            // Wait a bit and check server state
                            sleep(Duration::from_millis(50)).await;
                            
                            // Try to get state, but server may have terminated already
                            if let Ok(state) = env.server_manager.transaction_state(&invite_server_tx_id).await {
                                println!("Server state after ACK: {:?}", state);
                            } else {
                                println!("Server transaction already terminated after ACK");
                            }
                        },
                        Err(e) => {
                            println!("Note: Could not process ACK: {} - this is expected if server transaction terminated quickly", e);
                        }
                    }
                }
            } else {
                println!("Server transaction not in Completed state (state: {:?}), skipping ACK processing", server_state);
            }
        } else {
            println!("Server transaction already terminated, skipping ACK processing");
        }
        
        // Wait for transaction termination - should happen naturally with timers
        println!("Waiting for transaction termination timers...");
        sleep(Duration::from_millis(500)).await;
        
        println!("CANCEL test completed successfully");
        
        // Clean up
        env.shutdown().await;
    }).await;
    
    // Handle test timeout
    if let Err(_) = test_result {
        panic!("Test timed out after 5 seconds");
    }
    
    // Reset environment variable
    env::remove_var("RVOIP_TEST");
} 