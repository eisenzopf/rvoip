//! Port allocation system for RTP/RTCP
//!
//! This module provides a port allocator that manages a pool of available ports
//! for RTP and RTCP sessions. It ensures efficient port usage, handles conflicts,
//! and provides platform-specific optimizations.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use crate::error::Error;
use crate::Result;
use super::validation::{PlatformSocketStrategy, PlatformType, RtpSocketValidator};

/// The default RTP port range recommended by RFC 3550
/// 
/// The range 49152-65535 is for dynamic/private ports
/// For RTP we use a portion of this range by default
pub const DEFAULT_RTP_PORT_RANGE_START: u16 = 16384; // Commonly used start for RTP
pub const DEFAULT_RTP_PORT_RANGE_END: u16 = 32767;   // Commonly used end for RTP

/// Minimum port value in the valid range
pub const MIN_PORT: u16 = 1024; // Avoid privileged ports

/// Delay before a port can be reused after being released
const PORT_REUSE_DELAY_MS: u64 = 1000; // Default 1 second

/// Port allocation strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocationStrategy {
    /// Sequential allocation (starts from the beginning of the range)
    Sequential,
    /// Random allocation (picks a random port in the range)
    Random,
    /// Incremental allocation (starts from the last allocated port)
    Incremental,
}

/// Port pairing strategy for RTP/RTCP
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingStrategy {
    /// Use adjacent ports (RTP on even, RTCP on odd)
    Adjacent,
    /// Use separate random ports for RTP and RTCP
    Separate,
    /// Use the same port for both (RTCP multiplexing)
    Muxed,
}

/// Port allocation configuration
#[derive(Debug, Clone)]
pub struct PortAllocatorConfig {
    /// Starting port number of the allocation range
    pub port_range_start: u16,
    
    /// Ending port number of the allocation range
    pub port_range_end: u16,
    
    /// Port allocation strategy
    pub allocation_strategy: AllocationStrategy,
    
    /// Port pairing strategy for RTP/RTCP
    pub pairing_strategy: PairingStrategy,
    
    /// Whether to prefer reusing recently released ports
    pub prefer_port_reuse: bool,
    
    /// Default IP address for binding
    pub default_ip: IpAddr,
    
    /// Number of allocation retries before giving up
    pub allocation_retries: u32,
    
    /// Whether to validate port availability before returning
    pub validate_ports: bool,
}

impl Default for PortAllocatorConfig {
    fn default() -> Self {
        Self {
            port_range_start: DEFAULT_RTP_PORT_RANGE_START,
            port_range_end: DEFAULT_RTP_PORT_RANGE_END,
            allocation_strategy: AllocationStrategy::Random,
            pairing_strategy: PairingStrategy::Muxed, // Default to RTCP multiplexing
            prefer_port_reuse: true,
            default_ip: IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
            allocation_retries: 10,
            validate_ports: true,
        }
    }
}

/// Represents a recently released port that may be reused
struct ReleasedPort {
    /// The port number
    port: u16,
    
    /// The IP address the port was bound to
    ip: IpAddr,
    
    /// When the port was released
    released_at: Instant,
}

/// Port allocation manager
pub struct PortAllocator {
    /// Allocator configuration
    config: PortAllocatorConfig,
    
    /// Currently allocated ports
    allocated_ports: Arc<Mutex<HashSet<u16>>>,
    
    /// Recently released ports that can be reused
    released_ports: Arc<Mutex<Vec<ReleasedPort>>>,
    
    /// Maps session IDs to their allocated ports
    session_ports: Arc<Mutex<HashMap<String, Vec<u16>>>>,
    
    /// Last allocated port (for incremental strategy)
    last_port: Arc<Mutex<u16>>,
    
    /// Platform-specific socket strategy
    socket_strategy: PlatformSocketStrategy,
}

impl PortAllocator {
    /// Create a new port allocator with default configuration
    pub fn new() -> Self {
        Self::with_config(PortAllocatorConfig::default())
    }
    
