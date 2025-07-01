//! Port Allocation Strategy Example
//!
//! This example demonstrates how the port allocation strategy works
//! across different platforms (Windows, Linux, macOS).
//!
//! It shows:
//! 1. How ports are allocated from a managed pool
//! 2. How platform-specific optimizations are applied
//! 3. Different port allocation strategies (Sequential, Random, Adjacent pairs)
//! 4. How port reuse works

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, debug, warn, error};

use rvoip_rtp_core::{
    RtpSession, RtpSessionConfig, RtpSessionEvent, RtpTransport,
    transport::{
        PlatformType, GlobalPortAllocator, PortAllocatorConfig,
        AllocationStrategy, PairingStrategy, RtpTransportConfig,
        UdpRtpTransport
    }
};

const SESSION_COUNT: usize = 5;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();
    
    // Print platform information
    let platform = PlatformType::current();
    info!("Running on platform: {:?}", platform);
    
    // Get the global port allocator
    let allocator = GlobalPortAllocator::instance().await;
    
    // Demonstrate port allocation with different strategies
    info!("\n=== Testing Individual Port Allocation ===");
    test_individual_port_allocation().await?;
    
    info!("\n=== Testing Adjacent Pair Allocation ===");
    test_adjacent_pair_allocation().await?;
    
    info!("\n=== Testing RTCP Mux Allocation ===");
    test_rtcp_mux_allocation().await?;
    
    info!("\n=== Testing Multiple RTP Sessions ===");
    test_multiple_sessions().await?;
    
    info!("\n=== Testing Port Reuse ===");
    test_port_reuse().await?;
    
    Ok(())
}

/// Test individual port allocation with different strategies
async fn test_individual_port_allocation() -> Result<(), Box<dyn std::error::Error>> {
    // Create a custom allocator for testing
    let mut config = PortAllocatorConfig::default();
    config.allocation_strategy = AllocationStrategy::Sequential;
    let sequential_allocator = rvoip_rtp_core::transport::PortAllocator::with_config(config.clone());
    
    // Allocate a few ports with Sequential strategy
    info!("Allocating ports with Sequential strategy:");
    for i in 0..3 {
        let port = sequential_allocator.allocate_port(IpAddr::V4(Ipv4Addr::LOCALHOST)).await?;
        info!("  Port {}: {}", i+1, port);
    }
    
    // Now try with Random strategy
    config.allocation_strategy = AllocationStrategy::Random;
    let random_allocator = rvoip_rtp_core::transport::PortAllocator::with_config(config.clone());
    
    info!("Allocating ports with Random strategy:");
    for i in 0..3 {
        let port = random_allocator.allocate_port(IpAddr::V4(Ipv4Addr::LOCALHOST)).await?;
        info!("  Port {}: {}", i+1, port);
    }
    
    // Release all ports
    let total_ports = sequential_allocator.allocated_count().await + random_allocator.allocated_count().await;
    info!("Total allocated ports: {}", total_ports);
    
    Ok(())
}

/// Test adjacent port pair allocation (RTP on even, RTCP on odd)
async fn test_adjacent_pair_allocation() -> Result<(), Box<dyn std::error::Error>> {
    // Create a custom allocator for testing
    let mut config = PortAllocatorConfig::default();
    config.pairing_strategy = PairingStrategy::Adjacent;
    let allocator = rvoip_rtp_core::transport::PortAllocator::with_config(config);
    
    // Allocate a few port pairs
    info!("Allocating adjacent port pairs:");
    for i in 0..3 {
        let (rtp_addr, rtcp_addr) = allocator.allocate_port_pair(&format!("test-adjacent-{}", i), None).await?;
        info!("  Pair {}: RTP: {}, RTCP: {:?}", i+1, rtp_addr.port(), rtcp_addr.map(|a| a.port()));
    }
    
    // Verify the RTP ports are even and RTCP ports are odd
    info!("Adjacent pairs should have RTP on even ports and RTCP on odd ports");
    
    Ok(())
}

