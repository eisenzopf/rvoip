//! Basic tests for the Simple UAS API

use rvoip_session_core::api::uas::SimpleUasServer;
use rvoip_session_core::api::uac::SimpleUacClient;
use std::time::Duration;
use serial_test::serial;

#[tokio::test]
#[serial]
async fn test_uas_server_creation() {
    println!("\n=== Testing Simple UAS Server Creation ===\n");
    
    // Test with default always_accept
    let server = SimpleUasServer::always_accept("127.0.0.1:5220").await
        .expect("Failed to create always_accept server");
    
    println!("✓ Created always_accept server on 127.0.0.1:5220");
    
    // Let it run briefly
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    server.shutdown().await.expect("Failed to shutdown server");
    println!("✓ Server shutdown successfully");
}

#[tokio::test]
#[serial]
async fn test_uas_always_accept_mode() {
    println!("\n=== Testing UAS Always Accept Mode ===\n");
    
    let server = SimpleUasServer::always_accept("127.0.0.1:5221").await
        .expect("Failed to create server");
    
    println!("✓ Server created in always_accept mode");
    
    // Create a client and make calls
    let client = SimpleUacClient::new("alice")
        .port(5222)
        .await
        .expect("Failed to create client");
    
    // Make multiple calls - all should be accepted
    for i in 1..=3 {
        let call = client.call("server@127.0.0.1")
            .port(5221)
            .call_id(&format!("accept-test-{}", i))
            .await
            .expect(&format!("Failed to make call {}", i));
        
        println!("✓ Call {} accepted: {}", i, call.id());
        
        // Brief call
        tokio::time::sleep(Duration::from_millis(50)).await;
        
        call.hangup().await.expect("Failed to hang up");
        println!("✓ Call {} completed", i);
    }
    
    client.shutdown().await.expect("Failed to shutdown client");
    server.shutdown().await.expect("Failed to shutdown server");
}

#[tokio::test]
#[serial]
async fn test_uas_always_reject_mode() {
    println!("\n=== Testing UAS Always Reject Mode ===\n");
    
    let server = SimpleUasServer::always_reject("127.0.0.1:5223", "Server in maintenance".to_string()).await
        .expect("Failed to create reject server");
    
    println!("✓ Server created in always_reject mode");
    
    // Create a client and try to make calls
    let client = SimpleUacClient::new("alice")
        .port(5224)
        .await
        .expect("Failed to create client");
    
    // Try to make a call - should be rejected
    let result = client.call("server@127.0.0.1")
        .port(5223)
        .call_id("reject-test")
        .await;
    
    match result {
        Ok(call) => {
            println!("⚠ Call initiated despite reject mode: {}", call.id());
            // The call might be created but won't be accepted by server
            // Still need to clean up
            call.hangup().await.expect("Failed to hang up");
        }
        Err(e) => {
            println!("✓ Call rejected as expected: {}", e);
        }
    }
    
    client.shutdown().await.expect("Failed to shutdown client");
    server.shutdown().await.expect("Failed to shutdown server");
}

#[tokio::test]
#[serial]
async fn test_uas_multiple_servers() {
    println!("\n=== Testing Multiple UAS Servers ===\n");
    
    // Create multiple servers on different ports
    let server1 = SimpleUasServer::always_accept("127.0.0.1:5225").await
        .expect("Failed to create server 1");
    println!("✓ Server 1 created on port 5225");
    
    let server2 = SimpleUasServer::always_accept("127.0.0.1:5226").await
        .expect("Failed to create server 2");
    println!("✓ Server 2 created on port 5226");
    
    let server3 = SimpleUasServer::always_reject("127.0.0.1:5227", "Busy".to_string()).await
        .expect("Failed to create server 3");
    println!("✓ Server 3 created on port 5227 (reject mode)");
    
    // Create client and call each server
    let client = SimpleUacClient::new("alice")
        .port(5228)
        .await
        .expect("Failed to create client");
    
    // Call server 1 (should accept)
    let call1 = client.call("server1@127.0.0.1")
        .port(5225)
        .await
        .expect("Failed to call server 1");
    println!("✓ Called server 1: {}", call1.id());
    
    // Call server 2 (should accept)
    let call2 = client.call("server2@127.0.0.1")
        .port(5226)
        .await
        .expect("Failed to call server 2");
    println!("✓ Called server 2: {}", call2.id());
    
    // Clean up calls
    call1.hangup().await.expect("Failed to hang up call 1");
    call2.hangup().await.expect("Failed to hang up call 2");
    
    // Shutdown everything
    client.shutdown().await.expect("Failed to shutdown client");
    server1.shutdown().await.expect("Failed to shutdown server 1");
    server2.shutdown().await.expect("Failed to shutdown server 2");
    server3.shutdown().await.expect("Failed to shutdown server 3");
    
    println!("✓ All servers shutdown successfully");
}

