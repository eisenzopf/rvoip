//! Example showing usage of RTCP APP, BYE, and XR packet APIs
//!
//! This example demonstrates sending and receiving RTCP Application-Defined (APP),
//! Goodbye (BYE), and Extended Report (XR) packets between client and server
//! media transport.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time;
use tracing::{info, debug, warn};

use rvoip_rtp_core::api::{
    client::{
        transport::{MediaTransportClient, VoipMetrics},
        config::{ClientConfig, ClientConfigBuilder},
    },
    server::{
        transport::MediaTransportServer,
        config::{ServerConfig, ServerConfigBuilder},
    },
    common::{
        frame::MediaFrame,
        frame::MediaFrameType,
        events::MediaTransportEvent,
    },
};

// Events we want to wait for
#[derive(Default)]
struct ReceivedEvents {
    app_received: bool,
    bye_received: bool,
    xr_received: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    
    info!("Starting RTCP APP/BYE/XR example");
    
    // Create a timeout for the entire example
    let example_timeout = tokio::time::timeout(
        Duration::from_secs(15),  // 15 second timeout
        async {
            // Configure server
            let server_config = ServerConfigBuilder::new()
                .local_address("127.0.0.1:0".parse().unwrap())
                .rtcp_mux(true)
                .security_config(rvoip_rtp_core::api::server::security::ServerSecurityConfig { security_mode: rvoip_rtp_core::api::common::config::SecurityMode::None, ..Default::default() })
                .build()
                .expect("Failed to build server config");
            
            // Create server
            let server = rvoip_rtp_core::api::server::transport::server_transport_impl::DefaultMediaTransportServer::new(server_config).await?;
            
            // Start server
            server.start().await?;
            
            // Get the server's bound address
            let server_addr = server.get_local_address().await?;
            info!("Server bound to {}", server_addr);
            
            // Configure client
            let client_config = ClientConfigBuilder::new()
                .remote_address(server_addr)
                .rtcp_mux(true)
                .security_config(rvoip_rtp_core::api::client::security::ClientSecurityConfig { security_mode: rvoip_rtp_core::api::common::config::SecurityMode::None, ..Default::default() })
                .build();
            
            // Create client
            let client = rvoip_rtp_core::api::client::transport::client_transport_impl::DefaultMediaTransportClient::new(client_config).await?;
            
            // Create a shared state to track received events
            let received_events = Arc::new(Mutex::new(ReceivedEvents::default()));
            
            // Register event callbacks
            let server_events = Arc::clone(&received_events);
            server.on_event(Box::new(move |event| {
                match event {
                    MediaTransportEvent::RtcpAppReceived { ssrc, name, data } => {
                        info!("Server received RTCP APP packet: ssrc={:08x}, name={}, data_len={}",
                              ssrc, name, data.len());
                        if let Ok(text) = std::str::from_utf8(&data) {
                            info!("APP data: {}", text);
                        }
                        let mut events = server_events.blocking_lock();
                        events.app_received = true;
                    },
                    MediaTransportEvent::RtcpByeReceived { ssrc, reason } => {
                        info!("Server received RTCP BYE packet: ssrc={:08x}, reason={:?}", 
                              ssrc, reason);
                        let mut events = server_events.blocking_lock();
                        events.bye_received = true;
                    },
                    MediaTransportEvent::RtcpXrVoipMetrics { metrics } => {
                        info!("Server received RTCP XR VoIP metrics: ssrc={:08x}, loss_rate={}%, R-factor={}, MOS-LQ={}, MOS-CQ={}",
                              metrics.ssrc, metrics.loss_rate, metrics.r_factor, metrics.mos_lq, metrics.mos_cq);
                        let mut events = server_events.blocking_lock();
                        events.xr_received = true;
                    },
                    _ => {}
                }
            })).await?;
            
            let client_events = Arc::clone(&received_events);
            client.on_event(Box::new(move |event| {
                match event {
                    MediaTransportEvent::RtcpAppReceived { ssrc, name, data } => {
                        info!("Client received RTCP APP packet: ssrc={:08x}, name={}, data_len={}",
                              ssrc, name, data.len());
                        if let Ok(text) = std::str::from_utf8(&data) {
                            info!("APP data: {}", text);
                        }
                        let mut events = client_events.blocking_lock();
                        events.app_received = true;
                    },
                    MediaTransportEvent::RtcpByeReceived { ssrc, reason } => {
                        info!("Client received RTCP BYE packet: ssrc={:08x}, reason={:?}", 
                              ssrc, reason);
                        let mut events = client_events.blocking_lock();
                        events.bye_received = true;
                    },
                    MediaTransportEvent::RtcpXrVoipMetrics { metrics } => {
                        info!("Client received RTCP XR VoIP metrics: ssrc={:08x}, loss_rate={}%, R-factor={}, MOS-LQ={}, MOS-CQ={}",
                              metrics.ssrc, metrics.loss_rate, metrics.r_factor, metrics.mos_lq, metrics.mos_cq);
                        let mut events = client_events.blocking_lock();
                        events.xr_received = true;
                    },
                    _ => {}
                }
            })).await?;
            
            // Connect client to server
            info!("Connecting client to server");
            client.connect().await?;
            
            // Exchange some media packets to establish the session
            info!("Exchanging media packets");
            for i in 0..5 {
                // Create a simple frame
                let frame = MediaFrame {
                    frame_type: MediaFrameType::Audio,
                    data: vec![1, 2, 3, 4, 5, 6, 7, 8],
                    timestamp: i * 160, // 20ms of 8kHz audio
                    sequence: i as u16,
                    marker: i == 0, // First packet has marker bit
                    payload_type: 8, // PCMA
                    ssrc: 0, // Will be set by the transport
                    csrcs: Vec::new(),
                };
                
                // Send frame from client to server
                client.send_frame(frame.clone()).await?;
                
                // Wait a bit to allow server to process
                time::sleep(Duration::from_millis(10)).await;
                
                // Send response from server to client (to first client in list)
                let clients = server.get_clients().await?;
                if !clients.is_empty() {
                    server.send_frame_to(&clients[0].id, frame).await?;
                }
            }
            
            // Wait a bit for RTP transmission to stabilize
            time::sleep(Duration::from_millis(50)).await;
            
            // ----- Send RTCP APP packet from client to server -----
            
            info!("Sending RTCP APP packet from client to server");
            let app_data = "This is application-specific test data".as_bytes().to_vec();
            client.send_rtcp_app("TEST", app_data).await?;
            
            // Wait for the packet to be processed
            time::sleep(Duration::from_millis(50)).await;
            
            // ----- Send RTCP XR packet with VoIP metrics from client to server -----
            
            info!("Sending RTCP XR VoIP metrics from client to server");
            
            // Create sample VoIP metrics
            let mut metrics = VoipMetrics {
                ssrc: 0x12345678, // This will be replaced by actual SSRC
                loss_rate: 5,       // 5% packet loss
                discard_rate: 2,    // 2% discard rate
                burst_density: 10,  // 10% of lost packets are in bursts
                gap_density: 3,     // 3% of packets in gaps are lost
                burst_duration: 120, // 120ms average burst duration
                gap_duration: 5000,  // 5000ms average gap duration
                round_trip_delay: 150, // 150ms round-trip delay
                end_system_delay: 40,  // 40ms end system delay
                signal_level: 30,    // -30 dBm signal level
                noise_level: 70,     // -70 dBm noise level
                rerl: 25,           // 25 dB residual echo return loss
                r_factor: 80,       // 80 R-factor (good quality)
                mos_lq: 40,         // 4.0 MOS-LQ
                mos_cq: 37,         // 3.7 MOS-CQ
                jb_nominal: 60,     // 60ms nominal jitter buffer
                jb_maximum: 120,    // 120ms maximum jitter buffer
                jb_abs_max: 150,    // 150ms absolute maximum jitter buffer
            };
            
            client.send_rtcp_xr_voip_metrics(metrics).await?;
            
            // Wait for the packet to be processed
            time::sleep(Duration::from_millis(50)).await;
            
            // ----- Send RTCP APP packet from server to client -----
            
            info!("Sending RTCP APP packet from server to client");
            let server_app_data = "Server application data response".as_bytes().to_vec();
            
            let clients = server.get_clients().await?;
            if !clients.is_empty() {
                server.send_rtcp_app_to_client(&clients[0].id, "SVRT", server_app_data).await?;
            } else {
                warn!("No clients connected, cannot send APP packet");
            }
            
            // Wait for the packet to be processed
            time::sleep(Duration::from_millis(50)).await;
            
            // ----- Send RTCP XR packet with VoIP metrics from server to client -----
            
            info!("Sending RTCP XR VoIP metrics from server to client");
            
            // Create VoIP metrics for the server
            let server_metrics = VoipMetrics {
                ssrc: 0x87654321, // This will be replaced by actual SSRC
                loss_rate: 3,      // 3% packet loss
                discard_rate: 1,   // 1% discard rate
                burst_density: 8,  // 8% of lost packets are in bursts
                gap_density: 2,    // 2% of packets in gaps are lost
                burst_duration: 100, // 100ms average burst duration
                gap_duration: 6000, // 6000ms average gap duration
                round_trip_delay: 160, // 160ms round-trip delay
                end_system_delay: 50,  // 50ms end system delay
                signal_level: 25,    // -25 dBm signal level
                noise_level: 75,     // -75 dBm noise level
                rerl: 30,           // 30 dB residual echo return loss
                r_factor: 85,       // 85 R-factor (good quality)
                mos_lq: 42,         // 4.2 MOS-LQ
                mos_cq: 40,         // 4.0 MOS-CQ
                jb_nominal: 50,     // 50ms nominal jitter buffer
                jb_maximum: 100,    // 100ms maximum jitter buffer
                jb_abs_max: 120,    // 120ms absolute maximum jitter buffer
            };
            
            if !clients.is_empty() {
                server.send_rtcp_xr_voip_metrics_to_client(&clients[0].id, server_metrics).await?;
            } else {
                warn!("No clients connected, cannot send XR packet");
            }
            
            // Wait for the packet to be processed
            time::sleep(Duration::from_millis(50)).await;
            
            // ----- Send RTCP BYE packet from client to server -----
            
            info!("Sending RTCP BYE packet from client to server");
            client.send_rtcp_bye(Some("Example completed".to_string())).await?;
            
            // Wait for the packet to be processed and for events to be received
            time::sleep(Duration::from_millis(50)).await;
            
            // ----- Check if all packets were received -----
            
            let events = received_events.lock().await;
            
            info!("Event reception status:");
            info!("  APP packets received: {}", events.app_received);
            info!("  BYE packets received: {}", events.bye_received);
            info!("  XR packets received: {}", events.xr_received);
            
            // Disconnect client
            client.disconnect().await?;
            
            // Stop server
            server.stop().await?;
            
            // Short delay before returning to ensure everything is cleaned up
            time::sleep(Duration::from_millis(100)).await;
            
            Ok(()) as Result<(), Box<dyn std::error::Error>>
        }
    );
    
    // Handle the timeout result
    match example_timeout.await {
        Ok(result) => {
            info!("Example completed successfully within time limit");
            result
        },
        Err(_) => {
            // Timeout occurred
            info!("Example timed out after 15 seconds - forcing termination");
            Ok(())
        }
    }
} 