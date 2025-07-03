use std::time::Duration;
use serial_test::serial;

// Test for the Ultra-Simple SIP Server example
#[tokio::test]
#[serial]
async fn test_simple_sip_server_works() {
    // Add timeout to prevent hanging
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        
        use rvoip::session_core::prelude::*;
        
        async fn simple_sip_server() -> Result<()> {
            let _session_manager = SessionManagerBuilder::new()
                .with_sip_port(5060)
                .build()
                .await?;
            
            println!("✅ SIP server running on port 5060");
            
            // In a real application, you would do:
            // tokio::signal::ctrl_c().await?;
            
            Ok(())
        }
        
        // Test that the function works
        let result = simple_sip_server().await;
        assert!(result.is_ok());
        
        // Wait a moment before next test
        tokio::time::sleep(Duration::from_millis(500)).await;
    }).await;
    
    assert!(result.is_ok(), "Test timed out");
}

// Test for the Simple SIP Client example
#[tokio::test]
#[serial]
async fn test_simple_sip_client_works() {
    // Add timeout to prevent hanging
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        
        use rvoip::client_core::{ClientConfig, ClientManager, MediaConfig};
        
        async fn simple_sip_client() -> Result<(), Box<dyn std::error::Error>> {
            let config = ClientConfig::new()
                .with_sip_addr("127.0.0.1:5060".parse()?)
                .with_media_addr("127.0.0.1:20000".parse()?)
                .with_user_agent("MyApp/1.0".to_string())
                .with_media(MediaConfig {
                    preferred_codecs: vec!["PCMU".to_string(), "PCMA".to_string()],
                    ..Default::default()
                });
            
            let client = ClientManager::new(config).await?;
            client.start().await?;
            
            println!("📞 SIP client ready to make calls");
            
            // In a real application, you would make a call:
            // let call_id = client.make_call(
            //     "sip:alice@127.0.0.1".to_string(),
            //     "sip:bob@example.com".to_string(),
            //     None
            // ).await?;
            
            client.stop().await?;
            Ok(())
        }
        
        // Test that the function works
        let result = simple_sip_client().await;
        assert!(result.is_ok());
        
        // Wait a moment before next test
        tokio::time::sleep(Duration::from_millis(500)).await;
    }).await;
    
    assert!(result.is_ok(), "Test timed out");
}

// Test for the Call Center Setup example  
#[tokio::test]
#[serial]
async fn test_call_center_setup_works() {
    // Add timeout to prevent hanging
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        
        use rvoip::call_engine::{prelude::*, CallCenterServerBuilder};
        
        async fn call_center_setup() -> std::result::Result<(), Box<dyn std::error::Error>> {
            let mut config = CallCenterConfig::default();
            config.general.local_signaling_addr = "0.0.0.0:5060".parse()?;
            config.general.domain = "127.0.0.1".to_string();
            
            let mut server = CallCenterServerBuilder::new()
                .with_config(config)
                .with_database_path(":memory:".to_string())
                .build()
                .await?;
            
            server.start().await?;
            
            // Give the server a moment to fully initialize
            tokio::time::sleep(Duration::from_millis(500)).await;
            
            server.create_default_queues().await?;
            
            println!("🏢 Call Center Server started successfully!");
            
            // In a real application, you would run indefinitely:
            // server.run().await?;
            
            server.stop().await?;
            
            // Give the server a moment to fully shut down
            tokio::time::sleep(Duration::from_millis(500)).await;
            Ok(())
        }
        
        // Test that the function works
        let result = call_center_setup().await;
        assert!(result.is_ok());
        
        // Wait longer after call center test to ensure full cleanup
        tokio::time::sleep(Duration::from_millis(1000)).await;
    }).await;
    
    assert!(result.is_ok(), "Test timed out");
}

// Integration test that verifies the basic imports work
#[test]
#[serial]
fn test_imports_compile() {
    // Test that all the imports from the examples compile by actually referencing them
    use rvoip::session_core::prelude::SessionManagerBuilder;
    use rvoip::client_core::{ClientConfig, ClientManager, MediaConfig};
    use rvoip::call_engine::{prelude::CallCenterConfig, CallCenterServerBuilder};
    
    // Verify the types exist by checking their type names
    let _sm = std::any::type_name::<SessionManagerBuilder>();
    let _cc = std::any::type_name::<ClientConfig>();
    let _cm = std::any::type_name::<ClientManager>();
    let _mc = std::any::type_name::<MediaConfig>();
    let _ccc = std::any::type_name::<CallCenterConfig>();
    let _csb = std::any::type_name::<CallCenterServerBuilder>();
    
    // If this test compiles, the imports are working
    assert!(true);
} 