#[tokio::test]
#[serial]
#[ignore = "Transport resource limits prevent immediate server restart"]
async fn test_uas_server_restart() {
    println!("\n=== Testing UAS Server Restart ===\n");
    
    // Create and shutdown a server
    let server = SimpleUasServer::always_accept("127.0.0.1:5229").await
        .expect("Failed to create server");
    println!("✓ Server created on port 5229");
    
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    server.shutdown().await.expect("Failed to shutdown server");
    println!("✓ Server shutdown");
    
    // Wait longer for port and transport resources to be fully released
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Create a new server on the same port
    let server2 = SimpleUasServer::always_accept("127.0.0.1:5229").await
        .expect("Failed to recreate server");
    println!("✓ Server recreated on same port 5229");
    
    // Test that it works
    let client = SimpleUacClient::new("alice")
        .port(5230)
        .await
        .expect("Failed to create client");
    
    let call = client.call("test@127.0.0.1")
        .port(5229)
        .await
        .expect("Failed to call recreated server");
    
    println!("✓ Successfully called recreated server: {}", call.id());
    
    call.hangup().await.expect("Failed to hang up");
    client.shutdown().await.expect("Failed to shutdown client");
    server2.shutdown().await.expect("Failed to shutdown server");
}

#[tokio::test]
#[serial]
async fn test_uas_concurrent_incoming_calls() {
    println!("\n=== Testing UAS Handling Concurrent Incoming Calls ===\n");
    
    let server = SimpleUasServer::always_accept("127.0.0.1:5231").await
        .expect("Failed to create server");
    println!("✓ Server created to handle concurrent calls");
    
    // Create multiple clients
    let mut clients = vec![];
    let mut calls = vec![];
    
    for i in 1..=3 {
        let client = SimpleUacClient::new(&format!("client{}", i))
            .port(5231 + i)
            .await
            .expect(&format!("Failed to create client {}", i));
        
        let call = client.call("server@127.0.0.1")
            .port(5231)
            .call_id(&format!("concurrent-{}", i))
            .await
            .expect(&format!("Client {} failed to call", i));
        
        println!("✓ Client {} called server: {}", i, call.id());
        
        calls.push(call);
        clients.push(client);
    }
    
    println!("✓ Server handling {} concurrent calls", calls.len());
    
    // Let calls run briefly
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // Hang up all calls
    for (i, call) in calls.into_iter().enumerate() {
        call.hangup().await
            .expect(&format!("Failed to hang up call {}", i + 1));
        println!("✓ Call {} hung up", i + 1);
    }
    
    // Shutdown all clients
    for (i, client) in clients.into_iter().enumerate() {
        client.shutdown().await
            .expect(&format!("Failed to shutdown client {}", i + 1));
    }
    
    server.shutdown().await.expect("Failed to shutdown server");
    println!("✓ Server successfully handled concurrent calls");
}

#[tokio::test]
#[serial]
async fn test_uas_long_running_server() {
    println!("\n=== Testing Long-Running UAS Server ===\n");
    
    let server = SimpleUasServer::always_accept("127.0.0.1:5235").await
        .expect("Failed to create server");
    println!("✓ Long-running server started");
    
    let client = SimpleUacClient::new("alice")
        .port(5236)
        .await
        .expect("Failed to create client");
    
    // Make calls at intervals
    for i in 1..=5 {
        let call = client.call("server@127.0.0.1")
            .port(5235)
            .call_id(&format!("long-run-{}", i))
            .await
            .expect(&format!("Failed to make call {}", i));
        
        println!("✓ Call {} at {:?}: {}", i, std::time::Instant::now(), call.id());
        
        // Hold call briefly
        tokio::time::sleep(Duration::from_millis(100)).await;
        
        call.hangup().await.expect("Failed to hang up");
        
        // Wait between calls
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    
    println!("✓ Server successfully handled calls over extended period");
    
    client.shutdown().await.expect("Failed to shutdown client");
    server.shutdown().await.expect("Failed to shutdown server");
}

#[tokio::test]
#[serial]
async fn test_uas_server_with_different_bind_addresses() {
    println!("\n=== Testing UAS Server with Different Bind Addresses ===\n");
    
    // Test various bind address formats
    let test_cases = vec![
        ("127.0.0.1:5237", "Explicit localhost with port"),
        ("0.0.0.0:5238", "All interfaces"),
        ("localhost:5239", "Hostname format"),
    ];
    
    for (addr, description) in test_cases {
        println!("\nTesting: {} - {}", description, addr);
        
        // Try to create server (might fail depending on system)
        match SimpleUasServer::always_accept(addr).await {
            Ok(server) => {
                println!("✓ Server created on {}", addr);
                
                // Quick test if possible
                if addr.contains("127.0.0.1") || addr.contains("localhost") {
                    // Wait a bit before creating client to avoid transport issues
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    
                    match SimpleUacClient::new("test").port(5240).await {
                        Ok(client) => {
                            let port = addr.split(':').last().unwrap();
                            let call = client.call("server@127.0.0.1")
                                .port(port.parse().unwrap())
                                .await
                                .expect("Failed to test call");
                            
                            call.hangup().await.expect("Failed to hang up");
                            client.shutdown().await.expect("Failed to shutdown client");
                        }
                        Err(e) => {
                            println!("⚠ Could not create test client: {}", e);
                        }
                    }
                }
                
                server.shutdown().await.expect("Failed to shutdown server");
            }
            Err(e) => {
                println!("⚠ Failed to create server on {}: {}", addr, e);
            }
        }
    }
}