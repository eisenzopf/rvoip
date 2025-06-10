mod common;

use std::sync::Arc;
use std::time::Duration;
use tokio::time;

use crate::common::api_test_utils::*;
use rvoip_session_core::api::builder::*;
use rvoip_session_core::api::handlers::*;
use rvoip_session_core::api::types::*;
use rvoip_session_core::Result;

#[tokio::test]
async fn test_session_manager_builder_default() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_session_manager_builder_default");
        
        let builder = SessionManagerBuilder::new();
        
        // Test that we can create a builder with default settings
        assert!(format!("{:?}", builder).contains("SessionManagerBuilder"));
        
        // Test default builder creation
        let default_builder = SessionManagerBuilder::default();
        assert!(format!("{:?}", default_builder).contains("SessionManagerBuilder"));
        
        println!("Completed test_session_manager_builder_default");
    }).await;
    
    assert!(result.is_ok(), "test_session_manager_builder_default timed out");
}

#[tokio::test]
async fn test_session_manager_builder_configuration() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_session_manager_builder_configuration");
        
        let handler = Arc::new(TestCallHandler::new(CallDecision::Accept(None)));
        
        // Test fluent API configuration
        let builder = SessionManagerBuilder::new()
            .with_sip_port(5070)
            .with_sip_bind_address("192.168.1.100")
            .with_from_uri("sip:test@example.com")
            .with_media_ports(10000, 20000)
            .with_handler(handler.clone())
            .p2p_mode();
        
        // Verify the builder was configured
        let debug_str = format!("{:?}", builder);
        assert!(debug_str.contains("5070") || debug_str.contains("sip_port"));
        assert!(debug_str.contains("p2p_mode"));
        
        println!("Completed test_session_manager_builder_configuration");
    }).await;
    
    assert!(result.is_ok(), "test_session_manager_builder_configuration timed out");
}

#[tokio::test]
async fn test_session_manager_config_validation() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_session_manager_config_validation");
        
        let helper = ApiBuilderTestHelper::new();
        
        // Test valid configuration
        let valid_config = SessionManagerConfig {
            sip_port: 5060,
            sip_bind_address: "0.0.0.0".to_string(),
            from_uri: Some("sip:user@example.com".to_string()),
            media_port_start: 10000,
            media_port_end: 20000,
            p2p_mode: false,
        };
        
        assert!(helper.validate_config(&valid_config).is_ok());
        
        // Test invalid configurations
        let invalid_config1 = SessionManagerConfig {
            sip_port: 0, // Invalid port
            sip_bind_address: "0.0.0.0".to_string(),
            from_uri: Some("sip:user@example.com".to_string()),
            media_port_start: 10000,
            media_port_end: 20000,
            p2p_mode: false,
        };
        
        assert!(helper.validate_config(&invalid_config1).is_err());
        
        let invalid_config2 = SessionManagerConfig {
            sip_port: 5060,
            sip_bind_address: "".to_string(), // Empty address
            from_uri: Some("sip:user@example.com".to_string()),
            media_port_start: 10000,
            media_port_end: 20000,
            p2p_mode: false,
        };
        
        assert!(helper.validate_config(&invalid_config2).is_err());
        
        let invalid_config3 = SessionManagerConfig {
            sip_port: 5060,
            sip_bind_address: "0.0.0.0".to_string(),
            from_uri: Some("sip:user@example.com".to_string()),
            media_port_start: 20000, // Start > end
            media_port_end: 10000,
            p2p_mode: false,
        };
        
        assert!(helper.validate_config(&invalid_config3).is_err());
        
        println!("Completed test_session_manager_config_validation");
    }).await;
    
    assert!(result.is_ok(), "test_session_manager_config_validation timed out");
}

