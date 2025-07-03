//! Tests for server-oriented session management API
//!
//! These tests verify:
//! - ServerConfig creation and defaults
//! - Transport protocol selection
//! - Server-specific functionality
//! - Integration with transaction manager

use rvoip_session_core::api::*;
use std::sync::Arc;
use std::time::Duration;
use std::net::SocketAddr;

// Import server types from the re-exported location in mod.rs
use rvoip_session_core::api::{ServerConfig, TransportProtocol};

#[test]
fn test_server_config_default_values() {
    let config = ServerConfig::default();
    
    assert_eq!(config.bind_address, "0.0.0.0:5060".parse::<SocketAddr>().unwrap());
    assert_eq!(config.transport_protocol, TransportProtocol::Udp);
    assert_eq!(config.max_sessions, 1000);
    assert_eq!(config.session_timeout, Duration::from_secs(3600));
    assert_eq!(config.transaction_timeout, Duration::from_secs(32));
    assert!(config.enable_media);
    assert_eq!(config.server_name, "RVOIP-Server/1.0");
    assert_eq!(config.contact_uri, None);
}

#[test]
fn test_server_config_custom_values() {
    let config = ServerConfig {
        bind_address: "192.168.1.100:5061".parse().unwrap(),
        transport_protocol: TransportProtocol::Tcp,
        max_sessions: 5000,
        session_timeout: Duration::from_secs(7200),
        transaction_timeout: Duration::from_secs(60),
        enable_media: false,
        server_name: "CustomServer/2.0".to_string(),
        contact_uri: Some("sip:server@example.com".to_string()),
    };
    
    assert_eq!(config.bind_address.port(), 5061);
    assert_eq!(config.transport_protocol, TransportProtocol::Tcp);
    assert_eq!(config.max_sessions, 5000);
    assert_eq!(config.session_timeout, Duration::from_secs(7200));
    assert!(!config.enable_media);
    assert_eq!(config.server_name, "CustomServer/2.0");
    assert_eq!(config.contact_uri, Some("sip:server@example.com".to_string()));
}

#[test]
fn test_transport_protocol_variants() {
    // Test all transport protocol variants
    let protocols = vec![
        TransportProtocol::Udp,
        TransportProtocol::Tcp,
        TransportProtocol::Tls,
        TransportProtocol::Ws,
        TransportProtocol::Wss,
    ];
    
    for protocol in protocols {
        let config = ServerConfig {
            transport_protocol: protocol,
            ..Default::default()
        };
        
        assert_eq!(config.transport_protocol, protocol);
    }
}

#[test]
fn test_transport_protocol_equality() {
    assert_eq!(TransportProtocol::Udp, TransportProtocol::Udp);
    assert_ne!(TransportProtocol::Udp, TransportProtocol::Tcp);
    assert_ne!(TransportProtocol::Tcp, TransportProtocol::Tls);
    assert_ne!(TransportProtocol::Ws, TransportProtocol::Wss);
}

#[test]
fn test_server_config_clone() {
    let config = ServerConfig {
        bind_address: "10.0.0.1:5060".parse().unwrap(),
        transport_protocol: TransportProtocol::Tls,
        max_sessions: 2000,
        session_timeout: Duration::from_secs(1800),
        transaction_timeout: Duration::from_secs(16),
        enable_media: true,
        server_name: "ClonedServer/1.0".to_string(),
        contact_uri: Some("sip:clone@example.com".to_string()),
    };
    
    let cloned = config.clone();
    
    assert_eq!(cloned.bind_address, config.bind_address);
    assert_eq!(cloned.transport_protocol, config.transport_protocol);
    assert_eq!(cloned.max_sessions, config.max_sessions);
    assert_eq!(cloned.session_timeout, config.session_timeout);
    assert_eq!(cloned.transaction_timeout, config.transaction_timeout);
    assert_eq!(cloned.enable_media, config.enable_media);
    assert_eq!(cloned.server_name, config.server_name);
    assert_eq!(cloned.contact_uri, config.contact_uri);
}

#[test]
fn test_server_config_debug_format() {
    let config = ServerConfig::default();
    let debug_str = format!("{:?}", config);
    
    // Verify debug output contains key fields
    assert!(debug_str.contains("bind_address"));
    assert!(debug_str.contains("transport_protocol"));
    assert!(debug_str.contains("max_sessions"));
    assert!(debug_str.contains("server_name"));
}

#[test]
fn test_transport_protocol_debug_format() {
    let protocols = vec![
        (TransportProtocol::Udp, "Udp"),
        (TransportProtocol::Tcp, "Tcp"),
        (TransportProtocol::Tls, "Tls"),
        (TransportProtocol::Ws, "Ws"),
        (TransportProtocol::Wss, "Wss"),
    ];
    
    for (protocol, expected) in protocols {
        let debug_str = format!("{:?}", protocol);
        assert_eq!(debug_str, expected);
    }
}

