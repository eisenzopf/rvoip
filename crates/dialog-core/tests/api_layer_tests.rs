//! API Layer Tests for Dialog Core
//!
//! Tests for the high-level API functionality provided by dialog-core.

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::time::{timeout, Duration};

use rvoip_dialog_core::api::{DialogClient, DialogServer, DialogApi};
use rvoip_dialog_core::DialogId;
use rvoip_dialog_core::transaction::builders::{client_quick}; // Use the new builders
use rvoip_sip_core::{Request, Method, StatusCode, Uri, TypedHeader, ContentLength};
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_dialog_core::transaction::TransactionManager;
use rvoip_dialog_core::transaction::transport::{TransportManager, TransportManagerConfig};
use uuid::Uuid;

/// Test environment for dialog-core API testing using **REAL TRANSPORT**
struct DialogApiTestEnvironment {
    pub server: Arc<DialogServer>,
    pub client: Arc<DialogClient>,
    #[allow(dead_code)]
    pub server_transport: TransportManager,
    #[allow(dead_code)]
    pub client_transport: TransportManager,
    pub server_addr: SocketAddr,
    pub client_addr: SocketAddr,
}

impl DialogApiTestEnvironment {
    /// Create a new test environment with **REAL UDP TRANSPORT**
    async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        // Set test environment
        std::env::set_var("RVOIP_TEST", "1");
        
        // ------------- Server setup with REAL TRANSPORT -----------------
        
        // Create a transport manager for the server (real UDP)
        let server_config = TransportManagerConfig {
            enable_udp: true,
            enable_tcp: false,
            enable_ws: false,
            enable_tls: false,
            bind_addresses: vec!["127.0.0.1:0".parse()?], // Use ephemeral port
            ..Default::default()
        };
        
        let (mut server_transport, server_transport_rx) = TransportManager::new(server_config).await?;
        server_transport.initialize().await?;
        
        // Get the actual server address
        let server_addr = server_transport.default_transport().await
            .ok_or("No default transport")?.local_addr()?;
        
        println!("✅ Server bound to real UDP transport: {}", server_addr);
        
        // Create a transaction manager for the server with REAL transport
        let (server_transaction_manager, _server_events) = TransactionManager::with_transport_manager(
            server_transport.clone(),
            server_transport_rx,
            Some(100),
        ).await?;
        
        // ------------- Client setup with REAL TRANSPORT -----------------
        
        // Create a transport manager for the client (real UDP)
        let client_config = TransportManagerConfig {
            enable_udp: true,
            enable_tcp: false,
            enable_ws: false,
            enable_tls: false,
            bind_addresses: vec!["127.0.0.1:0".parse()?], // Use ephemeral port
            ..Default::default()
        };
        
        let (mut client_transport, client_transport_rx) = TransportManager::new(client_config).await?;
        client_transport.initialize().await?;
        
        // Get the actual client address
        let client_addr = client_transport.default_transport().await
            .ok_or("No default transport")?.local_addr()?;
        
        println!("✅ Client bound to real UDP transport: {}", client_addr);
        
        // Create a transaction manager for the client with REAL transport
        let (client_transaction_manager, _client_events) = TransactionManager::with_transport_manager(
            client_transport.clone(),
            client_transport_rx,
            Some(100),
        ).await?;
        
        // ------------- Create DialogServer and DialogClient -----------------
        
        // Create dialog server and client with REAL transaction managers
        let server_config = rvoip_dialog_core::api::config::ServerConfig::default();
        let client_config = rvoip_dialog_core::api::config::ClientConfig::default();
        
        let server = DialogServer::with_dependencies(
            Arc::new(server_transaction_manager),
            server_config
        ).await?;
        
        let client = DialogClient::with_dependencies(
            Arc::new(client_transaction_manager),
            client_config
        ).await?;
        
        Ok(Self {
            server: Arc::new(server),
            client: Arc::new(client),
            server_transport,
            client_transport,
            server_addr,
            client_addr,
        })
    }
    
    /// Create a proper SIP request with all required headers for REAL transport
    fn create_sip_request(&self, method: Method, to_uri: &str) -> Request {
        match method {
            Method::Invite => {
                // Use the new INVITE builder
                let from_uri = format!("sip:client@{}", self.client_addr);
                client_quick::invite(&from_uri, to_uri, self.client_addr, None)
                    .expect("Failed to create INVITE request")
            },
            _ => {
                // For other methods, use the fallback manual construction
                let branch = format!("z9hG4bK-{}", Uuid::new_v4().to_string().replace("-", ""));
                let call_id = format!("real-test-{}", Uuid::new_v4().to_string().replace("-", ""));
                let from_tag = format!("from-{}", Uuid::new_v4().to_string().replace("-", "").chars().take(8).collect::<String>());
                let from_uri = format!("sip:client@{}", self.client_addr);
                
                SimpleRequestBuilder::new(method, to_uri)
                    .expect("Failed to create request builder")
                    .from("Client UA", &from_uri, Some(&from_tag))
                    .to("Server UA", to_uri, None)
                    .call_id(&call_id)
                    .cseq(1)
                    .via(&self.client_addr.to_string(), "UDP", Some(&branch))
                    .max_forwards(70)
                    .header(TypedHeader::ContentLength(ContentLength::new(0)))
                    .build()
            }
        }
    }
    
    /// Shutdown the test environment
    async fn shutdown(self) {
        if let Err(e) = self.server.stop().await {
            println!("Warning: Error stopping server: {:?}", e);
        }
        if let Err(e) = self.client.stop().await {
            println!("Warning: Error stopping client: {:?}", e);
        }
        
        // Transport managers don't need explicit shutdown
        // They will be dropped automatically
        
        std::env::remove_var("RVOIP_TEST");
        println!("✅ Test environment shutdown complete");
    }
}

