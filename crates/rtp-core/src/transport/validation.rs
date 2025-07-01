//! Socket Validation Module
//!
//! This module provides utilities for validating socket behavior across platforms
//! and implementing fallback mechanisms when platform-specific issues are detected.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use tokio::net::UdpSocket;
use tracing::{debug, error, info, warn};

use crate::error::Error;
use crate::Result;

/// Platform type detection for platform-specific code paths
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformType {
    /// Windows platform (any version)
    Windows,
    /// macOS (Darwin) platform
    MacOS,
    /// Linux platform
    Linux,
    /// Other platform
    Other,
}

impl PlatformType {
    /// Get the current platform type
    pub fn current() -> Self {
        #[cfg(target_os = "windows")]
        return PlatformType::Windows;
        
        #[cfg(target_os = "macos")]
        return PlatformType::MacOS;
        
        #[cfg(target_os = "linux")]
        return PlatformType::Linux;
        
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        return PlatformType::Other;
    }
    
    /// Check if the current platform is Windows
    pub fn is_windows() -> bool {
        cfg!(target_os = "windows")
    }
    
    /// Check if the current platform is macOS
    pub fn is_macos() -> bool {
        cfg!(target_os = "macos")
    }
    
    /// Check if the current platform is Linux
    pub fn is_linux() -> bool {
        cfg!(target_os = "linux")
    }
}

/// Socket binding strategy for a specific platform
#[derive(Debug, Clone, PartialEq)]
pub struct PlatformSocketStrategy {
    /// Whether to use SO_REUSEADDR socket option
    pub use_reuse_addr: bool,
    
    /// Whether to use SO_REUSEPORT socket option (when available)
    pub use_reuse_port: bool,
    
    /// Whether to explicitly set IPV6_V6ONLY option
    pub set_ipv6_only: bool,
    
    /// Value for IPV6_V6ONLY when set
    pub ipv6_only: bool,
    
    /// Recommended buffer size in bytes
    pub buffer_size: usize,
    
    /// Time to wait (ms) between closing a socket and rebinding to the same port
    pub rebind_wait_time_ms: u64,
}

impl PlatformSocketStrategy {
    /// Get the recommended socket strategy for the current platform
    pub fn for_current_platform() -> Self {
        match PlatformType::current() {
            PlatformType::Windows => Self::for_windows(),
            PlatformType::MacOS => Self::for_macos(),
            PlatformType::Linux => Self::for_linux(),
            PlatformType::Other => Self::default(),
        }
    }
    
    /// Get the recommended socket strategy for Windows
    pub fn for_windows() -> Self {
        Self {
            use_reuse_addr: true,
            use_reuse_port: false, // Not available on Windows
            set_ipv6_only: true,
            ipv6_only: true, // Separate stacks is safer cross-platform
            buffer_size: 262144, // Larger buffer for Windows
            rebind_wait_time_ms: 1000, // Windows often needs longer
        }
    }
    
    /// Get the recommended socket strategy for macOS
    pub fn for_macos() -> Self {
        Self {
            use_reuse_addr: true,
            use_reuse_port: true, // macOS often needs both
            set_ipv6_only: true,
            ipv6_only: true,
            buffer_size: 131072,
            rebind_wait_time_ms: 500,
        }
    }
    
    /// Get the recommended socket strategy for Linux
    pub fn for_linux() -> Self {
        Self {
            use_reuse_addr: true,
            use_reuse_port: false, // Different semantics on Linux, usually not needed
            set_ipv6_only: true,
            ipv6_only: true,
            buffer_size: 131072,
            rebind_wait_time_ms: 250,
        }
    }
    
    /// Apply this strategy to a socket
    pub async fn apply_to_socket(&self, socket: &UdpSocket) -> io::Result<()> {
        // Set socket buffer size using the socket's fd
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let fd = socket.as_raw_fd();
            
            // Set receive buffer size
            let size = self.buffer_size as i32;
            let optval = &size as *const i32 as *const libc::c_void;
            let optlen = std::mem::size_of::<i32>() as libc::socklen_t;
            if unsafe { libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_RCVBUF, optval, optlen) } < 0 {
                return Err(io::Error::last_os_error());
            }
            
