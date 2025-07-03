//! Integration tests for the server transport implementation
//!
//! These tests verify that the refactored code works correctly as a whole.

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use crate::api::server::transport::MediaTransportServer;
    use crate::api::server::config::ServerConfig;
    use crate::api::server::transport::DefaultMediaTransportServer;
    use crate::api::common::extension::ExtensionFormat;
    
    #[tokio::test]
    async fn test_server_lifecycle() {
        // Create a server config with default values
        let config = ServerConfig {
            local_address: "127.0.0.1:0".parse().unwrap(), // Use port 0 to get a random port
            rtcp_mux: true,
            header_extensions_enabled: true,
            header_extension_format: ExtensionFormat::OneByte,
            csrc_management_enabled: true,
            enable_jitter_buffer: true,
            jitter_buffer_size: 50,
            jitter_max_packet_age_ms: 200,
            default_payload_type: 8,
            clock_rate: 8000,
            security_config: Default::default(),
            buffer_limits: Default::default(),
            high_performance_buffers_enabled: false,
            max_clients: 100,
            media_sync_enabled: Some(false),
            ssrc_demultiplexing_enabled: Some(false),
            transmit_buffer_config: Default::default(),
        };
        
        // Create a new server
        let server = DefaultMediaTransportServer::new(config).await.unwrap();
        
        // Start the server
        assert!(server.start().await.is_ok(), "Failed to start server");
        
        // Get the local address
        let addr = server.get_local_address().await.unwrap();
        println!("Server bound to {}", addr);
        
        // Get server stats (should be empty)
        let stats = server.get_stats().await.unwrap();
        assert_eq!(stats.streams.len(), 0, "New server should have no streams");
        
        // Enable CSRC management
        assert!(server.enable_csrc_management().await.unwrap(), "Failed to enable CSRC management");
        assert!(server.is_csrc_management_enabled().await.unwrap(), "CSRC management should be enabled");
        
        // Enable SSRC demultiplexing
        assert!(server.enable_ssrc_demultiplexing().await.unwrap(), "Failed to enable SSRC demultiplexing");
        assert!(server.is_ssrc_demultiplexing_enabled().await.unwrap(), "SSRC demultiplexing should be enabled");
        
        // Enable header extensions
        assert!(server.enable_header_extensions(ExtensionFormat::OneByte).await.unwrap(), "Failed to enable header extensions");
        assert!(server.is_header_extensions_enabled().await.unwrap(), "Header extensions should be enabled");
        
        // Configure a header extension
        server.configure_header_extension(1, "urn:ietf:params:rtp-hdrext:ssrc-audio-level".to_string()).await.unwrap();
        
        // Stop the server
        assert!(server.stop().await.is_ok(), "Failed to stop server");
        
        // Sleep briefly to ensure resources are released
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
} 