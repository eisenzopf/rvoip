//! Debug test to identify where tests are hanging

use rvoip_client_core::{ClientBuilder};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_minimal_client_creation() {
    println!("Test starting...");
    
    // Try with timeout on the whole test
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        println!("Creating client builder...");
        let builder = ClientBuilder::new()
            .user_agent("DebugTest/1.0")
            .local_address("127.0.0.1:15900".parse().unwrap());
        
        println!("Building client...");
        let client = builder.build().await;
        
        match client {
            Ok(c) => {
                println!("Client built successfully");
                Ok(c)
            }
            Err(e) => {
                println!("Client build failed: {}", e);
                Err(e)
            }
        }
    }).await;
    
    match result {
        Ok(Ok(_)) => println!("Test passed"),
        Ok(Err(e)) => panic!("Client creation failed: {}", e),
        Err(_) => panic!("Test timed out - likely hanging in client creation"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_minimal_no_client() {
    println!("Simple test without client - should complete");
    tokio::time::sleep(Duration::from_millis(100)).await;
    println!("Test completed");
} 