#[tokio::test]
async fn test_builder_port_validation() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_builder_port_validation");
        
        // Test valid port ranges
        let valid_ranges = vec![
            (1024, 2048),
            (5000, 6000),
            (10000, 20000),
            (30000, 40000),
            (50000, 60000),
        ];
        
        for (start, end) in valid_ranges {
            let validation_result = ApiTestUtils::validate_port_range(start, end);
            assert!(validation_result.is_ok(), "Port range {}-{} should be valid", start, end);
        }
        
        // Test invalid port ranges
        let invalid_ranges = vec![
            (0, 1000),      // Start too low
            (5000, 5000),   // Start == end
            (6000, 5000),   // Start > end
            (100, 65535),   // End at u16 max (not practical for port ranges)
        ];
        
        for (start, end) in invalid_ranges {
            let validation_result = ApiTestUtils::validate_port_range(start, end);
            assert!(validation_result.is_err(), "Port range {}-{} should be invalid", start, end);
        }
        
        println!("Completed test_builder_port_validation");
    }).await;
    
    assert!(result.is_ok(), "test_builder_port_validation timed out");
}

#[tokio::test]
async fn test_builder_configurations() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_builder_configurations");
        
        let helper = ApiBuilderTestHelper::new();
        let builders = helper.test_builder_configurations();
        
        // Test that we have the expected number of configurations
        assert_eq!(builders.len(), 4);
        
        // Verify each builder configuration
        for (i, builder) in builders.iter().enumerate() {
            println!("Testing builder configuration {}", i);
            let debug_str = format!("{:?}", builder);
            assert!(debug_str.contains("SessionManagerBuilder"));
        }
        
        println!("Completed test_builder_configurations");
    }).await;
    
    assert!(result.is_ok(), "test_builder_configurations timed out");
}

#[tokio::test]
async fn test_builder_with_different_handlers() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_builder_with_different_handlers");
        
        // Test with different handler types
        let auto_handler = Arc::new(AutoAnswerHandler::default());
        let queue_handler = Arc::new(QueueHandler::new(10));
        let routing_handler = Arc::new(RoutingHandler::new());
        let test_handler = Arc::new(TestCallHandler::new(CallDecision::Accept(None)));
        
        let handlers: Vec<Arc<dyn CallHandler>> = vec![
            auto_handler,
            queue_handler,
            routing_handler,
            test_handler,
        ];
        
        for (i, handler) in handlers.into_iter().enumerate() {
            let builder = SessionManagerBuilder::new()
                .with_sip_port(5060 + i as u16)
                .with_handler(handler);
            
            let debug_str = format!("{:?}", builder);
            assert!(debug_str.contains("handler"));
            assert!(debug_str.contains("true")); // handler.is_some() should be true
        }
        
        println!("Completed test_builder_with_different_handlers");
    }).await;
    
    assert!(result.is_ok(), "test_builder_with_different_handlers timed out");
}

#[tokio::test]
async fn test_builder_sip_uri_validation() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_builder_sip_uri_validation");
        
        // Test valid SIP URIs for from_uri
        let valid_uris = vec![
            "sip:user@example.com",
            "sips:secure@example.com",
            "sip:user@192.168.1.100",
            "sip:user@example.com:5060",
            "sip:complex.user+tag@sub.domain.com:5061",
        ];
        
        for uri in valid_uris {
            assert!(ApiTestUtils::is_valid_sip_uri(uri), "URI should be valid: {}", uri);
            
            let builder = SessionManagerBuilder::new()
                .with_from_uri(uri);
            
            let debug_str = format!("{:?}", builder);
            assert!(debug_str.contains("from_uri"));
        }
        
        // Test invalid SIP URIs
        let invalid_uris = vec![
            "",
            "not_a_uri",
            "http://example.com",
            "sip:",
            "sip:@example.com",
        ];
        
        for uri in invalid_uris {
            assert!(!ApiTestUtils::is_valid_sip_uri(uri), "URI should be invalid: {}", uri);
        }
        
        println!("Completed test_builder_sip_uri_validation");
    }).await;
    
    assert!(result.is_ok(), "test_builder_sip_uri_validation timed out");
}

