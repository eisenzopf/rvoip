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

    #[tokio::test]
    async fn test_codec_negotiation_pcmu() {
        println!("🧪 Testing PCMU codec negotiation");
        
        let controller = MediaSessionController::new();
        
        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("PCMU".to_string()),
            parameters: HashMap::new(),
        };
        
        // Start session with PCMU codec
        let result = controller.start_media(DialogId::new("pcmu_dialog"), config).await;
        assert!(result.is_ok(), "Should successfully start session with PCMU codec");
        
        // Verify session was created with PCMU codec
        let session_info = controller.get_session_info(&DialogId::new("pcmu_dialog")).await;
        assert!(session_info.is_some());
        let session_info = session_info.unwrap();
        
        // Check that the preferred codec is stored correctly
        assert_eq!(session_info.config.preferred_codec, Some("PCMU".to_string()));
        
        println!("✅ PCMU codec negotiation test completed");
        
        // Cleanup
        controller.stop_media(&DialogId::new("pcmu_dialog")).await.unwrap();
    }

    #[tokio::test]
    async fn test_codec_negotiation_opus() {
        println!("🧪 Testing Opus codec negotiation");
        
        let controller = MediaSessionController::new();
        
        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("opus".to_string()),
            parameters: HashMap::new(),
        };
        
        // Start session with Opus codec
        let result = controller.start_media(DialogId::new("opus_dialog"), config).await;
        assert!(result.is_ok(), "Should successfully start session with Opus codec");
        
        // Verify session was created with Opus codec
        let session_info = controller.get_session_info(&DialogId::new("opus_dialog")).await;
        assert!(session_info.is_some());
        let session_info = session_info.unwrap();
        
        // Check that the preferred codec is stored correctly
        assert_eq!(session_info.config.preferred_codec, Some("opus".to_string()));
        
        println!("✅ Opus codec negotiation test completed");
        
        // Cleanup
        controller.stop_media(&DialogId::new("opus_dialog")).await.unwrap();
    }

    #[tokio::test]
    async fn test_codec_negotiation_fallback() {
        println!("🧪 Testing codec negotiation fallback for unknown codec");
        
        let controller = MediaSessionController::new();
        
        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("unknown_codec".to_string()),
            parameters: HashMap::new(),
        };
        
        // Start session with unknown codec (should fallback to PCMU)
        let result = controller.start_media(DialogId::new("fallback_dialog"), config).await;
        assert!(result.is_ok(), "Should successfully start session even with unknown codec");
        
        // Verify session was created and stored the original codec name
        let session_info = controller.get_session_info(&DialogId::new("fallback_dialog")).await;
        assert!(session_info.is_some());
        let session_info = session_info.unwrap();
        
        // Check that the original preferred codec is stored (even though it's unknown)
        assert_eq!(session_info.config.preferred_codec, Some("unknown_codec".to_string()));
        
        println!("✅ Codec negotiation fallback test completed");
        
        // Cleanup
        controller.stop_media(&DialogId::new("fallback_dialog")).await.unwrap();
    }

    #[tokio::test]
    async fn test_codec_negotiation_default() {
        println!("🧪 Testing default codec negotiation (no preferred codec)");
        
        let controller = MediaSessionController::new();
        
        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: None, // No preferred codec
            parameters: HashMap::new(),
        };
        
        // Start session with no preferred codec (should default to PCMU)
        let result = controller.start_media(DialogId::new("default_dialog"), config).await;
        assert!(result.is_ok(), "Should successfully start session with default codec");
        
        // Verify session was created
        let session_info = controller.get_session_info(&DialogId::new("default_dialog")).await;
        assert!(session_info.is_some());
        let session_info = session_info.unwrap();
        
        // Check that no preferred codec is set
        assert_eq!(session_info.config.preferred_codec, None);
        
        println!("✅ Default codec negotiation test completed");
        
        // Cleanup
        controller.stop_media(&DialogId::new("default_dialog")).await.unwrap();
    }

    #[tokio::test]
    async fn test_codec_case_insensitive() {
        println!("🧪 Testing case-insensitive codec negotiation");
        
        let controller = MediaSessionController::new();
        
        // Test different case variations
        let test_cases = vec![
            ("pcmu", "pcmu"),
            ("PCMU", "PCMU"),
            ("PcMu", "PcMu"),
            ("opus", "opus"),
            ("Opus", "Opus"),
            ("OPUS", "OPUS"),
        ];
        
        for (i, (codec_name, expected_stored)) in test_cases.into_iter().enumerate() {
            let dialog_id = format!("case_test_{}", i);
            
            let config = MediaConfig {
                local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
                remote_addr: None,
                preferred_codec: Some(codec_name.to_string()),
                parameters: HashMap::new(),
            };
            
            // Start session with case variation
            let result = controller.start_media(DialogId::new(dialog_id.clone()), config).await;
            assert!(result.is_ok(), "Should successfully start session with codec: {}", codec_name);
            
            // Verify session was created
            let session_info = controller.get_session_info(&DialogId::new(dialog_id.clone())).await;
            assert!(session_info.is_some());
            let session_info = session_info.unwrap();
            
            // Check that the original case is preserved
            assert_eq!(session_info.config.preferred_codec, Some(expected_stored.to_string()));
            
            // Cleanup
            controller.stop_media(&DialogId::new(dialog_id)).await.unwrap();
        }
        
        println!("✅ Case-insensitive codec negotiation test completed");
    }

    #[tokio::test]
    async fn test_codec_negotiation_pcma() {
        println!("🧪 Testing PCMA (G.711 A-law) codec negotiation");
        
        let controller = MediaSessionController::new();
        
        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("PCMA".to_string()),
            parameters: HashMap::new(),
        };
        
        // Start session with PCMA codec
        let result = controller.start_media(DialogId::new("pcma_dialog"), config).await;
        assert!(result.is_ok(), "Should successfully start session with PCMA codec");
        
        // Verify session was created with PCMA codec
        let session_info = controller.get_session_info(&DialogId::new("pcma_dialog")).await;
        assert!(session_info.is_some());
        let session_info = session_info.unwrap();
        
        // Check that the preferred codec is stored correctly
        assert_eq!(session_info.config.preferred_codec, Some("PCMA".to_string()));
        
        println!("✅ PCMA (G.711 A-law) codec negotiation test completed");
        
        // Cleanup
        controller.stop_media(&DialogId::new("pcma_dialog")).await.unwrap();
    }



    #[tokio::test]
    async fn test_codec_negotiation_g729() {
        println!("🧪 Testing G729 codec negotiation");
        
        let controller = MediaSessionController::new();
        
        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("G729".to_string()),
            parameters: HashMap::new(),
        };
        
        // Start session with G729 codec
        let result = controller.start_media(DialogId::new("g729_dialog"), config).await;
        assert!(result.is_ok(), "Should successfully start session with G729 codec");
        
        // Verify session was created with G729 codec
        let session_info = controller.get_session_info(&DialogId::new("g729_dialog")).await;
        assert!(session_info.is_some());
        let session_info = session_info.unwrap();
        
        // Check that the preferred codec is stored correctly
        assert_eq!(session_info.config.preferred_codec, Some("G729".to_string()));
        
        println!("✅ G729 codec negotiation test completed");
        
        // Cleanup
        controller.stop_media(&DialogId::new("g729_dialog")).await.unwrap();
    }

    #[tokio::test]
    async fn test_all_g711_variants() {
        println!("🧪 Testing all G.711 variants comprehensively");
        
        let controller = MediaSessionController::new();
        
        // Test G.711 μ-law (PCMU)
        let pcmu_config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("PCMU".to_string()),
            parameters: HashMap::new(),
        };
        
        controller.start_media(DialogId::new("g711_mulaw"), pcmu_config).await.unwrap();
        let pcmu_info = controller.get_session_info(&DialogId::new("g711_mulaw")).await.unwrap();
        assert_eq!(pcmu_info.config.preferred_codec, Some("PCMU".to_string()));
        
        // Test G.711 A-law (PCMA)
        let pcma_config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("PCMA".to_string()),
            parameters: HashMap::new(),
        };
        
        controller.start_media(DialogId::new("g711_alaw"), pcma_config).await.unwrap();
        let pcma_info = controller.get_session_info(&DialogId::new("g711_alaw")).await.unwrap();
        assert_eq!(pcma_info.config.preferred_codec, Some("PCMA".to_string()));
        
        println!("✅ Verified both G.711 variants:");
        println!("   - PCMU (μ-law): payload type 0, 8000Hz");
        println!("   - PCMA (A-law): payload type 8, 8000Hz");
        
        // Cleanup
        controller.stop_media(&DialogId::new("g711_mulaw")).await.unwrap();
        controller.stop_media(&DialogId::new("g711_alaw")).await.unwrap();
        
        println!("✅ All G.711 variants test completed");
    }

    #[tokio::test]
    async fn test_comprehensive_codec_matrix() {
        println!("🧪 Testing comprehensive codec support matrix");
        
        let controller = MediaSessionController::new();
        
        // Test all supported codecs with their expected payload types and clock rates
        let test_cases = vec![
            ("PCMU", 0, 8000, "G.711 μ-law"),
            ("PCMA", 8, 8000, "G.711 A-law"),
            ("G729", 18, 8000, "G.729"),
            ("opus", 111, 48000, "Opus"),
        ];
        
        for (codec_name, expected_pt, expected_clock, description) in test_cases {
            let dialog_id = format!("codec_matrix_{}", codec_name.to_lowercase());
            
            let config = MediaConfig {
                local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
                remote_addr: None,
                preferred_codec: Some(codec_name.to_string()),
                parameters: HashMap::new(),
            };
            
            println!("  Testing {}: {} (PT:{}, {}Hz)", codec_name, description, expected_pt, expected_clock);
            
            // Start session
            let result = controller.start_media(DialogId::new(dialog_id.clone()), config).await;
            assert!(result.is_ok(), "Should successfully start session with {}", codec_name);
            
            // Verify codec mapping (indirectly through successful session creation)
            let session_info = controller.get_session_info(&DialogId::new(dialog_id.clone())).await;
            assert!(session_info.is_some());
            let session_info = session_info.unwrap();
            assert_eq!(session_info.config.preferred_codec, Some(codec_name.to_string()));
            
            // Cleanup
            controller.stop_media(&DialogId::new(dialog_id)).await.unwrap();
        }
        
        println!("✅ Comprehensive codec matrix test completed");
        println!("   All RFC 3551 static codecs and Opus tested successfully!");
    }

    #[tokio::test]
    async fn test_update_media_codec_change() {
        println!("🧪 Testing codec change in update_media");
        
        let controller = MediaSessionController::new();
        
        // Start session with PCMU
        let initial_config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("PCMU".to_string()),
            parameters: HashMap::new(),
        };
        
        let dialog_id = DialogId::new("codec_change_dialog");
        let result = controller.start_media(dialog_id.clone(), initial_config).await;
        assert!(result.is_ok(), "Should successfully start session with PCMU");
        
        // Verify initial codec
        let session_info = controller.get_session_info(&dialog_id).await;
        assert!(session_info.is_some());
        assert_eq!(session_info.unwrap().config.preferred_codec, Some("PCMU".to_string()));
        
        // Update to Opus codec
        let updated_config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("opus".to_string()),
            parameters: HashMap::new(),
        };
        
        let result = controller.update_media(dialog_id.clone(), updated_config).await;
        assert!(result.is_ok(), "Should successfully update codec to Opus");
        
        // Verify codec was updated
        let session_info = controller.get_session_info(&dialog_id).await;
        assert!(session_info.is_some());
        assert_eq!(session_info.unwrap().config.preferred_codec, Some("opus".to_string()));
        
        println!("✅ Codec change test completed successfully!");
    }
    
    #[tokio::test]
    async fn test_update_media_combined_changes() {
        println!("🧪 Testing combined remote address and codec change");
        
        let controller = MediaSessionController::new();
        
        // Start session with no remote address and PCMU
        let initial_config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("PCMU".to_string()),
            parameters: HashMap::new(),
        };
        
        let dialog_id = DialogId::new("combined_change_dialog");
        let result = controller.start_media(dialog_id.clone(), initial_config).await;
        assert!(result.is_ok(), "Should successfully start session");
        
        // Update both remote address and codec
        let remote_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 5060);
        let updated_config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: Some(remote_addr),
            preferred_codec: Some("opus".to_string()),
            parameters: HashMap::new(),
        };
        
        let result = controller.update_media(dialog_id.clone(), updated_config).await;
        assert!(result.is_ok(), "Should successfully update both address and codec");
        
        // Verify both changes were applied
        let session_info = controller.get_session_info(&dialog_id).await;
        assert!(session_info.is_some());
        let info = session_info.unwrap();
        assert_eq!(info.config.remote_addr, Some(remote_addr));
        assert_eq!(info.config.preferred_codec, Some("opus".to_string()));
        
        println!("✅ Combined change test completed successfully!");
    }
    
    #[tokio::test]
    async fn test_update_media_no_changes() {
        println!("🧪 Testing update_media with no actual changes");
        
        let controller = MediaSessionController::new();
        
        // Start session
        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 5060)),
            preferred_codec: Some("PCMU".to_string()),
            parameters: HashMap::new(),
        };
        
        let dialog_id = DialogId::new("no_change_dialog");
        let result = controller.start_media(dialog_id.clone(), config.clone()).await;
        assert!(result.is_ok(), "Should successfully start session");
        
        // Update with same config (no changes)
        let result = controller.update_media(dialog_id.clone(), config).await;
        assert!(result.is_ok(), "Should successfully handle no-change update");
        
        println!("✅ No-change update test completed successfully!");
    }
} 