//! Unit tests for MediaSessionController
//!
//! This module contains all unit tests for the controller functionality.

#[cfg(test)]
mod tests {
    use super::super::*;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::collections::HashMap;
    use crate::types::DialogId;
    
    #[tokio::test]
    async fn test_start_stop_session() {
        let controller = MediaSessionController::new();
        
        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: None,
            parameters: HashMap::new(),
        };
        
        // Start session
        let result = controller.start_media(DialogId::new("dialog1"), config).await;
        assert!(result.is_ok());
        
        // Check session exists
        let session_info = controller.get_session_info(&DialogId::new("dialog1")).await;
        assert!(session_info.is_some());
        
        // Stop session
        let result = controller.stop_media(&DialogId::new("dialog1")).await;
        assert!(result.is_ok());
        
        // Check session is removed
        let session_info = controller.get_session_info(&DialogId::new("dialog1")).await;
        assert!(session_info.is_none());
    }
    
    #[tokio::test]
    async fn test_create_relay() {
        let controller = MediaSessionController::new();
        
        let config_a = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: None,
            parameters: HashMap::new(),
        };
        
        let config_b = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: None,
            parameters: HashMap::new(),
        };
        
        // Start both sessions
        controller.start_media(DialogId::new("dialog1"), config_a).await.unwrap();
        controller.start_media(DialogId::new("dialog2"), config_b).await.unwrap();
        
        // Create relay should succeed but not actually create relay since no MediaRelay is configured
        let result = controller.create_relay("dialog1".to_string(), "dialog2".to_string()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_dynamic_port_allocation() {
        println!("🧪 Testing dynamic port allocation integration");
        
        let controller = MediaSessionController::new();
        
        // Create multiple sessions to verify different ports are allocated
        let mut session_infos = Vec::new();
        
        for i in 0..3 {
            let dialog_id = format!("test_dialog_{}", i);
            let config = MediaConfig {
                local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
                remote_addr: None,
                preferred_codec: None,
                parameters: HashMap::new(),
            };
            
            println!("📞 Creating session: {}", dialog_id);
            controller.start_media(DialogId::new(dialog_id.clone()), config).await
                .expect("Failed to start media session");
            
            let session_info = controller.get_session_info(&DialogId::new(dialog_id)).await
                .expect("Session should exist");
            
            println!("✅ Session created with port: {:?}", session_info.rtp_port);
            assert!(session_info.rtp_port.is_some(), "Port should be allocated");
            
            session_infos.push(session_info);
        }
        
        // Verify different ports were allocated
        let mut ports = Vec::new();
        for session_info in &session_infos {
            if let Some(port) = session_info.rtp_port {
                ports.push(port);
            }
        }
        
        // Remove duplicates and check that we have unique ports
        ports.sort();
        ports.dedup();
        assert_eq!(ports.len(), 3, "All sessions should have unique ports");
        
        println!("🎯 Allocated ports: {:?}", ports);
        
        // Verify all ports are in valid range (no privileged ports)
        for &port in &ports {
            assert!(port >= 1024, "Port should be >= 1024 (non-privileged)");
            assert!(port <= 65535, "Port should be <= 65535 (valid range)");
        }
        
        println!("✅ All ports are in valid range and unique");
        
        // Clean up sessions
        for i in 0..3 {
            let dialog_id = format!("test_dialog_{}", i);
            controller.stop_media(&DialogId::new(dialog_id)).await
                .expect("Failed to stop media session");
        }
        
        println!("✨ Dynamic port allocation test completed successfully!");
        println!("🔧 rtp-core's PortAllocator is providing conflict-free dynamic allocation");
    }
} 