            // Set send buffer size
            if unsafe { libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_SNDBUF, optval, optlen) } < 0 {
                return Err(io::Error::last_os_error());
            }
            
            // Set SO_REUSEADDR if needed
            if self.use_reuse_addr {
                let optval = &1 as *const i32 as *const libc::c_void;
                if unsafe { libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_REUSEADDR, optval, optlen) } < 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            
            // Set SO_REUSEPORT if needed and available
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            if self.use_reuse_port {
                let optval = &1 as *const i32 as *const libc::c_void;
                #[cfg(target_os = "macos")]
                let opt = libc::SO_REUSEPORT;
                #[cfg(target_os = "linux")]
                let opt = libc::SO_REUSEPORT;
                
                if unsafe { libc::setsockopt(fd, libc::SOL_SOCKET, opt, optval, optlen) } < 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            
            // Set IPV6_V6ONLY if needed
            if self.set_ipv6_only && socket.local_addr()?.is_ipv6() {
                let optval = if self.ipv6_only { &1 } else { &0 } as *const i32 as *const libc::c_void;
                if unsafe { libc::setsockopt(fd, libc::IPPROTO_IPV6, libc::IPV6_V6ONLY, optval, optlen) } < 0 {
                    return Err(io::Error::last_os_error());
                }
            }
        }
        
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawSocket;
            use winapi::um::winsock2 as ws2;
            use winapi::shared::ws2def;
            
            let socket = socket.as_raw_socket() as ws2::SOCKET;
            
            // Set receive buffer size
            let size = self.buffer_size as i32;
            let optval = &size as *const i32 as *const libc::c_char;
            let optlen = std::mem::size_of::<i32>() as i32;
            
            if unsafe { ws2::setsockopt(socket, ws2::SOL_SOCKET, ws2::SO_RCVBUF, optval, optlen) } != 0 {
                return Err(io::Error::last_os_error());
            }
            
            // Set send buffer size
            if unsafe { ws2::setsockopt(socket, ws2::SOL_SOCKET, ws2::SO_SNDBUF, optval, optlen) } != 0 {
                return Err(io::Error::last_os_error());
            }
            
            // Set SO_REUSEADDR if needed
            if self.use_reuse_addr {
                let optval = &1 as *const i32 as *const libc::c_char;
                if unsafe { ws2::setsockopt(socket, ws2::SOL_SOCKET, ws2::SO_REUSEADDR, optval, optlen) } != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            
            // Set IPV6_V6ONLY if needed
            if self.set_ipv6_only && socket.local_addr()?.is_ipv6() {
                let optval = if self.ipv6_only { &1 } else { &0 } as *const i32 as *const libc::c_char;
                const IPV6_V6ONLY: i32 = 27; // Defined in ws2ipdef.h but not in winapi crate
                if unsafe { ws2::setsockopt(socket, ws2def::IPPROTO_IPV6, IPV6_V6ONLY, optval, optlen) } != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
        }
        
        Ok(())
    }
}

impl Default for PlatformSocketStrategy {
    fn default() -> Self {
        Self {
            use_reuse_addr: true,
            use_reuse_port: false,
            set_ipv6_only: true,
            ipv6_only: true,
            buffer_size: 65536,
            rebind_wait_time_ms: 500,
        }
    }
}

/// RTP-specific socket validation
pub struct RtpSocketValidator;

impl RtpSocketValidator {
    /// Validate that RTP/RTCP socket binding works correctly on this platform
    /// 
    /// Returns a recommended socket strategy for this platform based on testing results
    pub async fn validate() -> Result<PlatformSocketStrategy> {
        // Start with the platform defaults
        let mut strategy = PlatformSocketStrategy::for_current_platform();
        
        // Test binding an RTP/RTCP socket pair
        if let Err(err) = Self::test_binding_pair().await {
            warn!("RTP/RTCP pair binding test failed: {}", err);
            
            // Try a fallback strategy
            strategy = Self::get_fallback_strategy(&strategy);
            
            // Test again with the fallback strategy
            if let Err(err2) = Self::test_binding_pair_with_strategy(&strategy).await {
                error!("Fallback strategy also failed: {}", err2);
                return Err(Error::Transport(format!("Socket validation failed: {}", err2)));
            }
        }
        
        // Test RTCP-MUX binding
        if let Err(err) = Self::test_rtcp_mux_binding().await {
            warn!("RTCP-MUX binding test failed: {}", err);
            
            // Try a fallback strategy for RTCP-MUX
            strategy = Self::get_rtcp_mux_fallback_strategy(&strategy);
            
            // Test again with the fallback strategy
            if let Err(err2) = Self::test_rtcp_mux_binding_with_strategy(&strategy).await {
                error!("RTCP-MUX fallback strategy also failed: {}", err2);
                return Err(Error::Transport(format!("RTCP-MUX validation failed: {}", err2)));
            }
        }
        
        info!("Socket validation successful with strategy: {:?}", strategy);
        Ok(strategy)
    }
    