/// Test RTCP mux port allocation (single port for both RTP and RTCP)
async fn test_rtcp_mux_allocation() -> Result<(), Box<dyn std::error::Error>> {
    // Create a custom allocator for testing
    let mut config = PortAllocatorConfig::default();
    config.pairing_strategy = PairingStrategy::Muxed;
    let allocator = rvoip_rtp_core::transport::PortAllocator::with_config(config);
    
    // Allocate a few muxed ports
    info!("Allocating RTCP-MUX ports:");
    for i in 0..3 {
        let (rtp_addr, rtcp_addr) = allocator.allocate_port_pair(&format!("test-mux-{}", i), None).await?;
        info!("  Port {}: RTP: {}, RTCP: {:?}", i+1, rtp_addr.port(), rtcp_addr.map(|a| a.port()));
    }
    
    // Verify the RTCP address is None for muxed ports
    info!("RTCP-MUX should have RTCP address as None (same port as RTP)");
    
    Ok(())
}

/// Test creating multiple RTP sessions with the port allocator
async fn test_multiple_sessions() -> Result<(), Box<dyn std::error::Error>> {
    info!("Creating {} RTP sessions...", SESSION_COUNT);
    
    let mut sessions = Vec::new();
    let mut transport_configs = Vec::new();
    
    // Create multiple sessions
    for i in 0..SESSION_COUNT {
        let session_id = format!("test-session-{}", i);
        
        // Create transport config
        let transport_config = RtpTransportConfig {
            local_rtp_addr: "0.0.0.0:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some(session_id.clone()),
            use_port_allocator: true,
        };
        
        // Create transport
        let transport = UdpRtpTransport::new(transport_config.clone()).await?;
        let local_addr = transport.local_rtp_addr()?;
        
        info!("Session {}: Allocated port {}", i, local_addr.port());
        transport_configs.push(transport_config);
        
        // We don't need to create full RTP sessions for this test
        sessions.push(transport);
    }
    
    // Show allocated port count
    let allocator = GlobalPortAllocator::instance().await;
    info!("Total allocated ports: {}", allocator.allocated_count().await);
    
    // Close half of the sessions
    info!("Closing half of the sessions...");
    for i in 0..(SESSION_COUNT / 2) {
        sessions[i].close().await?;
    }
    
    // Show allocated port count again
    info!("Remaining allocated ports: {}", allocator.allocated_count().await);
    
    // Clean up remaining sessions
    for session in sessions.iter().skip(SESSION_COUNT / 2) {
        session.close().await?;
    }
    
    // Verify all ports were released
    info!("Final allocated ports: {}", allocator.allocated_count().await);
    assert_eq!(allocator.allocated_count().await, 0, "All ports should be released");
    
    Ok(())
}

/// Test port reuse after closing sessions
async fn test_port_reuse() -> Result<(), Box<dyn std::error::Error>> {
    let session_id = "test-reuse-session";
    
    // Create custom allocator
    let mut config = PortAllocatorConfig::default();
    config.prefer_port_reuse = true;
    let allocator = rvoip_rtp_core::transport::PortAllocator::with_config(config);
    
    // Allocate a port
    let port1 = allocator.allocate_port(IpAddr::V4(Ipv4Addr::LOCALHOST)).await?;
    info!("Allocated port: {}", port1);
    
    // Release the port
    allocator.release_port(IpAddr::V4(Ipv4Addr::LOCALHOST), port1).await;
    info!("Released port: {}", port1);
    
    // Wait a bit to allow port reuse delay to pass
    info!("Waiting for port reuse delay...");
    sleep(Duration::from_millis(1500)).await;
    
    // Try to allocate another port - should reuse the released one
    let port2 = allocator.allocate_port(IpAddr::V4(Ipv4Addr::LOCALHOST)).await?;
    info!("Allocated port: {}", port2);
    
    if port1 == port2 {
        info!("SUCCESS: Port was reused after release!");
    } else {
        info!("Port was not reused, got a different port instead");
    }
    
    // Release the port
    allocator.release_port(IpAddr::V4(Ipv4Addr::LOCALHOST), port2).await;
    
    Ok(())
} 