    /// Create a new port allocator with custom configuration
    pub fn with_config(config: PortAllocatorConfig) -> Self {
        let socket_strategy = PlatformSocketStrategy::for_current_platform();
        
        Self {
            config: config.clone(),
            allocated_ports: Arc::new(Mutex::new(HashSet::new())),
            released_ports: Arc::new(Mutex::new(Vec::new())),
            session_ports: Arc::new(Mutex::new(HashMap::new())),
            last_port: Arc::new(Mutex::new(config.port_range_start)),
            socket_strategy,
        }
    }
    
    /// Get the current platform socket strategy
    pub fn socket_strategy(&self) -> PlatformSocketStrategy {
        self.socket_strategy.clone()
    }
    
    /// Set a custom platform socket strategy
    pub fn set_socket_strategy(&mut self, strategy: PlatformSocketStrategy) {
        self.socket_strategy = strategy;
    }
    
    /// Allocate a pair of ports for RTP and RTCP
    ///
    /// Returns (rtp_socket_addr, rtcp_socket_addr)
    /// If RTCP multiplexing is enabled, both addresses will be the same
    pub async fn allocate_port_pair(&self, session_id: &str, ip: Option<IpAddr>) -> Result<(SocketAddr, Option<SocketAddr>)> {
        let ip = ip.unwrap_or(self.config.default_ip);
        
        // Clean up any stale released ports
        self.cleanup_released_ports().await;
        
        match self.config.pairing_strategy {
            PairingStrategy::Muxed => {
                // Allocate a single port for both RTP and RTCP
                let port = self.allocate_port(ip).await?;
                let socket_addr = SocketAddr::new(ip, port);
                
                // Track this allocation
                self.track_allocation(session_id, port).await?;
                
                Ok((socket_addr, None))
            },
            PairingStrategy::Adjacent => {
                // Find an available even port for RTP
                let rtp_port = match self.find_available_even_port(ip).await {
                    Some(port) => port,
                    None => return Err(Error::Transport("No available even ports for RTP".to_string())),
                };
                
                let rtcp_port = rtp_port + 1;
                
                // Verify the RTCP port is available
                if !self.is_port_available(ip, rtcp_port).await {
                    // Release the RTP port since we can't use the pair
                    self.release_port(ip, rtp_port).await;
                    return Err(Error::Transport(format!("RTCP port {} is not available", rtcp_port)));
                }
                
                // Mark both ports as allocated
                let rtp_addr = SocketAddr::new(ip, rtp_port);
                let rtcp_addr = SocketAddr::new(ip, rtcp_port);
                
                // Track this allocation
                self.track_allocation(session_id, rtp_port).await?;
                self.track_allocation(session_id, rtcp_port).await?;
                
                Ok((rtp_addr, Some(rtcp_addr)))
            },
            PairingStrategy::Separate => {
                // Allocate two separate ports
                let rtp_port = self.allocate_port(ip).await?;
                let rtcp_port = self.allocate_port(ip).await?;
                
                let rtp_addr = SocketAddr::new(ip, rtp_port);
                let rtcp_addr = SocketAddr::new(ip, rtcp_port);
                
                // Track this allocation
                self.track_allocation(session_id, rtp_port).await?;
                self.track_allocation(session_id, rtcp_port).await?;
                
                Ok((rtp_addr, Some(rtcp_addr)))
            },
        }
    }
    
    /// Allocate a single port for generic usage
    pub async fn allocate_port(&self, ip: IpAddr) -> Result<u16> {
        let mut retries = 0;
        
        while retries < self.config.allocation_retries {
            // Try to use a recently released port first if preferred
            if self.config.prefer_port_reuse {
                if let Some(port) = self.find_reusable_port(ip).await {
                    if self.claim_port(ip, port).await {
                        return Ok(port);
                    }
                }
            }
            
            // Get a port based on allocation strategy
            let port = match self.config.allocation_strategy {
                AllocationStrategy::Sequential => self.get_next_sequential_port().await,
                AllocationStrategy::Random => self.get_random_port().await,
                AllocationStrategy::Incremental => self.get_next_incremental_port().await,
            };
            
            // Check if the port is available
            if self.claim_port(ip, port).await {
                return Ok(port);
            }
            
            retries += 1;
        }
        
        Err(Error::Transport("Failed to allocate port after maximum retries".to_string()))
    }
    