#[test]
fn test_server_config_ipv6_address() {
    let config = ServerConfig {
        bind_address: "[::1]:5060".parse().unwrap(),
        ..Default::default()
    };
    
    assert!(config.bind_address.is_ipv6());
    assert_eq!(config.bind_address.port(), 5060);
}

#[test]
fn test_server_config_timeout_boundaries() {
    // Test minimum timeouts
    let min_config = ServerConfig {
        session_timeout: Duration::from_secs(1),
        transaction_timeout: Duration::from_millis(100),
        ..Default::default()
    };
    
    assert_eq!(min_config.session_timeout, Duration::from_secs(1));
    assert_eq!(min_config.transaction_timeout, Duration::from_millis(100));
    
    // Test maximum reasonable timeouts
    let max_config = ServerConfig {
        session_timeout: Duration::from_secs(86400), // 24 hours
        transaction_timeout: Duration::from_secs(300), // 5 minutes
        ..Default::default()
    };
    
    assert_eq!(max_config.session_timeout, Duration::from_secs(86400));
    assert_eq!(max_config.transaction_timeout, Duration::from_secs(300));
}

#[test]
fn test_server_config_with_different_ports() {
    let configs = vec![
        ("0.0.0.0:5060", 5060),
        ("0.0.0.0:5061", 5061),
        ("127.0.0.1:8080", 8080),
        ("[::]:5060", 5060),
    ];
    
    for (addr_str, expected_port) in configs {
        let config = ServerConfig {
            bind_address: addr_str.parse().unwrap(),
            ..Default::default()
        };
        
        assert_eq!(config.bind_address.port(), expected_port);
    }
}

#[tokio::test]
async fn test_create_full_server_manager_placeholder() {
    // Note: This test is limited because create_full_server_manager
    // requires a real TransactionManager, which is complex to mock
    
    // Test that the function exists and has the right signature
    // In a real implementation, this would create a proper server manager
    
    // For now, just verify the types compile correctly
    let _config = ServerConfig::default();
    
    // The actual function would be called like:
    // let tm = Arc::new(TransactionManager::new(...));
    // let server = create_full_server_manager(tm, config).await?;
    
    // This ensures the API is properly defined even if implementation is pending
}

#[test]
fn test_server_config_server_name_variations() {
    let names = vec![
        "SimpleServer/1.0",
        "RVOIP-PBX/2.5.1",
        "CustomSIPServer/3.0-beta",
        "MyServer",
    ];
    
    for name in names {
        let config = ServerConfig {
            server_name: name.to_string(),
            ..Default::default()
        };
        
        assert_eq!(config.server_name, name);
    }
}

#[test]
fn test_server_config_contact_uri_variations() {
    let uris = vec![
        Some("sip:server@example.com"),
        Some("sip:pbx@192.168.1.100:5060"),
        Some("sips:secure@example.com"),
        None,
    ];
    
    for uri in uris {
        let config = ServerConfig {
            contact_uri: uri.map(|s| s.to_string()),
            ..Default::default()
        };
        
        assert_eq!(config.contact_uri, uri.map(|s| s.to_string()));
    }
}

#[test]
fn test_server_config_media_enabled_disabled() {
    // Test with media enabled
    let enabled_config = ServerConfig {
        enable_media: true,
        ..Default::default()
    };
    assert!(enabled_config.enable_media);
    
    // Test with media disabled
    let disabled_config = ServerConfig {
        enable_media: false,
        ..Default::default()
    };
    assert!(!disabled_config.enable_media);
}

#[test]
fn test_server_config_max_sessions_limits() {
    let limits = vec![
        1,      // Minimum reasonable
        100,    // Small server
        1000,   // Default
        10000,  // Large server
        100000, // Very large server
    ];
    
    for limit in limits {
        let config = ServerConfig {
            max_sessions: limit,
            ..Default::default()
        };
        
        assert_eq!(config.max_sessions, limit);
    }
}

// Integration test placeholder for future implementation
#[cfg(test)]
mod integration_tests {
    use super::*;
    
    #[tokio::test]
    #[ignore] // Ignored until server implementation is complete
    async fn test_server_with_real_transaction_manager() {
        // This test would create a real server with transaction manager
        // Currently ignored as it requires full implementation
        
        // Example of what it would look like:
        /*
        let config = ServerConfig {
            bind_address: "127.0.0.1:0".parse().unwrap(), // Random port
            ..Default::default()
        };
        
        let tm = create_test_transaction_manager().await;
        let server = create_full_server_manager(tm, config).await.unwrap();
        
        // Test server operations
        let bridges = server.list_bridges();
        assert_eq!(bridges.len(), 0);
        
        // Create sessions and test bridging
        let session1 = server.create_outgoing_session().await.unwrap();
        let session2 = server.create_outgoing_session().await.unwrap();
        
        let bridge_id = server.bridge_sessions(&session1, &session2).await.unwrap();
        assert_eq!(server.list_bridges().len(), 1);
        
        server.destroy_bridge(&bridge_id).await.unwrap();
        assert_eq!(server.list_bridges().len(), 0);
        */
    }
} 