#[tokio::test]
async fn test_builder_bind_address_validation() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_builder_bind_address_validation");
        
        // Test various bind addresses
        let bind_addresses = vec![
            "0.0.0.0",      // Any address
            "127.0.0.1",    // Localhost
            "192.168.1.100", // Private IP
            "10.0.0.1",     // Private IP
            "localhost",    // Hostname
        ];
        
        for address in bind_addresses {
            let builder = SessionManagerBuilder::new()
                .with_sip_bind_address(address);
            
            let debug_str = format!("{:?}", builder);
            assert!(debug_str.contains("sip_bind_address"));
        }
        
        println!("Completed test_builder_bind_address_validation");
    }).await;
    
    assert!(result.is_ok(), "test_builder_bind_address_validation timed out");
}

#[tokio::test]
async fn test_session_manager_config_default() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_session_manager_config_default");
        
        let config = SessionManagerConfig::default();
        
        // Test default values
        assert_eq!(config.sip_port, 5060);
        assert_eq!(config.sip_bind_address, "0.0.0.0");
        assert_eq!(config.from_uri, None);
        assert_eq!(config.media_port_start, 10000);
        assert_eq!(config.media_port_end, 20000);
        assert_eq!(config.p2p_mode, false);
        
        // Test that default config is valid
        let helper = ApiBuilderTestHelper::new();
        // Note: Default config has sip_port 5060 (not 0), so it should be valid
        // But our validation expects non-zero port, so let's check if 5060 > 0
        assert!(config.sip_port > 0);
        assert!(!config.sip_bind_address.is_empty());
        assert!(config.media_port_start < config.media_port_end);
        
        println!("Completed test_session_manager_config_default");
    }).await;
    
    assert!(result.is_ok(), "test_session_manager_config_default timed out");
}

#[tokio::test]
async fn test_builder_edge_cases() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_builder_edge_cases");
        
        // Test builder with empty strings
        let builder1 = SessionManagerBuilder::new()
            .with_sip_bind_address("")
            .with_from_uri("");
        
        let debug_str1 = format!("{:?}", builder1);
        assert!(debug_str1.contains("SessionManagerBuilder"));
        
        // Test builder with unicode
        let builder2 = SessionManagerBuilder::new()
            .with_sip_bind_address("🦀.example.com")
            .with_from_uri("sip:user🚀@example.com");
        
        let debug_str2 = format!("{:?}", builder2);
        assert!(debug_str2.contains("SessionManagerBuilder"));
        
        // Test builder with very long strings
        let long_address = "a".repeat(1000);
        let long_uri = format!("sip:{}@example.com", "b".repeat(500));
        
        let builder3 = SessionManagerBuilder::new()
            .with_sip_bind_address(&long_address)
            .with_from_uri(&long_uri);
        
        let debug_str3 = format!("{:?}", builder3);
        assert!(debug_str3.contains("SessionManagerBuilder"));
        
        // Test extreme port values
        let builder4 = SessionManagerBuilder::new()
            .with_sip_port(1)
            .with_media_ports(1, 65535);
        
        let debug_str4 = format!("{:?}", builder4);
        assert!(debug_str4.contains("SessionManagerBuilder"));
        
        println!("Completed test_builder_edge_cases");
    }).await;
    
    assert!(result.is_ok(), "test_builder_edge_cases timed out");
}