    /// Release all ports associated with a session
    pub async fn release_session(&self, session_id: &str) -> Result<()> {
        let mut sessions = self.session_ports.lock().await;
        
        if let Some(ports) = sessions.remove(session_id) {
            // Get the IP to use for releasing
            let ip = self.config.default_ip;
            
            // Release each port
            for port in ports {
                self.release_port(ip, port).await;
            }
            
            Ok(())
        } else {
            Err(Error::Transport(format!("No session found with ID: {}", session_id)))
        }
    }
    
    /// Release a specific port
    pub async fn release_port(&self, ip: IpAddr, port: u16) {
        // Remove from allocated ports
        {
            let mut allocated = self.allocated_ports.lock().await;
            allocated.remove(&port);
        }
        
        // Add to released ports for potential reuse
        {
            let mut released = self.released_ports.lock().await;
            released.push(ReleasedPort {
                port,
                ip,
                released_at: Instant::now(),
            });
        }
        
        debug!("Released port {} on {}", port, ip);
    }
    
    /// Create a validated socket for a given address
    /// 
    /// This applies the platform-specific socket settings
    pub async fn create_validated_socket(&self, addr: SocketAddr) -> Result<UdpSocket> {
        // Bind socket
        let socket = UdpSocket::bind(addr).await
            .map_err(|e| Error::Transport(format!("Failed to bind socket to {}: {}", addr, e)))?;
        
        // Apply platform-specific settings
        self.socket_strategy.apply_to_socket(&socket).await
            .map_err(|e| Error::Transport(format!("Failed to apply socket settings: {}", e)))?;
        
        Ok(socket)
    }
    
    /// Get the total number of currently allocated ports
    pub async fn allocated_count(&self) -> usize {
        let allocated = self.allocated_ports.lock().await;
        allocated.len()
    }
    
    /// Get the total number of ports in the configured range
    pub fn total_ports(&self) -> usize {
        (self.config.port_range_end - self.config.port_range_start + 1) as usize
    }
    
    /// Find an available even port (for RTP when using adjacent port pairs)
    async fn find_available_even_port(&self, ip: IpAddr) -> Option<u16> {
        let mut retries = 0;
        
        while retries < self.config.allocation_retries * 2 {
            // Get a candidate port based on the allocation strategy
            let mut port = match self.config.allocation_strategy {
                AllocationStrategy::Sequential => self.get_next_sequential_port().await,
                AllocationStrategy::Random => self.get_random_port().await,
                AllocationStrategy::Incremental => self.get_next_incremental_port().await,
            };
            
            // Make sure it's even
            if port % 2 != 0 {
                port -= 1;
                if port < self.config.port_range_start {
                    port = self.config.port_range_start + (port % 2);
                }
            }
            
            // Check if the port is available
            if self.is_port_available(ip, port).await {
                return Some(port);
            }
            
            retries += 1;
        }
        
        None
    }
    
    /// Find a recently released port that can be reused
    async fn find_reusable_port(&self, ip: IpAddr) -> Option<u16> {
        let mut released = self.released_ports.lock().await;
        
        // Find a released port for the same IP that's been released long enough
        let now = Instant::now();
        let reuse_delay = Duration::from_millis(PORT_REUSE_DELAY_MS);
        
        // Find a suitable port
        let index = released.iter().position(|p| 
            p.ip == ip && 
            now.duration_since(p.released_at) > reuse_delay
        );
        
        // If found, remove and return it
        if let Some(idx) = index {
            let port = released.remove(idx).port;
            Some(port)
        } else {
            None
        }
    }
    
