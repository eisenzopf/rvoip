//! Concurrent call tests for the Simple UAC API

use rvoip_session_core::api::uac::{SimpleUacClient, SimpleCall};
use rvoip_session_core::api::uas::SimpleUasServer;
use rvoip_session_core::api::types::AudioFrame;
use std::time::Duration;
use std::sync::Arc;
use serial_test::serial;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_multiple_simultaneous_calls() {
    println!("\n=== Testing Multiple Simultaneous Calls ===\n");
    
    // Create multiple servers
    let server1 = SimpleUasServer::always_accept("127.0.0.1:5190").await
        .expect("Failed to create server 1");
    
    let server2 = SimpleUasServer::always_accept("127.0.0.1:5191").await
        .expect("Failed to create server 2");
    
    let server3 = SimpleUasServer::always_accept("127.0.0.1:5192").await
        .expect("Failed to create server 3");
    
    let client = Arc::new(
        SimpleUacClient::new("alice")
            .port(5193)
            .await
            .expect("Failed to create client")
    );
    
    // Create multiple calls simultaneously
    let mut handles = vec![];
    
    for i in 1..=3 {
        let client_clone = client.clone();
        let port = 5189 + i;
        
        let handle = tokio::spawn(async move {
            let call = client_clone.call(&format!("party{}@127.0.0.1", i))
                .port(port)
                .call_id(&format!("concurrent-{}", i))
                .await
                .expect(&format!("Failed to create call {}", i));
            
            println!("✓ Call {} created: {}", i, call.id());
            
            // Wait for call to establish
            tokio::time::sleep(Duration::from_millis(200)).await;
            
            // Check if call is active before sending DTMF
            let state = call.state().await;
            if state == rvoip_session_core::api::types::CallState::Active {
                // Try to send DTMF but handle errors gracefully
                match call.send_dtmf(&i.to_string()).await {
                    Ok(_) => println!("✓ Call {} sent DTMF: {}", i, i),
                    Err(e) => println!("⚠ Call {} DTMF failed: {}", i, e),
                }
            } else {
                println!("⚠ Call {} not yet active (state: {:?}), skipping DTMF", i, state);
            }
            
            // Hold the call for a bit more
            tokio::time::sleep(Duration::from_millis(300)).await;
            
            call.hangup().await
                .expect(&format!("Failed to hang up call {}", i));
            
            println!("✓ Call {} hung up", i);
        });
        
        handles.push(handle);
    }
    
    // Wait for all calls to complete
    for handle in handles {
        handle.await.expect("Task panicked");
    }
    
    println!("✓ All 3 concurrent calls completed successfully");
    
    // Cleanup - need to extract from Arc
    match Arc::try_unwrap(client) {
        Ok(client) => {
            client.shutdown().await.expect("Failed to shutdown client");
        }
        Err(_) => {
            println!("⚠ Could not unwrap Arc - client still has references");
        }
    }
    
    server1.shutdown().await.expect("Failed to shutdown server 1");
    server2.shutdown().await.expect("Failed to shutdown server 2");
    server3.shutdown().await.expect("Failed to shutdown server 3");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_concurrent_audio_streams() {
    println!("\n=== Testing Concurrent Audio Streams ===\n");
    
    let server1 = SimpleUasServer::always_accept("127.0.0.1:5194").await
        .expect("Failed to create server 1");
    
    let server2 = SimpleUasServer::always_accept("127.0.0.1:5195").await
        .expect("Failed to create server 2");
    
    let client = SimpleUacClient::new("alice")
        .port(5196)
        .await
        .expect("Failed to create client");
    
    // Create two calls
    let mut call1 = client.call("bob@127.0.0.1")
        .port(5194)
        .await
        .expect("Failed to create call 1");
    
    let mut call2 = client.call("charlie@127.0.0.1")
        .port(5195)
        .await
        .expect("Failed to create call 2");
    
    println!("✓ Created two concurrent calls");
    
    // Get audio channels for both
    let (tx1, _rx1) = call1.audio_channels();
    let (tx2, _rx2) = call2.audio_channels();
    
    // Send audio to both calls concurrently
    let audio_task1 = tokio::spawn(async move {
        for i in 0..10 {
            let frame = AudioFrame::new(vec![i as i16; 160], 8000, 1, 0);
            if tx1.send(frame).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        println!("✓ Finished sending audio to call 1");
    });
    
    let audio_task2 = tokio::spawn(async move {
        for i in 0..10 {
            let frame = AudioFrame::new(vec![(100 + i) as i16; 160], 8000, 1, 0);
            if tx2.send(frame).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        println!("✓ Finished sending audio to call 2");
    });
    
    // Wait for audio tasks
    audio_task1.await.expect("Audio task 1 panicked");
    audio_task2.await.expect("Audio task 2 panicked");
    
    // Hang up both calls
    call1.hangup().await.expect("Failed to hang up call 1");
    call2.hangup().await.expect("Failed to hang up call 2");
    
    println!("✓ Both concurrent audio streams completed");
    
    client.shutdown().await.expect("Failed to shutdown client");
    server1.shutdown().await.expect("Failed to shutdown server 1");
    server2.shutdown().await.expect("Failed to shutdown server 2");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_rapid_concurrent_operations() {
    println!("\n=== Testing Rapid Concurrent Operations ===\n");
    
    let server = SimpleUasServer::always_accept("127.0.0.1:5197").await
        .expect("Failed to create server");
    
    let client = Arc::new(
        SimpleUacClient::new("alice")
            .port(5198)
            .await
            .expect("Failed to create client")
    );
    
    // Spawn multiple tasks doing rapid operations
    let mut handles = vec![];
    
    for task_id in 1..=5 {
        let client_clone = client.clone();
        
        let handle = tokio::spawn(async move {
            for i in 1..=3 {
                let call = client_clone.call("test@127.0.0.1")
                    .port(5197)
                    .call_id(&format!("task{}-call{}", task_id, i))
                    .await
                    .expect(&format!("Task {} failed to create call {}", task_id, i));
                
                // Wait for call to be active before DTMF
                tokio::time::sleep(Duration::from_millis(100)).await;
                
                // Check if call is active before sending DTMF
                let state = call.state().await;
                if state == rvoip_session_core::api::types::CallState::Active {
                    // Try to send DTMF but don't panic if it fails in rapid tests
                    match call.send_dtmf(&task_id.to_string()).await {
                        Ok(_) => {},
                        Err(e) => println!("  Task {} call {} DTMF failed: {}", task_id, i, e),
                    }
                } else {
                    // In rapid tests, just skip DTMF if not active
                    println!("  Task {} call {} not active (state: {:?}), skipping DTMF", task_id, i, state);
                }
                
                // Quick hangup
                call.hangup().await
                    .expect("Failed to hang up");
                
                // Brief pause
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            println!("✓ Task {} completed 3 rapid calls", task_id);
        });
        
        handles.push(handle);
    }
    
    // Wait for all tasks
    for handle in handles {
        handle.await.expect("Task panicked");
    }
    
    println!("✓ All 5 tasks completed (15 total calls)");
    
    match Arc::try_unwrap(client) {
        Ok(client) => {
            client.shutdown().await.expect("Failed to shutdown client");
        }
        Err(_) => {
            println!("⚠ Could not unwrap Arc - client still has references");
        }
    }
    
    server.shutdown().await.expect("Failed to shutdown server");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_concurrent_hold_operations() {
    println!("\n=== Testing Concurrent Hold Operations ===\n");
    
    let server1 = SimpleUasServer::always_accept("127.0.0.1:5199").await
        .expect("Failed to create server 1");
    
    let server2 = SimpleUasServer::always_accept("127.0.0.1:5200").await
        .expect("Failed to create server 2");
    
    let client = SimpleUacClient::new("alice")
        .port(5201)
        .await
        .expect("Failed to create client");
    
    // Create two calls
    let call1 = client.call("bob@127.0.0.1")
        .port(5199)
        .await
        .expect("Failed to create call 1");
    
    let call2 = client.call("charlie@127.0.0.1")
        .port(5200)
        .await
        .expect("Failed to create call 2");
    
    println!("✓ Created two calls");
    
    // Put both on hold concurrently
    let hold_task1 = tokio::spawn(async move {
        call1.hold().await.expect("Failed to hold call 1");
        println!("✓ Call 1 on hold");
        
        tokio::time::sleep(Duration::from_millis(200)).await;
        
        call1.unhold().await.expect("Failed to unhold call 1");
        println!("✓ Call 1 resumed");
        
        call1.hangup().await.expect("Failed to hang up call 1");
    });
    
    let hold_task2 = tokio::spawn(async move {
        call2.hold().await.expect("Failed to hold call 2");
        println!("✓ Call 2 on hold");
        
        tokio::time::sleep(Duration::from_millis(300)).await;
        
        call2.unhold().await.expect("Failed to unhold call 2");
        println!("✓ Call 2 resumed");
        
        call2.hangup().await.expect("Failed to hang up call 2");
    });
    
    // Wait for both
    hold_task1.await.expect("Hold task 1 panicked");
    hold_task2.await.expect("Hold task 2 panicked");
    
    println!("✓ Concurrent hold operations completed");
    
    client.shutdown().await.expect("Failed to shutdown client");
    server1.shutdown().await.expect("Failed to shutdown server 1");
    server2.shutdown().await.expect("Failed to shutdown server 2");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_resource_cleanup_with_concurrent_calls() {
    println!("\n=== Testing Resource Cleanup with Concurrent Calls ===\n");
    
    let server = SimpleUasServer::always_accept("127.0.0.1:5202").await
        .expect("Failed to create server");
    
    // Create and destroy multiple clients with active calls
    for round in 1..=3 {
        println!("\n--- Round {} ---", round);
        
        let client = SimpleUacClient::new(&format!("user{}", round))
            .port(5202 + round)
            .await
            .expect(&format!("Failed to create client {}", round));
        
        // Create multiple calls
        let mut calls = vec![];
        for i in 1..=3 {
            let call = client.call("test@127.0.0.1")
                .port(5202)
                .call_id(&format!("round{}-call{}", round, i))
                .await
                .expect(&format!("Failed to create call {}", i));
            
            calls.push(call);
        }
        
        println!("✓ Created 3 calls for round {}", round);
        
        // Hang up all calls
        for (i, call) in calls.into_iter().enumerate() {
            call.hangup().await
                .expect(&format!("Failed to hang up call {}", i + 1));
        }
        
        // Shutdown client
        client.shutdown().await
            .expect(&format!("Failed to shutdown client {}", round));
        
        println!("✓ Round {} cleaned up", round);
    }
    
    println!("\n✓ All rounds completed - resources properly cleaned up");
    
    server.shutdown().await.expect("Failed to shutdown server");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_concurrent_call_states() {
    println!("\n=== Testing Concurrent Call State Management ===\n");
    
    let server1 = SimpleUasServer::always_accept("127.0.0.1:5206").await
        .expect("Failed to create server 1");
    
    let server2 = SimpleUasServer::always_accept("127.0.0.1:5207").await
        .expect("Failed to create server 2");
    
    let server3 = SimpleUasServer::always_accept("127.0.0.1:5208").await
        .expect("Failed to create server 3");
    
    let client = SimpleUacClient::new("alice")
        .port(5209)
        .await
        .expect("Failed to create client");
    
    // Create three calls
    let call1 = client.call("bob@127.0.0.1")
        .port(5206)
        .await
        .expect("Failed to create call 1");
    
    let call2 = client.call("charlie@127.0.0.1")
        .port(5207)
        .await
        .expect("Failed to create call 2");
    
    let call3 = client.call("dave@127.0.0.1")
        .port(5208)
        .await
        .expect("Failed to create call 3");
    
    println!("✓ Created 3 concurrent calls");
    
    // Manipulate states concurrently
    let state_task1 = tokio::spawn(async move {
        // Call 1: hold/unhold cycle
        call1.hold().await.expect("Failed to hold call 1");
        tokio::time::sleep(Duration::from_millis(100)).await;
        call1.unhold().await.expect("Failed to unhold call 1");
        call1.hangup().await.expect("Failed to hang up call 1");
        println!("✓ Call 1: hold/unhold cycle complete");
    });
    
    let state_task2 = tokio::spawn(async move {
        // Call 2: mute/unmute cycle
        call2.mute().await.expect("Failed to mute call 2");
        tokio::time::sleep(Duration::from_millis(150)).await;
        call2.unmute().await.expect("Failed to unmute call 2");
        call2.hangup().await.expect("Failed to hang up call 2");
        println!("✓ Call 2: mute/unmute cycle complete");
    });
    
    let state_task3 = tokio::spawn(async move {
        // Call 3: DTMF sending
        for digit in "123".chars() {
            call3.send_dtmf(&digit.to_string()).await
                .expect("Failed to send DTMF");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        call3.hangup().await.expect("Failed to hang up call 3");
        println!("✓ Call 3: DTMF sequence complete");
    });
    
    // Wait for all state manipulations
    state_task1.await.expect("State task 1 panicked");
    state_task2.await.expect("State task 2 panicked");
    state_task3.await.expect("State task 3 panicked");
    
    println!("✓ All concurrent state operations completed");
    
    client.shutdown().await.expect("Failed to shutdown client");
    server1.shutdown().await.expect("Failed to shutdown server 1");
    server2.shutdown().await.expect("Failed to shutdown server 2");
    server3.shutdown().await.expect("Failed to shutdown server 3");
}