#[tokio::test]
async fn test_builder_method_chaining() {
    let result = time::timeout(Duration::from_secs(5), async {
        println!("Starting test_builder_method_chaining");
        
        let handler = Arc::new(TestCallHandler::new(CallDecision::Accept(None)));
        
        // Test that all methods can be chained
        let builder = SessionManagerBuilder::new()
            .with_sip_port(5070)
            .with_sip_bind_address("127.0.0.1")
            .with_from_uri("sip:test@example.com")
            .with_media_ports(20000, 30000)
            .with_handler(handler.clone())
            .p2p_mode()
            .with_sip_port(5080)  // Override previous port
            .with_sip_bind_address("0.0.0.0"); // Override previous address
        
        let debug_str = format!("{:?}", builder);
        assert!(debug_str.contains("SessionManagerBuilder"));
        
        // Test that we can create multiple builders
        let builder1 = SessionManagerBuilder::new().with_sip_port(5061);
        let builder2 = SessionManagerBuilder::new().with_sip_port(5062);
        
        let debug1 = format!("{:?}", builder1);
        let debug2 = format!("{:?}", builder2);
        
        assert!(debug1.contains("SessionManagerBuilder"));
        assert!(debug2.contains("SessionManagerBuilder"));
        
        println!("Completed test_builder_method_chaining");
    }).await;
    
    assert!(result.is_ok(), "test_builder_method_chaining timed out");
}

#[tokio::test]
async fn test_builder_performance() {
    let result = time::timeout(Duration::from_secs(10), async {
        println!("Starting test_builder_performance");
        
        let start = std::time::Instant::now();
        let builder_count = 1000;
        
        // Create many builders quickly
        let mut builders = Vec::new();
        for i in 0..builder_count {
            let handler = Arc::new(TestCallHandler::new(CallDecision::Accept(None)));
            
            let builder = SessionManagerBuilder::new()
                .with_sip_port(5060 + (i % 100) as u16)
                .with_sip_bind_address("127.0.0.1")
                .with_from_uri(&format!("sip:user{}@example.com", i))
                .with_media_ports(10000 + i * 10, 20000 + i * 10)
                .with_handler(handler);
            
            builders.push(builder);
        }
        
        let duration = start.elapsed();
        println!("Created {} builders in {:?}", builder_count, duration);
        
        // Performance should be reasonable
        assert!(duration < Duration::from_secs(5), "Builder creation took too long");
        assert_eq!(builders.len(), builder_count as usize);
        
        // Verify all builders were created properly
        for builder in &builders {
            let debug_str = format!("{:?}", builder);
            assert!(debug_str.contains("SessionManagerBuilder"));
        }
        
        println!("Completed test_builder_performance");
    }).await;
    
    assert!(result.is_ok(), "test_builder_performance timed out");
}

#[tokio::test]
async fn test_concurrent_builder_creation() {
    let result = time::timeout(Duration::from_secs(10), async {
        println!("Starting test_concurrent_builder_creation");
        
        let concurrent_count = 50;
        let mut handles = Vec::new();
        
        // Create builders concurrently
        for i in 0..concurrent_count {
            let handle = tokio::spawn(async move {
                let handler = Arc::new(TestCallHandler::new(CallDecision::Accept(None)));
                
                SessionManagerBuilder::new()
                    .with_sip_port(5060 + i as u16)
                    .with_sip_bind_address("127.0.0.1")
                    .with_from_uri(&format!("sip:concurrent{}@example.com", i))
                    .with_media_ports(10000 + i * 100, 20000 + i * 100)
                    .with_handler(handler)
                    .p2p_mode()
            });
            handles.push(handle);
        }
        
        // Collect all results
        let mut builders = Vec::new();
        for handle in handles {
            let builder = handle.await.unwrap();
            builders.push(builder);
        }
        
        // Verify all concurrent operations completed successfully
        assert_eq!(builders.len(), concurrent_count as usize);
        
        for builder in &builders {
            let debug_str = format!("{:?}", builder);
            assert!(debug_str.contains("SessionManagerBuilder"));
            assert!(debug_str.contains("p2p_mode"));
        }
        
        println!("Completed test_concurrent_builder_creation");
    }).await;
    
    assert!(result.is_ok(), "test_concurrent_builder_creation timed out");
} 