    /// Try to claim a port
    async fn claim_port(&self, ip: IpAddr, port: u16) -> bool {
        if !self.is_port_available(ip, port).await {
            return false;
        }
        
        // If validation is required, actually try to bind the socket
        if self.config.validate_ports {
            let addr = SocketAddr::new(ip, port);
            
            // Before trying to bind, add to our allocated ports to avoid race conditions
            {
                let mut allocated = self.allocated_ports.lock().await;
                allocated.insert(port);
            }
            
            // Try binding
            match UdpSocket::bind(addr).await {
                Ok(socket) => {
                    // Get the local address for logging before we drop the socket
                    let local_addr = socket.local_addr().ok();
                    
                    // Explicitly close the socket, which should release it
                    drop(socket);
                    
                    // On some platforms we need a small delay before the port is truly released
                    if let Some(local_addr) = local_addr {
                        debug!("Successfully validated port {} (bound to {})", port, local_addr);
                    }
                    
                    // Delay based on platform
                    let delay_ms = self.socket_strategy.rebind_wait_time_ms;
                    if delay_ms > 0 {
                        sleep(Duration::from_millis(delay_ms)).await;
                    }
                    
                    true
                },
                Err(e) => {
                    debug!("Failed to bind to port {}: {}", port, e);
                    
                    // Remove from allocated since we couldn't bind
                    let mut allocated = self.allocated_ports.lock().await;
                    allocated.remove(&port);
                    
                    false
                }
            }
        } else {
            // No validation required, just mark as allocated
            let mut allocated = self.allocated_ports.lock().await;
            allocated.insert(port);
            true
        }
    }
    
    /// Check if a port is available
    async fn is_port_available(&self, ip: IpAddr, port: u16) -> bool {
        // First, check if it's in our allocated ports
        let allocated = self.allocated_ports.lock().await;
        if allocated.contains(&port) {
            return false;
        }
        
        // Check if it's in the configured range
        if port < self.config.port_range_start || port > self.config.port_range_end {
            return false;
        }
        
        // If we're not doing validation, we're done
        if !self.config.validate_ports {
            return true;
        }
        
        // Try to create a UDP socket to verify availability
        let addr = SocketAddr::new(ip, port);
        match UdpSocket::bind(addr).await {
            Ok(_) => true,
            Err(_) => false,
        }
    }
    
    /// Get the next port using sequential allocation
    async fn get_next_sequential_port(&self) -> u16 {
        let mut last_port = self.last_port.lock().await;
        let port = *last_port;
        
        // Update last port
        *last_port = if port + 1 > self.config.port_range_end {
            self.config.port_range_start
        } else {
            port + 1
        };
        
        port
    }
    
    /// Get the next port using incremental allocation
    async fn get_next_incremental_port(&self) -> u16 {
        let mut last_port = self.last_port.lock().await;
        
        // Start from the last allocated port
        let port = *last_port;
        
        // Update last port
        *last_port = if port + 1 > self.config.port_range_end {
            self.config.port_range_start
        } else {
            port + 1
        };
        
        port
    }
    
    /// Get a random port from the configured range
    async fn get_random_port(&self) -> u16 {
        use rand::Rng;
        
        let range_size = self.config.port_range_end - self.config.port_range_start + 1;
        let offset = rand::thread_rng().gen_range(0..range_size);
        
        self.config.port_range_start + offset
    }
    
    /// Track a port allocation for a session
    async fn track_allocation(&self, session_id: &str, port: u16) -> Result<()> {
        let mut sessions = self.session_ports.lock().await;
        let session_ports = sessions.entry(session_id.to_string()).or_insert_with(Vec::new);
        session_ports.push(port);
        Ok(())
    }
    
    /// Clean up ports that were released long ago
    async fn cleanup_released_ports(&self) {
        let mut released = self.released_ports.lock().await;
        
        // Calculate the cutoff time for cleanup
        let now = Instant::now();
        let reuse_delay = Duration::from_millis(PORT_REUSE_DELAY_MS * 10); // Much longer than reuse delay
        
        // Remove old entries
        released.retain(|p| now.duration_since(p.released_at) <= reuse_delay);
    }
}

/// Singleton port allocator for the application
pub struct GlobalPortAllocator;

