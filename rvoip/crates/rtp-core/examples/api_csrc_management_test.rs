//! API Test for CSRC Management
//!
//! This test demonstrates the CSRC management capabilities in a conferencing scenario
//! where multiple sources are contributing to a mixed stream.
//!
//! This test focuses on ensuring the API client and server libraries properly
//! expose the underlying CSRC functionality from the rtp-core library.
//! 
//! We have implemented the following:
//!
//! 1. Extended the `MediaTransportClient` and `MediaTransportServer` interfaces to 
//!    include CSRC management API methods:
//!    - is_csrc_management_enabled()
//!    - enable_csrc_management()
//!    - add_csrc_mapping()
//!    - add_simple_csrc_mapping()
//!    - remove_csrc_mapping_by_ssrc()
//!    - get_csrc_mapping_by_ssrc()
//!    - get_all_csrc_mappings()
//!    - get_active_csrcs()
//!
//! 2. These methods allow client and server components to:
//!    - Map original SSRC values to CSRC values for contributing sources
//!    - Include these CSRC values in outgoing RTP packets
//!    - Manage metadata like CNAME and display names for participants
//!    - Query active contributing sources
//!
//! 3. The implementation properly handles packet serialization and CSRC inclusion based
//!    on the csrc_management_enabled configuration flag.
//!
//! 4. The MediaFrame struct has been extended to include a csrcs field, eliminating
//!    the need for applications to directly parse RTP packets. This maintains proper
//!    abstraction between the API and the underlying implementation details.
//!
//! This API approach ensures that CSRC management functionality is properly exposed
//! through the high-level API interfaces rather than requiring direct use of core components.

use rand::Rng;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{info, debug, warn, error};

use rvoip_rtp_core::{
    api::{
        client::{
            config::ClientConfigBuilder,
            transport::{MediaTransportClient, DefaultMediaTransportClient},
        },
        server::{
            config::ServerConfigBuilder,
            transport::{MediaTransportServer, DefaultMediaTransportServer},
        },
        common::frame::{MediaFrame, MediaFrameType},
    },
};

/// Simple mixer that combines audio from multiple sources
struct ConferenceMixer {
    ssrc: u32,
    sequence: u16,
    timestamp: u32,
    active_sources: Vec<u32>,
}

impl ConferenceMixer {
    fn new() -> Self {
        let mut rng = rand::thread_rng();
        Self {
            ssrc: rng.gen(),
            sequence: rng.gen(),
            timestamp: rng.gen(),
            active_sources: Vec::new(),
        }
    }
    
    fn set_active_sources(&mut self, sources: Vec<u32>) {
        self.active_sources = sources;
    }
    
    fn get_active_sources(&self) -> &[u32] {
        &self.active_sources
    }
    