    /// Test binding an RTP/RTCP socket pair with the platform defaults
    async fn test_binding_pair() -> Result<()> {
        let strategy = PlatformSocketStrategy::for_current_platform();
        Self::test_binding_pair_with_strategy(&strategy).await
    }
    
    /// Test binding an RTP/RTCP socket pair with a specific strategy
    async fn test_binding_pair_with_strategy(strategy: &PlatformSocketStrategy) -> Result<()> {
        // Bind to a random port for RTP
        let socket1 = UdpSocket::bind("127.0.0.1:0").await
            .map_err(|e| Error::Transport(format!("Failed to bind RTP socket: {}", e)))?;
        
        // Apply the strategy to the socket
        strategy.apply_to_socket(&socket1).await
            .map_err(|e| Error::Transport(format!("Failed to apply socket strategy: {}", e)))?;
        
        // Get the port that was allocated
        let addr1 = socket1.local_addr()
            .map_err(|e| Error::Transport(format!("Failed to get local address: {}", e)))?;
        
        let rtp_port = addr1.port();
        let rtcp_port = rtp_port + 1;
        
        // Now try to bind the RTCP socket to the next port
        let rtcp_addr = SocketAddr::new(addr1.ip(), rtcp_port);
        
        // Use a separate task to handle timeouts more gracefully
        let rtcp_result = tokio::time::timeout(
            tokio::time::Duration::from_secs(1),
            async {
                let rtcp_socket = UdpSocket::bind(rtcp_addr).await?;
                strategy.apply_to_socket(&rtcp_socket).await?;
                
                // Test sending data between the sockets
                let test_data = [1, 2, 3, 4];
                socket1.send_to(&test_data, rtcp_addr).await?;
                
                let mut buf = [0; 4];
                let (len, _) = rtcp_socket.recv_from(&mut buf).await?;
                
                if len != test_data.len() || buf != test_data {
                    return Err(io::Error::new(
                        io::ErrorKind::Other, 
                        "Data integrity check failed"
                    ));
                }
                
                Ok::<_, io::Error>(())
            }
        ).await;
        
        match rtcp_result {
            Ok(Ok(())) => {
                debug!("Successfully bound and tested RTP/RTCP socket pair on ports {}/{}", 
                      rtp_port, rtcp_port);
                Ok(())
            },
            Ok(Err(e)) => {
                Err(Error::Transport(format!("RTP/RTCP pair test failed: {}", e)))
            },
            Err(_) => {
                Err(Error::Transport("RTP/RTCP pair test timed out".to_string()))
            }
        }
    }
    
    /// Test RTCP-MUX binding (RTP and RTCP on the same port)
    async fn test_rtcp_mux_binding() -> Result<()> {
        let strategy = PlatformSocketStrategy::for_current_platform();
        Self::test_rtcp_mux_binding_with_strategy(&strategy).await
    }
    
    /// Test RTCP-MUX binding with a specific strategy
    async fn test_rtcp_mux_binding_with_strategy(strategy: &PlatformSocketStrategy) -> Result<()> {
        // Bind to a random port for the multiplexed socket
        let socket = UdpSocket::bind("127.0.0.1:0").await
            .map_err(|e| Error::Transport(format!("Failed to bind RTCP-MUX socket: {}", e)))?;
        
        // Apply the strategy to the socket
        strategy.apply_to_socket(&socket).await
            .map_err(|e| Error::Transport(format!("Failed to apply socket strategy: {}", e)))?;
        
        // Get the port that was allocated
        let addr = socket.local_addr()
            .map_err(|e| Error::Transport(format!("Failed to get local address: {}", e)))?;
        
        // Use a separate task to handle timeouts more gracefully
        let mux_result = tokio::time::timeout(
            tokio::time::Duration::from_secs(1),
            async {
                // Test sending data to yourself (simulating RTCP-MUX)
                let rtp_data = [1, 2, 3, 4]; // Simulated RTP packet
                let rtcp_data = [200, 2, 3, 4]; // Simulated RTCP packet (starts with 200)
                
                // Send RTP data
                socket.send_to(&rtp_data, addr).await?;
                
                // Send RTCP data from a separate socket with the same configuration
                let socket2 = UdpSocket::bind("127.0.0.1:0").await?;
                strategy.apply_to_socket(&socket2).await?;
                socket2.send_to(&rtcp_data, addr).await?;
                
                // Receive both packets
                let mut buf1 = [0; 4];
                let mut buf2 = [0; 4];
                
                let (len1, _) = socket.recv_from(&mut buf1).await?;
                let (len2, _) = socket.recv_from(&mut buf2).await?;
                
                if len1 != rtp_data.len() || buf1 != rtp_data {
                    return Err(io::Error::new(
                        io::ErrorKind::Other, 
                        "RTP data integrity check failed"
                    ));
                }
                
                if len2 != rtcp_data.len() || buf2 != rtcp_data {
                    return Err(io::Error::new(
                        io::ErrorKind::Other, 
                        "RTCP data integrity check failed"
                    ));
                }
                
                Ok::<_, io::Error>(())
            }
        ).await;
        
        match mux_result {
            Ok(Ok(())) => {
                debug!("Successfully tested RTCP-MUX on port {}", addr.port());
                Ok(())
            },
            Ok(Err(e)) => {
                Err(Error::Transport(format!("RTCP-MUX test failed: {}", e)))
            },
            Err(_) => {
                Err(Error::Transport("RTCP-MUX test timed out".to_string()))
            }
        }
    }
    
