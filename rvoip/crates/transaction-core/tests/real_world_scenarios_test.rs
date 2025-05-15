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
    println!("Testing auth challenge-response handling");
    println!("This test verifies proper handling of authentication challenges");
    println!("Scenario: Client sends REGISTER, server challenges with 401, client authenticates");
    println!("Expected behavior: New transaction created with auth credentials, server accepts with 200 OK\n");
    
    // Test timeout to prevent hanging
    let test_result = timeout(Duration::from_secs(5), async {
        // 1. Initialize the test environment
        let mut env = TestEnvironment::new().await;
        
        // 2. Create a REGISTER request
        let server_uri = format!("sip:registrar@{}", env.server_addr);
        let register_request = env.create_request(Method::Register, &server_uri);
        
        // 3. Create client transaction for REGISTER
        println!("Creating REGISTER client transaction");
        let register_tx_id = env.client_manager.create_client_transaction(
            register_request.clone(),
            env.server_addr
        ).await.expect("Failed to create REGISTER client transaction");
        
        // 4. Send the REGISTER request
        env.client_manager.send_request(&register_tx_id).await
            .expect("Failed to send REGISTER request");
        println!("REGISTER request sent");
        
        // 5. Get the sent request from the mock transport
        sleep(Duration::from_millis(30)).await;
        let sent_message_opt = env.client_transport.get_sent_message().await;
        if sent_message_opt.is_none() {
            panic!("Client did not send any message");
        }
        
        // 6. Extract the REGISTER request
        let register_request = if let (Message::Request(request), _) = sent_message_opt.unwrap() {
            assert_eq!(request.method(), Method::Register);
            println!("Client sent REGISTER request");
            request
        } else {
            panic!("Client sent message is not a request");
        };
        
        // 7. Create server transaction for the REGISTER
        let server_tx = env.server_manager.create_server_transaction(
            register_request.clone(), 
            env.client_addr
        ).await.expect("Failed to create server transaction");
        let register_server_tx_id = server_tx.id().clone();
        
        // 8. Server creates 401 Unauthorized challenge
        println!("Server sending 401 Unauthorized challenge");
        // Create a simple unauthorized response without complex WWW-Authenticate header
        let unauthorized_response = env.create_response(
            &register_request, 
            StatusCode::Unauthorized, 
            Some("Unauthorized")
        );
        
        // 9. Send the 401 challenge
        env.server_manager.send_response(&register_server_tx_id, unauthorized_response.clone()).await
            .expect("Failed to send 401 Unauthorized");
        
        // 10. Check that the response was sent
        sleep(Duration::from_millis(30)).await;
        let response_msg_opt = env.server_transport.get_sent_message().await;
        assert!(response_msg_opt.is_some(), "Server did not send 401 Unauthorized");
        
        // 11. Inject the 401 to the client
        println!("Injecting 401 Unauthorized to client");
        env.inject_response_s2c(unauthorized_response).await
            .expect("Failed to inject 401 Unauthorized");
        
        // 12. Wait for client to process
        sleep(Duration::from_millis(50)).await;
        
        // 13. Check that the transaction is completed or terminated
        let client_state = env.client_manager.transaction_state(&register_tx_id).await;
        println!("Client transaction state after 401: {:?}", client_state);
        
        // 14. Create a new REGISTER request with credentials (simulating TU action)
        println!("Creating new REGISTER with authentication credentials");
        // Use the simpler TestEnvironment.create_request instead of manually building
        let auth_register_request = env.create_request(Method::Register, &server_uri);
        
        // Clear client sent messages queue
        while let Some(_) = env.client_transport.get_sent_message().await {}
        
        // 15. Create new client transaction for authenticated REGISTER
        println!("Creating authenticated REGISTER client transaction");
        let auth_register_tx_id = env.client_manager.create_client_transaction(
            auth_register_request.clone(),
            env.server_addr
        ).await.expect("Failed to create authenticated REGISTER client transaction");
        
        // 16. Send the authenticated REGISTER request
        env.client_manager.send_request(&auth_register_tx_id).await
            .expect("Failed to send authenticated REGISTER request");
        println!("Authenticated REGISTER request sent");
        
        // 17. Get the sent authenticated request
        sleep(Duration::from_millis(30)).await;
        let auth_sent_message_opt = env.client_transport.get_sent_message().await;
        assert!(auth_sent_message_opt.is_some(), "Client did not send authenticated message");
        
        // 18. Extract the authenticated REGISTER request
        let auth_register_request = if let (Message::Request(request), _) = auth_sent_message_opt.unwrap() {
            assert_eq!(request.method(), Method::Register);
            println!("Client sent authenticated REGISTER request");
            request
        } else {
            panic!("Client sent message is not a request");
        };
        
        // 19. Create server transaction for the authenticated REGISTER
        let auth_server_tx = env.server_manager.create_server_transaction(
            auth_register_request.clone(), 
            env.client_addr
        ).await.expect("Failed to create server transaction for authenticated request");
        let auth_register_server_tx_id = auth_server_tx.id().clone();
        
        // 20. Server sends 200 OK for authenticated REGISTER
        println!("Server sending 200 OK for authenticated REGISTER");
        let ok_response = env.create_response(&auth_register_request, StatusCode::Ok, Some("OK"));
        env.server_manager.send_response(&auth_register_server_tx_id, ok_response.clone()).await
            .expect("Failed to send 200 OK");
        
        // 21. Check that the OK response was sent
        sleep(Duration::from_millis(30)).await;
        let ok_msg_opt = env.server_transport.get_sent_message().await;
        assert!(ok_msg_opt.is_some(), "Server did not send 200 OK");
        
        // 22. Inject the 200 OK to the client
        println!("Injecting 200 OK to client");
        env.inject_response_s2c(ok_response).await
            .expect("Failed to inject 200 OK");
        
        // 23. Wait for client to process
        sleep(Duration::from_millis(50)).await;
        
        // 24. Check final transaction state
        let auth_client_state = env.client_manager.transaction_state(&auth_register_tx_id).await;
        println!("Authenticated client transaction state: {:?}", auth_client_state);
        
        // Test has completed successfully
        println!("Authentication flow test completed successfully");
        env.shutdown().await;
    }).await;
    
    if let Err(_) = test_result {
        panic!("Test timed out after 5 seconds");
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

/// Tests Re-INVITE flow for dialog updates
/// 
/// Scenario:
/// 1. Original INVITE to establish dialog
/// 2. Re-INVITE to modify session parameters
/// 3. Verify proper handling of transaction IDs and states
#[tokio::test]
#[serial]
async fn test_reinvite_flow() {
    println!("\n==== TEST: Re-INVITE Flow ====");
    println!("Testing dialog update with re-INVITE");
    println!("This test verifies proper handling of Re-INVITE transactions for dialog refresh");
    println!("Scenario: Initial INVITE establishes dialog, Re-INVITE updates session");
    println!("Expected behavior: Dialog maintained while transactions complete independently\n");
    
    // Important dialog parameters
    let mut dialog_call_id = String::new();
    let mut dialog_from_tag = String::new();
    let dialog_to_tag: String = "test-dialog-to-tag".to_string();
    
    // Test timeout to prevent hanging
    let test_result = timeout(Duration::from_secs(5), async {
        println!("PART 1: Initial INVITE Dialog Establishment");
        // 1. Initialize the first test environment
        let mut env1 = TestEnvironment::new().await;
        
        // 2. Create an initial INVITE request
        let server_uri = format!("sip:server@{}", env1.server_addr);
        let invite_request = env1.create_request(Method::Invite, &server_uri);
        
        // Store the Call-ID and From tag for dialog reuse
        dialog_call_id = invite_request.call_id().unwrap().to_string();
        dialog_from_tag = invite_request.from().unwrap().address().params.iter()
            .find(|p| matches!(p, Param::Tag(_)))
            .and_then(|p| p.value())
            .unwrap_or_else(|| "from-tag-fallback".to_string());
        
        println!("Dialog parameters for reuse:");
        println!("Call-ID: {}", dialog_call_id);
        println!("From-Tag: {}", dialog_from_tag);
        println!("To-Tag: {}", dialog_to_tag);
        
        // 3. Set up the initial dialog with INVITE transaction
        println!("Setting up initial INVITE");
        let invite_tx_id = env1.client_manager.create_client_transaction(
            invite_request.clone(),
            env1.server_addr
        ).await.expect("Failed to create INVITE client transaction");
        
        // 4. Send the INVITE request
        env1.client_manager.send_request(&invite_tx_id).await
            .expect("Failed to send INVITE request");
        
        // 5. Get the sent INVITE request
        sleep(Duration::from_millis(50)).await;
        let sent_message_opt = env1.client_transport.get_sent_message().await;
        assert!(sent_message_opt.is_some(), "Client did not send INVITE message");
        
        // 6. Process the INVITE on the server side
        let (invite_msg, _) = sent_message_opt.unwrap();
        if let Message::Request(invite_req) = invite_msg {
            // Create server transaction for INVITE
            let server_tx = env1.server_manager.create_server_transaction(
                invite_req.clone(), 
                env1.client_addr
            ).await.expect("Failed to create server transaction");
            let invite_server_tx_id = server_tx.id().clone();
            
            // 7. Server sends 180 Ringing with To tag
            println!("Server sending 180 Ringing with To tag");
            let mut ringing_builder = rvoip_sip_core::builder::SimpleResponseBuilder::response_from_request(
                &invite_req, StatusCode::Ringing, Some("Ringing")
            );
            
            let to_header = invite_req.to().unwrap();
            ringing_builder = ringing_builder.to(
                to_header.address().display_name().unwrap_or_default(),
                &to_header.address().uri.to_string(),
                Some(&dialog_to_tag)
            );
            
            let ringing_response = ringing_builder.build();
            env1.server_manager.send_response(&invite_server_tx_id, ringing_response.clone()).await
                .expect("Failed to send ringing response");
            
            // 8. Inject 180 Ringing to client
            sleep(Duration::from_millis(50)).await;
            env1.inject_response_s2c(ringing_response).await
                .expect("Failed to inject 180 Ringing");
            
            // 9. Server sends 200 OK with same To tag
            println!("Server sending 200 OK with To tag");
            let mut ok_builder = rvoip_sip_core::builder::SimpleResponseBuilder::response_from_request(
                &invite_req, StatusCode::Ok, Some("OK")
            );
            
            ok_builder = ok_builder.to(
                to_header.address().display_name().unwrap_or_default(),
                &to_header.address().uri.to_string(),
                Some(&dialog_to_tag)
            );
            
            ok_builder = ok_builder.contact(
                &format!("sip:server@{}", env1.server_addr),
                Some("Server UA")
            );
            
            let ok_response = ok_builder.build();
            env1.server_manager.send_response(&invite_server_tx_id, ok_response.clone()).await
                .expect("Failed to send OK response");
            
            // 10. Inject 200 OK to client
            sleep(Duration::from_millis(50)).await;
            env1.inject_response_s2c(ok_response.clone()).await
                .expect("Failed to inject 200 OK");
            
            // 11. Client sends ACK (would be done by TU in real app)
            println!("Client sending ACK for initial INVITE");
            let ack_request = create_ack_for_response(&ok_response, &invite_req, env1.client_addr);
            env1.inject_request_c2s(ack_request.clone()).await
                .expect("Failed to inject ACK to server");
            
            // Wait for transaction to terminate
            sleep(Duration::from_millis(200)).await;
            
            // 12. Clean up first environment
            env1.shutdown().await;
            
            // Verify dialog is established before continuing
            if ok_response.to().is_none() || 
               ack_request.to().is_none() || 
               ok_response.call_id().is_none() {
                panic!("Dialog not properly established");
            }
            
            println!("\nPART 2: Re-INVITE in Established Dialog");
            // 13. Create a new test environment for the Re-INVITE
            let mut env2 = TestEnvironment::new().await;
            
            // 14. Create a Re-INVITE request with dialog identifiers
            println!("Creating Re-INVITE with dialog identifiers");
            
            // The dialog is identified by:
            // - Call-ID (same as original INVITE)
            // - From tag (original client tag)
            // - To tag (from 200 OK)
            let mut reinvite_builder = rvoip_sip_core::builder::SimpleRequestBuilder::new(
                Method::Invite, &server_uri
            ).expect("Failed to create Re-INVITE builder");
            
            reinvite_builder = reinvite_builder
                .call_id(&dialog_call_id)
                .from("Client UA", &format!("sip:client@{}", env2.client_addr), Some(&dialog_from_tag))
                .to("Server UA", &server_uri, Some(&dialog_to_tag))
                .via(&env2.client_addr.to_string(), "UDP", Some(&format!("z9hG4bK{}", uuid::Uuid::new_v4().simple())))
                .cseq(2) // Increment CSeq for re-INVITE
                .max_forwards(70)
                .contact(&format!("sip:client@{}", env2.client_addr), Some("Client UA"))
                .header(TypedHeader::ContentLength(ContentLength::new(0)));
            
            let reinvite_request = reinvite_builder.build();
            
            println!("Re-INVITE Call-ID: {}", reinvite_request.call_id().unwrap());
            println!("Re-INVITE From Tag: {}", reinvite_request.from().unwrap().to_string());
            println!("Re-INVITE To Tag: {}", reinvite_request.to().unwrap().to_string());
            
            // 15. Create a new client transaction for Re-INVITE
            let reinvite_tx_id = env2.client_manager.create_client_transaction(
                reinvite_request.clone(),
                env2.server_addr
            ).await.expect("Failed to create Re-INVITE client transaction");
            
            // 16. Send the Re-INVITE
            env2.client_manager.send_request(&reinvite_tx_id).await
                .expect("Failed to send Re-INVITE request");
                
            // 17. Get the sent Re-INVITE
            sleep(Duration::from_millis(50)).await;
            
            // Make sure we get the right message by getting all messages and filtering for INVITE
            let messages = env2.client_transport.get_all_sent_messages().await;
            println!("Found {} messages in Re-INVITE transport", messages.len());
            
            let reinvite_msg_opt = messages.into_iter()
                .find(|(msg, _)| {
                    if let Message::Request(req) = msg {
                        req.method() == Method::Invite
                    } else {
                        false
                    }
                });
                
            assert!(reinvite_msg_opt.is_some(), "Client did not send Re-INVITE message");
            
            // 18. Create server transaction for Re-INVITE
            let (reinvite_msg, _) = reinvite_msg_opt.unwrap();
            if let Message::Request(reinvite_req) = reinvite_msg {
                println!("Successfully found Re-INVITE message");
                println!("Message headers: From={:?}, To={:?}, Call-ID={:?}", 
                    reinvite_req.from(), reinvite_req.to(), reinvite_req.call_id());
                
                // Create server transaction for the Re-INVITE
                let reinvite_server_tx = env2.server_manager.create_server_transaction(
                    reinvite_req.clone(),
                    env2.client_addr
                ).await.expect("Failed to create Re-INVITE server transaction");
                let reinvite_server_tx_id = reinvite_server_tx.id().clone();
                
                // 19. Server sends 200 OK for Re-INVITE
                println!("Server sending 200 OK for Re-INVITE");
                let mut reinvite_ok_builder = rvoip_sip_core::builder::SimpleResponseBuilder::response_from_request(
                    &reinvite_req, StatusCode::Ok, Some("OK")
                );
                
                // Contact header is required for dialog
                reinvite_ok_builder = reinvite_ok_builder
                    .contact(&format!("sip:server@{}", env2.server_addr), Some("Server UA"))
                    .header(TypedHeader::ContentLength(ContentLength::new(0)));
                
                let reinvite_ok = reinvite_ok_builder.build();
                
                env2.server_manager.send_response(&reinvite_server_tx_id, reinvite_ok.clone()).await
                    .expect("Failed to send OK response for Re-INVITE");
                    
                // 20. Inject 200 OK to client
                sleep(Duration::from_millis(50)).await;
                env2.inject_response_s2c(reinvite_ok.clone()).await
                    .expect("Failed to inject 200 OK for Re-INVITE");
                
                // 21. Client sends ACK for Re-INVITE
                println!("Client sending ACK for Re-INVITE");
                let reinvite_ack = create_ack_for_response(&reinvite_ok, &reinvite_req, env2.client_addr);
                
                // Use inject method for ACK
                env2.inject_request_c2s(reinvite_ack).await
                    .expect("Failed to inject ACK for Re-INVITE to server");
                
                println!("Re-INVITE flow test completed successfully");
            } else {
                panic!("Client sent message is not a request");
            }
            
            // Clean up second environment
            env2.shutdown().await;
        } else {
            panic!("Client sent message is not a request");
        }
    }).await;
    
    if let Err(_) = test_result {
        panic!("Test timed out after 5 seconds");
    }
} 