impl GlobalPortAllocator {
    /// Get the global port allocator instance
    pub async fn instance() -> Arc<PortAllocator> {
        // Create the static Mutex if it doesn't exist
        static INSTANCE: once_cell::sync::OnceCell<Mutex<Option<Arc<PortAllocator>>>> = once_cell::sync::OnceCell::new();
        let static_mutex = INSTANCE.get_or_init(|| Mutex::new(None));
        
        // Lock the mutex and get/create the allocator
        let mut allocator = static_mutex.lock().await;
        
        if allocator.is_none() {
            // Create a new allocator with platform-specific settings
            let mut config = PortAllocatorConfig::default();
            
            // Adjust configuration based on platform
            match PlatformType::current() {
                PlatformType::Windows => {
                    // Windows typically has more restrictive port reuse
                    config.allocation_retries = 15;
                    config.port_range_start = 20000;
                    config.port_range_end = 30000;
                },
                PlatformType::MacOS => {
                    // macOS often needs a bit more time between rebinds
                    config.allocation_retries = 12;
                },
                PlatformType::Linux => {
                    // Linux is generally more flexible with port reuse
                    config.allocation_retries = 8;
                },
                _ => {}
            }
            
            // Create the allocator
            let port_allocator = PortAllocator::with_config(config.clone());
            *allocator = Some(Arc::new(port_allocator));
            
            // Log creation
            if let Some(ref alloc) = *allocator {
                info!("Created global port allocator with range {}-{}", 
                     config.port_range_start, 
                     config.port_range_end);
            }
        }
        
        // Return a clone of the Arc
        allocator.as_ref().unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::runtime::Runtime;
    
    #[test]
    fn test_port_allocator_creation() {
        let allocator = PortAllocator::new();
        assert_eq!(allocator.config.port_range_start, DEFAULT_RTP_PORT_RANGE_START);
        assert_eq!(allocator.config.port_range_end, DEFAULT_RTP_PORT_RANGE_END);
    }
    
    #[test]
    fn test_port_allocator_with_custom_config() {
        let config = PortAllocatorConfig {
            port_range_start: 10000,
            port_range_end: 20000,
            allocation_strategy: AllocationStrategy::Sequential,
            pairing_strategy: PairingStrategy::Adjacent,
            prefer_port_reuse: false,
            default_ip: IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            allocation_retries: 20,
            validate_ports: false,
        };
        
        let allocator = PortAllocator::with_config(config.clone());
        assert_eq!(allocator.config.port_range_start, config.port_range_start);
        assert_eq!(allocator.config.port_range_end, config.port_range_end);
        assert_eq!(allocator.config.allocation_strategy, config.allocation_strategy);
    }
    
    #[test]
    fn test_port_allocation() {
        let rt = Runtime::new().unwrap();
        
        rt.block_on(async {
            let allocator = PortAllocator::new();
            
            // Allocate a port
            let port = allocator.allocate_port(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)).await;
            assert!(port.is_ok());
            
            let port = port.unwrap();
            assert!(port >= DEFAULT_RTP_PORT_RANGE_START);
            assert!(port <= DEFAULT_RTP_PORT_RANGE_END);
            
            // Check that it's marked as allocated
            assert_eq!(allocator.allocated_count().await, 1);
            
            // Release the port
            allocator.release_port(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), port).await;
            
            // After release, allocated count should be 0
            assert_eq!(allocator.allocated_count().await, 0);
        });
    }
    
    #[test]
    fn test_port_pair_allocation() {
        let rt = Runtime::new().unwrap();
        
        rt.block_on(async {
            // Create allocator with Adjacent pairing strategy and validation disabled
            let config = PortAllocatorConfig {
                pairing_strategy: PairingStrategy::Adjacent,
                validate_ports: false, // Disable validation to avoid hanging
                default_ip: IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), // Use localhost instead of unspecified
                ..Default::default()
            };
            let allocator = PortAllocator::with_config(config);
            
            // Allocate a port pair
            let result = allocator.allocate_port_pair("test-session", None).await;
            assert!(result.is_ok());
            
            let (rtp_addr, rtcp_addr) = result.unwrap();
            assert!(rtcp_addr.is_some());
            
            let rtcp_addr = rtcp_addr.unwrap();
            
            // RTP port should be even
            assert_eq!(rtp_addr.port() % 2, 0);
            
            // RTCP port should be RTP port + 1
            assert_eq!(rtcp_addr.port(), rtp_addr.port() + 1);
            
            // Check the session allocations - should be 2 ports in the session
            let sessions = allocator.session_ports.lock().await;
            if let Some(session_ports) = sessions.get("test-session") {
                assert_eq!(session_ports.len(), 2, "Expected 2 ports allocated to the session");
            } else {
                panic!("Session test-session not found");
            }
            drop(sessions); // Explicitly drop the lock
            
            // Release the session
            let result = allocator.release_session("test-session").await;
            assert!(result.is_ok());
            
            // After release, session should be removed from session_ports
            let sessions = allocator.session_ports.lock().await;
            assert!(!sessions.contains_key("test-session"), "Session should be removed after release");
        });
    }
    
    #[test]
    fn test_muxed_port_allocation() {
        let rt = Runtime::new().unwrap();
        
        rt.block_on(async {
            // Create allocator with Muxed pairing strategy
            let config = PortAllocatorConfig {
                pairing_strategy: PairingStrategy::Muxed,
                ..Default::default()
            };
            let allocator = PortAllocator::with_config(config);
            
            // Allocate a port pair
            let result = allocator.allocate_port_pair("test-session", None).await;
            assert!(result.is_ok());
            
            let (rtp_addr, rtcp_addr) = result.unwrap();
            
            // RTCP address should be None for muxed
            assert!(rtcp_addr.is_none());
            
            // Check that only one port is marked as allocated
            assert_eq!(allocator.allocated_count().await, 1);
            
            // Release the session
            let result = allocator.release_session("test-session").await;
            assert!(result.is_ok());
            
            // After release, allocated count should be 0
            assert_eq!(allocator.allocated_count().await, 0);
        });
    }
    
    #[test]
    fn test_separate_port_allocation() {
        let rt = Runtime::new().unwrap();
        
        rt.block_on(async {
            // Create allocator with Separate pairing strategy
            let config = PortAllocatorConfig {
                pairing_strategy: PairingStrategy::Separate,
                ..Default::default()
            };
            let allocator = PortAllocator::with_config(config);
            
            // Allocate a port pair
            let result = allocator.allocate_port_pair("test-session", None).await;
            assert!(result.is_ok());
            
            let (rtp_addr, rtcp_addr) = result.unwrap();
            assert!(rtcp_addr.is_some());
            
            let rtcp_addr = rtcp_addr.unwrap();
            
            // RTP and RTCP ports should be different
            assert_ne!(rtp_addr.port(), rtcp_addr.port());
            
            // Check that both ports are marked as allocated
            assert_eq!(allocator.allocated_count().await, 2);
            
            // Release the session
            let result = allocator.release_session("test-session").await;
            assert!(result.is_ok());
            
            // After release, allocated count should be 0
            assert_eq!(allocator.allocated_count().await, 0);
        });
    }
    
    #[test]
    fn test_global_allocator() {
        let rt = Runtime::new().unwrap();
        
        rt.block_on(async {
            // Get the global allocator instance
            let allocator1 = GlobalPortAllocator::instance().await;
            
            // Get it again - should be the same instance
            let allocator2 = GlobalPortAllocator::instance().await;
            
            // Record the current Arc strong count - it varies depending on
            // other tests but allocator1 and allocator2 should have the same count
            let count1 = Arc::strong_count(&allocator1);
            let count2 = Arc::strong_count(&allocator2);
            assert_eq!(count1, count2);
            
            // Allocate a port
            let port = allocator1.allocate_port(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)).await;
            assert!(port.is_ok());
            
            // Check that it's marked as allocated in both instances (they're the same instance)
            let count1 = allocator1.allocated_count().await;
            let count2 = allocator2.allocated_count().await;
            assert_eq!(count1, count2);
            assert!(count1 > 0);
        });
    }
} 