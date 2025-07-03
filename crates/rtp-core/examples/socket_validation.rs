//! Socket Validation Example
//!
//! This example demonstrates the cross-platform socket validation functionality
//! which ensures RTP/RTCP sockets work correctly across different operating systems.

use std::time::Duration;
use tokio::time::sleep;
use tokio::net::UdpSocket;
use tracing::{info, debug, warn, error};
use rvoip_rtp_core::transport::{PlatformType, PlatformSocketStrategy, RtpSocketValidator};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for better debug output
    tracing_subscriber::fmt::init();
    
    // Detect the current platform
    let platform = PlatformType::current();
    info!("Running on platform: {:?}", platform);
    
    // Display the default socket strategy for this platform
    let default_strategy = PlatformSocketStrategy::for_current_platform();
    info!("Default socket strategy for this platform: {:#?}", default_strategy);
    
    // Platform-specific notes
    match platform {
        PlatformType::Windows => {
            info!("Windows socket notes:");
            info!("- Windows uses Winsock API rather than Berkeley sockets");
            info!("- SO_REUSEADDR behaves differently than on Unix systems");
            info!("- Windows has no direct equivalent to Unix's SO_REUSEPORT");
        },
        PlatformType::MacOS => {
            info!("macOS socket notes:");
            info!("- Typically requires both SO_REUSEADDR AND SO_REUSEPORT");
            info!("- Has more BSD-like socket behavior");
            info!("- IPV6_V6ONLY defaults to ON (IPv6 sockets don't handle IPv4)");
        },
        PlatformType::Linux => {
            info!("Linux socket notes:");
            info!("- Usually SO_REUSEADDR alone is sufficient");
            info!("- SO_REUSEPORT has different semantics (load balancing)");
            info!("- Older kernels default IPV6_V6ONLY to OFF");
        },
        PlatformType::Other => {
            info!("Running on an unsupported platform");
            info!("Will use generic socket settings");
        }
    }
    
    // Run socket validation to determine the optimal strategy
    info!("\nRunning socket validation tests...");
    
    let validation_result = RtpSocketValidator::validate().await;
    
    match &validation_result {
        Ok(ref strategy) => {
            info!("Socket validation succeeded!");
            info!("Recommended socket strategy: {:#?}", strategy);
            
            // Show differences from default
            if strategy.use_reuse_addr != default_strategy.use_reuse_addr {
                info!("Use SO_REUSEADDR changed from {} to {}", 
                      default_strategy.use_reuse_addr, strategy.use_reuse_addr);
            }
            
            if strategy.use_reuse_port != default_strategy.use_reuse_port {
                info!("Use SO_REUSEPORT changed from {} to {}", 
                      default_strategy.use_reuse_port, strategy.use_reuse_port);
            }
            
            if strategy.buffer_size != default_strategy.buffer_size {
                info!("Buffer size changed from {} to {} bytes", 
                      default_strategy.buffer_size, strategy.buffer_size);
            }
            
            if strategy.rebind_wait_time_ms != default_strategy.rebind_wait_time_ms {
                info!("Rebind wait time changed from {} to {} ms", 
                      default_strategy.rebind_wait_time_ms, strategy.rebind_wait_time_ms);
            }
        },
        Err(ref e) => {
            error!("Socket validation failed: {}", e);
            error!("Using default settings may result in issues on this platform");
        }
    }
    
    // Demonstrate using the platform-specific socket strategy
    info!("\nDemonstrating socket creation with platform-specific settings...");
    
    // Create two sockets using platform-specific settings
    let strategy = match &validation_result {
        Ok(s) => s.clone(),
        Err(_) => PlatformSocketStrategy::for_current_platform(),
    };
    
    // Create socket 1
    let socket1 = UdpSocket::bind("127.0.0.1:0").await?;
    strategy.apply_to_socket(&socket1).await?;
    let addr1 = socket1.local_addr()?;
    
    // Create socket 2
    let socket2 = UdpSocket::bind("127.0.0.1:0").await?;
    strategy.apply_to_socket(&socket2).await?;
    let addr2 = socket2.local_addr()?;
    
    info!("Successfully created two sockets with platform-specific settings:");
    info!("Socket 1: {}", addr1);
    info!("Socket 2: {}", addr2);
    
    // Show that we can send data between these sockets
    info!("\nSending test data between sockets...");
    
    let test_data = b"Cross-platform socket test";
    socket1.send_to(test_data, addr2).await?;
    
    let mut buf = vec![0u8; 1024];
    let (len, from) = socket2.recv_from(&mut buf).await?;
    
    info!("Socket 2 received {} bytes from {}", len, from);
    
    if &buf[..len] == test_data {
        info!("Data integrity check passed!");
    } else {
        error!("Data integrity check failed!");
    }
    
    // Send a response back
    let response = b"Response received";
    socket2.send_to(response, addr1).await?;
    
    let (len, from) = socket1.recv_from(&mut buf).await?;
    info!("Socket 1 received {} bytes from {}", len, from);
    
    if &buf[..len] == response {
        info!("Response data integrity check passed!");
    } else {
        error!("Response data integrity check failed!");
    }
    
    // Demonstrate socket rebinding
    info!("\nDemonstrating socket rebinding (requiring SO_REUSEADDR)...");
    
    // Close socket1 by dropping it
    drop(socket1);
    
    // Wait for the recommended time
    info!("Waiting {}ms before rebinding...", strategy.rebind_wait_time_ms);
    sleep(Duration::from_millis(strategy.rebind_wait_time_ms)).await;
    
    // Try to bind to the same port again
    let socket3 = UdpSocket::bind(addr1).await?;
    strategy.apply_to_socket(&socket3).await?;
    
    info!("Successfully rebound to port: {}", addr1.port());
    
    // Send data to socket 2 again
    let rebind_data = b"Socket rebind test successful";
    socket3.send_to(rebind_data, addr2).await?;
    
    let (len, from) = socket2.recv_from(&mut buf).await?;
    info!("Socket 2 received {} bytes from rebinded socket", len);
    
    if &buf[..len] == rebind_data {
        info!("Rebind data integrity check passed!");
    } else {
        error!("Rebind data integrity check failed!");
    }
    
    info!("\nCross-platform socket validation test completed successfully!");
    
    Ok(())
} 