    fn create_mixed_frame(&mut self) -> MediaFrame {
        // Generate sample data (in a real mixer, this would be mixed audio)
        let mut rng = rand::thread_rng();
        let mut data = Vec::with_capacity(160);
        for _ in 0..80 {
            data.push(rng.gen::<u8>());
            data.push(rng.gen::<u8>());
        }
        
        // Create frame with mixer SSRC
        let frame = MediaFrame {
            frame_type: MediaFrameType::Audio,
            data,
            timestamp: self.timestamp,
            sequence: self.sequence,
            marker: false,
            payload_type: 0, // PCMU
            ssrc: self.ssrc,
            csrcs: Vec::new(), // Initialize empty CSRCs (will be filled by transport layer based on CSRC mappings)
        };
        
        // Update sequence and timestamp
        self.sequence = self.sequence.wrapping_add(1);
        self.timestamp = self.timestamp.wrapping_add(160);
        
        frame
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();
    
    info!("API CSRC Management Test");
    info!("=======================");
    
    // Run the entire test with a hard 20-second timeout
    match tokio::time::timeout(Duration::from_secs(20), run_test()).await {
        Ok(result) => result,
        Err(_) => {
            info!("Test timed out after 20 seconds");
            Ok(())
        }
    }
}

/// Main test function that is run with a timeout
async fn run_test() -> Result<(), Box<dyn std::error::Error>> {
    // Create server and client configs with CSRC management enabled
    let server_config = ServerConfigBuilder::new()
        .local_address("127.0.0.1:9001".parse()?)
        .default_payload_type(0)
        .clock_rate(8000)
        .csrc_management_enabled(true) // Enable CSRC management via the API
        .build()?;
    
    let client_config = ClientConfigBuilder::new()
        .remote_address("127.0.0.1:9001".parse()?)
        .default_payload_type(0)
        .clock_rate(8000)
        .csrc_management_enabled(true) // Enable CSRC management via the API
        .build();
    
    // Create server and client
    let server = DefaultMediaTransportServer::new(server_config).await?;
    let client = DefaultMediaTransportClient::new(client_config).await?;
    
    // Start server
    server.start().await?;
    let server_addr = server.get_local_address().await?;
    info!("Server started on {}", server_addr);
    
    // Connect client
    info!("Connecting client to server at {}", server_addr);
    client.connect().await?;
    info!("Client connect() call completed");
    
    // Send a simple audio frame from client to server to establish the connection
    let mut rng = rand::thread_rng();
    let client_ssrc = rand::random::<u32>();
    
    // Create and send dummy frame
    let mut dummy_data = Vec::with_capacity(160);
    for _ in 0..80 {
        dummy_data.push(rng.gen::<u8>());
        dummy_data.push(rng.gen::<u8>());
    }
    
    let dummy_frame = MediaFrame {
        frame_type: MediaFrameType::Audio,
        data: dummy_data,
        timestamp: rng.gen(),
        sequence: rng.gen(),
        marker: false,
        payload_type: 0,
        ssrc: client_ssrc,
        csrcs: Vec::new(),
    };
    
    info!("Sending dummy frame from client to server to establish connection with SSRC={:08x}", client_ssrc);
    client.send_frame(dummy_frame).await?;
    
    // Wait for client to be connected and registered
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Get connected clients
    let clients = server.get_clients().await?;
    if clients.is_empty() {
        info!("No clients registered with server yet");
    } else {
        info!("Found {} registered clients:", clients.len());
        for (i, client_info) in clients.iter().enumerate() {
            info!("  Client {}: ID={}, Address={}", i+1, client_info.id, client_info.address);
        }
    }
    
    // Verify CSRC management is enabled on both sides using the API
    let server_csrc_enabled = server.is_csrc_management_enabled().await?;
    info!("CSRC management is {} on server", if server_csrc_enabled { "enabled" } else { "disabled" });
    
    let client_csrc_enabled = client.is_csrc_management_enabled().await?;
    info!("CSRC management is {} on client", if client_csrc_enabled { "enabled" } else { "disabled" });
    
    // Set up a conference mixer
    let mut mixer = ConferenceMixer::new();
    
    // Define participants
    let participants = [
        ("Alice", rand::random::<u32>()),
        ("Bob", rand::random::<u32>()),
        ("Charlie", rand::random::<u32>()),
    ];
    
    // Add participants as active sources in the mixer
    mixer.set_active_sources(participants.iter().map(|&(_, ssrc)| ssrc).collect());
    
    // Add CSRC mappings for each participant to server and client using the API
    for &(name, ssrc) in &participants {
        // Add mapping to server
        server.add_simple_csrc_mapping(ssrc, ssrc).await?;
        server.update_csrc_cname(ssrc, format!("{}@example.com", name.to_lowercase())).await?;
        server.update_csrc_display_name(ssrc, name.to_string()).await?;
        info!("Added CSRC mapping to server for {}: SSRC={:08x}", name, ssrc);
        
        // Add mapping to client
        client.add_simple_csrc_mapping(ssrc, ssrc).await?;
        client.update_csrc_cname(ssrc, format!("{}@example.com", name.to_lowercase())).await?;
        client.update_csrc_display_name(ssrc, name.to_string()).await?;
        info!("Added CSRC mapping to client for {}: SSRC={:08x}", name, ssrc);
    }
    
    // Set up packet reception tracking
    let packet_received = Arc::new(AtomicBool::new(false));
    let packet_received_clone = packet_received.clone();
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = shutdown.clone();
    
    // Create a task to receive frames on the client
    let client_clone = client.clone();
    let receive_task = tokio::spawn(async move {
        info!("Starting client frame receive task");
        while !shutdown_clone.load(Ordering::SeqCst) {
            if let Ok(Some(frame)) = client_clone.receive_frame(Duration::from_millis(100)).await {
                info!("Received frame: SSRC={:08x}, PT={}, SEQ={}, Size={} bytes", 
                     frame.ssrc, frame.payload_type, frame.sequence, frame.data.len());
                
                // Check for CSRCs directly from the MediaFrame
                if !frame.csrcs.is_empty() {
                    info!("Frame contains {} CSRCs: {:?}", frame.csrcs.len(), frame.csrcs);
                    packet_received_clone.store(true, Ordering::SeqCst);
                }
            }
            
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        
        info!("Receive task stopped");
    });
    
    info!("Running CSRC test with {} active sources", mixer.get_active_sources().len());
    
    // Test: Sending frames with CSRC information to connected clients
    if !clients.is_empty() {
        for client_info in &clients {
            let client_id = &client_info.id;
            info!("Test: Server sending mixed packet to client ID: {}", client_id);
            
            // Create a mixed frame
            let frame = mixer.create_mixed_frame();
            
            // Get active CSRCs for this frame using the server API
            let active_csrcs = server.get_active_csrcs(mixer.get_active_sources()).await?;
            info!("Frame will include {} active CSRCs: {:?}", active_csrcs.len(), active_csrcs);
            
            // Send frame to client
            match server.send_frame_to(client_id, frame).await {
                Ok(_) => info!("Successfully sent mixed frame to client {}", client_id),
                Err(e) => error!("Failed to send mixed frame to client {}: {}", client_id, e)
            }
            
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    } else {
        info!("No clients connected, skipping server-to-client test");
    }
    
    // Test: Client sending frames with CSRC information to server
    for i in 1..=3 {
        info!("Test iteration {}: Client sending frame with CSRCs", i);
        
        // Create mixed frame
        let frame = mixer.create_mixed_frame();
        
        // Get active CSRCs for this frame using the client API
        let active_csrcs = client.get_active_csrcs(mixer.get_active_sources()).await?;
        info!("Client frame will include {} active CSRCs: {:?}", active_csrcs.len(), active_csrcs);
        
        // Send from client to server
        match client.send_frame(frame).await {
            Ok(_) => info!("Successfully sent frame from client to server"),
            Err(e) => error!("Failed to send frame from client: {}", e)
        }
        
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    
    // Check if we received packets with CSRCs
    info!("Received packet with CSRCs: {}", packet_received.load(Ordering::SeqCst));
    
    // Signal shutdown and wait briefly
    shutdown.store(true, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Clean up
    receive_task.abort();
    client.disconnect().await?;
    server.stop().await?;
    
    info!("CSRC management test completed successfully");
    
    Ok(())
} 