/// Test DialogServer basic lifecycle with REAL transport
#[tokio::test]
async fn test_dialog_server_lifecycle_real_transport() -> Result<(), Box<dyn std::error::Error>> {
    let env = DialogApiTestEnvironment::new().await?;
    
    println!("🚀 Testing DialogServer lifecycle with REAL UDP transport");
    
    // Test server lifecycle
    timeout(Duration::from_secs(5), async {
        env.server.start().await?;
        println!("✅ DialogServer started successfully");
        
        env.server.stop().await?;
        println!("✅ DialogServer stopped successfully");
        
        Ok::<(), Box<dyn std::error::Error>>(())
    }).await??;
    
    env.shutdown().await;
    Ok(())
}

/// Test DialogClient basic lifecycle with REAL transport
#[tokio::test]
async fn test_dialog_client_lifecycle_real_transport() -> Result<(), Box<dyn std::error::Error>> {
    let env = DialogApiTestEnvironment::new().await?;
    
    println!("🚀 Testing DialogClient lifecycle with REAL UDP transport");
    
    // Test client lifecycle
    timeout(Duration::from_secs(5), async {
        env.client.start().await?;
        println!("✅ DialogClient started successfully");
        
        env.client.stop().await?;
        println!("✅ DialogClient stopped successfully");
        
        Ok::<(), Box<dyn std::error::Error>>(())
    }).await??;
    
    env.shutdown().await;
    Ok(())
}

/// Test basic dialog operations with REAL transport
#[tokio::test]
async fn test_basic_dialog_operations_real_transport() -> Result<(), Box<dyn std::error::Error>> {
    let env = DialogApiTestEnvironment::new().await?;
    
    println!("🚀 Testing basic dialog operations with REAL UDP transport");
    
    env.server.start().await?;
    env.client.start().await?;
    
    // Test create outgoing dialog
    let local_uri: Uri = "sip:alice@example.com".parse()?;
    let remote_uri: Uri = format!("sip:bob@{}", env.server_addr).parse()?;
    
    // Create dialog through server API
    let dialog_id = env.server.create_outgoing_dialog(local_uri.clone(), remote_uri.clone(), None).await?;
    println!("✅ Created dialog: {}", dialog_id);
    
    // Test dialog operations
    let active_dialogs = env.server.list_active_dialogs().await;
    assert!(active_dialogs.contains(&dialog_id), "Created dialog should be in active list");
    println!("✅ Dialog found in active list");
    
    // Test dialog information retrieval
    let dialog_info = env.server.get_dialog_info(&dialog_id).await?;
    assert_eq!(dialog_info.local_uri, local_uri);
    assert_eq!(dialog_info.remote_uri, remote_uri);
    println!("✅ Dialog info retrieved correctly");
    
    let dialog_state = env.server.get_dialog_state(&dialog_id).await?;
    println!("✅ Dialog state: {:?}", dialog_state);
    
    // Test dialog termination
    env.server.terminate_dialog(&dialog_id).await?;
    println!("✅ Dialog terminated successfully");
    
    env.server.stop().await?;
    env.client.stop().await?;
    
    env.shutdown().await;
    Ok(())
}

/// Test client dialog operations with REAL transport
#[tokio::test]
async fn test_dialog_client_operations_real_transport() -> Result<(), Box<dyn std::error::Error>> {
    let env = DialogApiTestEnvironment::new().await?;
    
    println!("🚀 Testing client dialog operations with REAL UDP transport");
    
    env.client.start().await?;
    
    // Test create outgoing dialog
    let local_uri: Uri = format!("sip:alice@{}", env.client_addr).parse()?;
    let remote_uri: Uri = format!("sip:bob@{}", env.server_addr).parse()?;
    
    let dialog = env.client.create_dialog(&local_uri.to_string(), &remote_uri.to_string()).await?;
    let dialog_id = dialog.id().clone();
    println!("✅ Created client dialog: {}", dialog_id);
    
    // Test dialog operations
    let active_dialogs = env.client.list_active_dialogs().await;
    assert!(active_dialogs.contains(&dialog_id), "Created dialog should be in active list");
    println!("✅ Client dialog found in active list");
    
    // Test dialog information retrieval
    let dialog_info = env.client.get_dialog_info(&dialog_id).await?;
    assert_eq!(dialog_info.local_uri, local_uri);
    assert_eq!(dialog_info.remote_uri, remote_uri);
    println!("✅ Client dialog info retrieved correctly");
    
    let dialog_state = env.client.get_dialog_state(&dialog_id).await?;
    println!("✅ Client dialog state: {:?}", dialog_state);
    
    // Test dialog termination
    env.client.terminate_dialog(&dialog_id).await?;
    println!("✅ Client dialog terminated successfully");
    
    env.client.stop().await?;
    
    env.shutdown().await;
    Ok(())
}