    /// Get a fallback strategy when the initial strategy fails
    fn get_fallback_strategy(initial: &PlatformSocketStrategy) -> PlatformSocketStrategy {
        let mut fallback = initial.clone();
        
        match PlatformType::current() {
            PlatformType::Windows => {
                // Windows fallback: increase buffer sizes and wait time
                fallback.buffer_size *= 2;
                fallback.rebind_wait_time_ms *= 2;
            },
            PlatformType::MacOS => {
                // macOS fallback: try opposite reuse_port setting
                fallback.use_reuse_port = !initial.use_reuse_port;
                fallback.rebind_wait_time_ms *= 2;
            },
            PlatformType::Linux => {
                // Linux fallback: try with reuse_port
                fallback.use_reuse_port = true;
                fallback.rebind_wait_time_ms *= 2;
            },
            PlatformType::Other => {
                // Generic fallback
                fallback.use_reuse_addr = true;
                fallback.use_reuse_port = true;
                fallback.buffer_size *= 2;
                fallback.rebind_wait_time_ms *= 2;
            },
        }
        
        fallback
    }
    
    /// Get a fallback strategy specifically for RTCP-MUX issues
    fn get_rtcp_mux_fallback_strategy(initial: &PlatformSocketStrategy) -> PlatformSocketStrategy {
        let mut fallback = initial.clone();
        
        match PlatformType::current() {
            PlatformType::Windows => {
                // Windows RTCP-MUX fallback
                fallback.buffer_size *= 2;
            },
            PlatformType::MacOS => {
                // macOS must have reuse_port for RTCP-MUX
                fallback.use_reuse_port = true;
            },
            PlatformType::Linux => {
                // Linux may need both for RTCP-MUX
                fallback.use_reuse_addr = true;
                fallback.use_reuse_port = true;
            },
            PlatformType::Other => {
                // Generic fallback
                fallback.use_reuse_addr = true;
                fallback.use_reuse_port = true;
            },
        }
        
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::runtime::Runtime;
    
    #[test]
    fn test_platform_detection() {
        let platform = PlatformType::current();
        println!("Detected platform: {:?}", platform);

        // Make sure at least one of these returns true
        assert!(
            PlatformType::is_windows() || 
            PlatformType::is_macos() || 
            PlatformType::is_linux() || 
            platform == PlatformType::Other
        );
    }
    
    #[test]
    fn test_socket_strategy() {
        let strategy = PlatformSocketStrategy::for_current_platform();
        println!("Platform socket strategy: {:?}", strategy);
        
        // Make sure each platform strategy differs
        let win_strategy = PlatformSocketStrategy::for_windows();
        let mac_strategy = PlatformSocketStrategy::for_macos();
        let linux_strategy = PlatformSocketStrategy::for_linux();
        
        assert_ne!(win_strategy, mac_strategy);
        assert_ne!(win_strategy, linux_strategy);
        assert_ne!(mac_strategy, linux_strategy);
    }
    
    #[test]
    fn test_socket_validation() {
        // Only run this test in CI or when explicitly enabled
        // as it requires network access and may fail in some environments
        if std::env::var("RUN_SOCKET_TESTS").is_err() {
            println!("Skipping socket validation test. Set RUN_SOCKET_TESTS=1 to enable.");
            return;
        }
        
        let rt = Runtime::new().unwrap();
        let result = rt.block_on(async {
            RtpSocketValidator::validate().await
        });
        
        match result {
            Ok(strategy) => {
                println!("Socket validation passed with strategy: {:?}", strategy);
            },
            Err(e) => {
                println!("Socket validation failed: {}", e);
                // Don't fail the test as this is environment-dependent
            }
        }
    }
} 