/// Test real call operations with REAL transport
#[tokio::test]
async fn test_call_operations_real_transport() -> Result<(), Box<dyn std::error::Error>> {
    let env = DialogApiTestEnvironment::new().await?;
    
    println!("🚀 Testing call operations with REAL UDP transport");
    
    env.server.start().await?;
    env.client.start().await?;
    
    // Create a proper INVITE request for real transport
    let invite_request = env.create_sip_request(
        Method::Invite, 
        &format!("sip:bob@{}", env.server_addr)
    );
    
    // Test handle incoming INVITE
    let call_handle = env.server.handle_invite(invite_request, env.client_addr).await?;
    let dialog_id = call_handle.dialog().id().clone();
    println!("✅ Created call handle for dialog: {}", dialog_id);
    
    // Test call operations
    let accept_result = env.server.accept_call(&dialog_id, Some("SDP answer".to_string())).await;
    println!("✅ Accept call result: {:?}", accept_result);
    
    let reject_result = env.server.reject_call(&dialog_id, StatusCode::BadRequest, Some("Request rejected".to_string())).await;
    println!("✅ Reject call result: {:?}", reject_result);
    
    let terminate_result = env.server.terminate_call(&dialog_id).await;
    println!("✅ Terminate call result: {:?}", terminate_result);
    
    env.server.stop().await?;
    env.client.stop().await?;
    
    env.shutdown().await;
    Ok(())
}

/// Test API error handling with REAL transport
#[tokio::test]
async fn test_api_error_handling_real_transport() -> Result<(), Box<dyn std::error::Error>> {
    let env = DialogApiTestEnvironment::new().await?;
    
    println!("🚀 Testing API error handling with REAL UDP transport");
    
    env.server.start().await?;
    
    // Test operations with invalid dialog ID
    let invalid_dialog_id = DialogId::new();
    
    // These should return errors gracefully
    let info_result = env.server.get_dialog_info(&invalid_dialog_id).await;
    assert!(info_result.is_err(), "Should return error for invalid dialog ID");
    println!("✅ Invalid dialog ID properly rejected");
    
    let state_result = env.server.get_dialog_state(&invalid_dialog_id).await;
    assert!(state_result.is_err(), "Should return error for invalid dialog ID");
    println!("✅ Invalid dialog state query properly rejected");
    
    env.server.stop().await?;
    
    env.shutdown().await;
    Ok(())
}

/// Test concurrent API operations with REAL transport
#[tokio::test]
async fn test_concurrent_api_operations_real_transport() -> Result<(), Box<dyn std::error::Error>> {
    let env = DialogApiTestEnvironment::new().await?;
    
    println!("🚀 Testing concurrent operations with REAL UDP transport");
    
    env.server.start().await?;
    
    // Create multiple dialogs concurrently
    let mut handles = Vec::new();
    
    for i in 0..5 {  // Test with 5 concurrent dialogs
        let server_clone = Arc::clone(&env.server);
        let server_addr = env.server_addr;
        let handle = tokio::spawn(async move {
            let local_uri: Uri = format!("sip:user{}@example.com", i).parse().unwrap();
            let remote_uri: Uri = format!("sip:peer{}@{}", i, server_addr).parse().unwrap();
            
            server_clone.create_outgoing_dialog(local_uri, remote_uri, None).await
        });
        handles.push(handle);
    }
    
    // Wait for all dialogs to be created
    let mut dialog_ids = Vec::new();
    for handle in handles {
        match handle.await? {
            Ok(dialog_id) => {
                dialog_ids.push(dialog_id);
                println!("✅ Created concurrent dialog: {}", dialog_ids.last().unwrap());
            }
            Err(e) => println!("Warning: Failed to create dialog: {:?}", e),
        }
    }
    
    println!("✅ Successfully created {} concurrent dialogs", dialog_ids.len());
    
    // Cleanup all dialogs
    for dialog_id in dialog_ids {
        if let Err(e) = env.server.terminate_dialog(&dialog_id).await {
            println!("Warning: Failed to terminate dialog: {:?}", e);
        }
    }
    
    env.server.stop().await?;
    
    env.shutdown().await;
    Ok(())
}

/// Test DialogManager access through API with REAL transport
#[tokio::test]
async fn test_dialog_manager_access_real_transport() -> Result<(), Box<dyn std::error::Error>> {
    let env = DialogApiTestEnvironment::new().await?;
    
    println!("🚀 Testing DialogManager access with REAL UDP transport");
    
    // Test access to underlying dialog manager
    let _dialog_manager = env.server.dialog_manager();
    println!("✅ Successfully accessed underlying DialogManager");
    
    // This verifies that the API provides access to the dialog manager
    // for advanced use cases (as documented in our API design)
    
    env.shutdown().await;
    Ok(())
} 