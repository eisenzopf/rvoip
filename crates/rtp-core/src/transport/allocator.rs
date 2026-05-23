//! Port allocation system for RTP/RTCP
//!
//! This module provides a port allocator that manages a pool of available ports
//! for RTP and RTCP sessions. It ensures efficient port usage, handles conflicts,
//! and provides platform-specific optimizations.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use super::validation::{PlatformSocketStrategy, PlatformType, RtpSocketValidator};
use crate::error::Error;
use crate::Result;

/// The default RTP port range recommended by RFC 3550
///
/// The range 49152-65535 is for dynamic/private ports
/// For RTP we use a portion of this range by default
pub const DEFAULT_RTP_PORT_RANGE_START: u16 = 16384; // Commonly used start for RTP
pub const DEFAULT_RTP_PORT_RANGE_END: u16 = 32767; // Commonly used end for RTP

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

    /// Optional preallocation hint for allocation tracking indexes.
    ///
    /// `0` preserves lazy allocation. Servers with known RTP capacity can set
    /// this to the expected active media-session count to avoid rehashing under
    /// call setup bursts.
    pub capacity_hint: usize,
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
            // Each retry is one random pick + one probe bind. Bind-EADDRINUSE
            // from peer allocators sharing the same range counts against this
            // budget, so the value must absorb cross-allocator contention in
            // multi-coord test processes — not just allocator-internal collisions.
            allocation_retries: 50,
            validate_ports: true,
            capacity_hint: 0,
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

struct IndexedAllocatorState {
    available_ports: BTreeSet<u16>,
    allocated_ports: HashSet<u16>,
    session_ports: HashMap<String, Vec<(IpAddr, u16)>>,
    last_port: u16,
}

/// Port allocation manager
pub struct PortAllocator {
    /// Allocator configuration
    config: PortAllocatorConfig,

    /// Single-lock indexed state for high-capacity unvalidated allocators.
    indexed_state: Option<Arc<StdMutex<IndexedAllocatorState>>>,

    /// Currently allocated ports
    allocated_ports: Arc<Mutex<HashSet<u16>>>,

    /// Recently released ports that can be reused
    released_ports: Arc<Mutex<Vec<ReleasedPort>>>,

    /// Last time the released-port list was compacted.
    last_released_cleanup: Arc<Mutex<Instant>>,

    /// Maps session IDs to their allocated `(ip, port)` pairs
    session_ports: Arc<Mutex<HashMap<String, Vec<(IpAddr, u16)>>>>,

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
        let indexed_state = Self::indexed_pool_enabled(&config).then(|| {
            Arc::new(StdMutex::new(IndexedAllocatorState {
                available_ports: (config.port_range_start..=config.port_range_end).collect(),
                allocated_ports: HashSet::with_capacity(config.capacity_hint),
                session_ports: HashMap::with_capacity(config.capacity_hint),
                last_port: config.port_range_start,
            }))
        });

        Self {
            config: config.clone(),
            indexed_state,
            allocated_ports: Arc::new(Mutex::new(HashSet::with_capacity(config.capacity_hint))),
            released_ports: Arc::new(Mutex::new(Vec::with_capacity(
                config.capacity_hint.min(1024),
            ))),
            last_released_cleanup: Arc::new(Mutex::new(Instant::now())),
            session_ports: Arc::new(Mutex::new(HashMap::with_capacity(config.capacity_hint))),
            last_port: Arc::new(Mutex::new(config.port_range_start)),
            socket_strategy,
        }
    }

    fn indexed_pool_enabled(config: &PortAllocatorConfig) -> bool {
        matches!(
            config.allocation_strategy,
            AllocationStrategy::Sequential | AllocationStrategy::Incremental
        ) && matches!(
            config.pairing_strategy,
            PairingStrategy::Muxed | PairingStrategy::Separate
        ) && !config.prefer_port_reuse
            && !config.validate_ports
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
    pub async fn allocate_port_pair(
        &self,
        session_id: &str,
        ip: Option<IpAddr>,
    ) -> Result<(SocketAddr, Option<SocketAddr>)> {
        let ip = ip.unwrap_or(self.config.default_ip);

        if self.indexed_state.is_some() {
            return self.allocate_indexed_port_pair(session_id, ip);
        }

        // Clean up any stale released ports
        self.cleanup_released_ports().await;

        match self.config.pairing_strategy {
            PairingStrategy::Muxed => {
                // Allocate a single port for both RTP and RTCP
                let port = self.allocate_port(ip).await?;
                let socket_addr = SocketAddr::new(ip, port);

                // Track this allocation
                self.track_allocation(session_id, ip, port).await?;

                Ok((socket_addr, None))
            }
            PairingStrategy::Adjacent => {
                // Need to atomically allocate an even/odd port pair
                let mut retries = 0;
                while retries < self.config.allocation_retries {
                    // Get a candidate even port
                    let mut port = match self.config.allocation_strategy {
                        AllocationStrategy::Sequential => self.get_next_sequential_port().await,
                        AllocationStrategy::Random => self.get_random_port().await,
                        AllocationStrategy::Incremental => self.get_next_incremental_port().await,
                    };

                    // Make sure it's even
                    if port % 2 != 0 {
                        port -= 1;
                        if port < self.config.port_range_start {
                            port =
                                self.config.port_range_start + (self.config.port_range_start % 2);
                        }
                    }

                    let rtp_port = port;
                    let rtcp_port = port + 1;

                    // Check range validity
                    if rtcp_port > self.config.port_range_end {
                        retries += 1;
                        continue;
                    }

                    // Try to claim both ports atomically
                    let mut allocated = self.allocated_ports.lock().await;
                    if !allocated.contains(&rtp_port) && !allocated.contains(&rtcp_port) {
                        // Both are free, claim them
                        allocated.insert(rtp_port);
                        allocated.insert(rtcp_port);
                        drop(allocated);

                        // Track this allocation
                        self.track_allocation(session_id, ip, rtp_port).await?;
                        self.track_allocation(session_id, ip, rtcp_port).await?;

                        let rtp_addr = SocketAddr::new(ip, rtp_port);
                        let rtcp_addr = SocketAddr::new(ip, rtcp_port);

                        debug!(
                            "Allocated adjacent ports {} and {} for session {}",
                            rtp_port, rtcp_port, session_id
                        );
                        return Ok((rtp_addr, Some(rtcp_addr)));
                    }

                    retries += 1;
                }

                return Err(Error::Transport(
                    "Failed to allocate adjacent port pair after maximum retries".to_string(),
                ));
            }
            PairingStrategy::Separate => {
                // Allocate two separate ports
                let rtp_port = self.allocate_port(ip).await?;
                let rtcp_port = self.allocate_port(ip).await?;

                let rtp_addr = SocketAddr::new(ip, rtp_port);
                let rtcp_addr = SocketAddr::new(ip, rtcp_port);

                // Track this allocation
                self.track_allocation(session_id, ip, rtp_port).await?;
                self.track_allocation(session_id, ip, rtcp_port).await?;

                Ok((rtp_addr, Some(rtcp_addr)))
            }
        }
    }

    /// Allocate a single port for generic usage
    pub async fn allocate_port(&self, ip: IpAddr) -> Result<u16> {
        if self.indexed_state.is_some() {
            return self.allocate_indexed_unvalidated();
        }

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

        Err(Error::Transport(
            "Failed to allocate port after maximum retries".to_string(),
        ))
    }

    fn allocate_indexed_port_pair(
        &self,
        session_id: &str,
        ip: IpAddr,
    ) -> Result<(SocketAddr, Option<SocketAddr>)> {
        let state = self
            .indexed_state
            .as_ref()
            .ok_or_else(|| Error::Transport("Indexed port pool is not configured".to_string()))?;
        let mut state = state
            .lock()
            .map_err(|_| Error::Transport("Indexed port pool lock poisoned".to_string()))?;

        match self.config.pairing_strategy {
            PairingStrategy::Muxed => {
                let port = self.take_indexed_port(&mut state)?;
                self.track_indexed_allocation(&mut state, session_id, ip, port);
                Ok((SocketAddr::new(ip, port), None))
            }
            PairingStrategy::Separate => {
                let rtp_port = self.take_indexed_port(&mut state)?;
                let rtcp_port = match self.take_indexed_port(&mut state) {
                    Ok(port) => port,
                    Err(e) => {
                        self.release_indexed_port(&mut state, rtp_port);
                        return Err(e);
                    }
                };
                self.track_indexed_allocation(&mut state, session_id, ip, rtp_port);
                self.track_indexed_allocation(&mut state, session_id, ip, rtcp_port);
                Ok((
                    SocketAddr::new(ip, rtp_port),
                    Some(SocketAddr::new(ip, rtcp_port)),
                ))
            }
            PairingStrategy::Adjacent => Err(Error::Transport(
                "Indexed port pool does not support adjacent port pairs".to_string(),
            )),
        }
    }

    fn allocate_indexed_unvalidated(&self) -> Result<u16> {
        let state = self
            .indexed_state
            .as_ref()
            .ok_or_else(|| Error::Transport("Indexed port pool is not configured".to_string()))?;
        let mut state = state
            .lock()
            .map_err(|_| Error::Transport("Indexed port pool lock poisoned".to_string()))?;

        self.take_indexed_port(&mut state)
    }

    fn take_indexed_port(&self, state: &mut IndexedAllocatorState) -> Result<u16> {
        loop {
            let cursor = state.last_port;
            let port = state
                .available_ports
                .range(cursor..)
                .next()
                .copied()
                .or_else(|| state.available_ports.iter().next().copied());

            let Some(port) = port else {
                return Err(Error::Transport(
                    "Failed to allocate port after maximum retries".to_string(),
                ));
            };

            state.available_ports.remove(&port);

            if state.allocated_ports.contains(&port) {
                continue;
            }

            state.allocated_ports.insert(port);
            state.last_port = if port >= self.config.port_range_end {
                self.config.port_range_start
            } else {
                port + 1
            };
            return Ok(port);
        }
    }

    fn track_indexed_allocation(
        &self,
        state: &mut IndexedAllocatorState,
        session_id: &str,
        ip: IpAddr,
        port: u16,
    ) {
        state
            .session_ports
            .entry(session_id.to_string())
            .or_default()
            .push((ip, port));
    }

    fn release_indexed_port(&self, state: &mut IndexedAllocatorState, port: u16) {
        if state.allocated_ports.remove(&port)
            && port >= self.config.port_range_start
            && port <= self.config.port_range_end
        {
            state.available_ports.insert(port);
        }
    }

    /// Release all ports associated with a session
    pub async fn release_session(&self, session_id: &str) -> Result<()> {
        if let Some(state) = &self.indexed_state {
            let mut state = state
                .lock()
                .map_err(|_| Error::Transport("Indexed port pool lock poisoned".to_string()))?;

            if let Some(ports) = state.session_ports.remove(session_id) {
                for (_ip, port) in ports {
                    self.release_indexed_port(&mut state, port);
                }
                return Ok(());
            }

            return Err(Error::Transport(format!(
                "No session found with ID: {}",
                session_id
            )));
        }

        let mut sessions = self.session_ports.lock().await;

        if let Some(ports) = sessions.remove(session_id) {
            // Release each port on the same IP it was allocated for.
            for (ip, port) in ports {
                self.release_port(ip, port).await;
            }

            Ok(())
        } else {
            Err(Error::Transport(format!(
                "No session found with ID: {}",
                session_id
            )))
        }
    }

    /// Release a specific port
    pub async fn release_port(&self, ip: IpAddr, port: u16) {
        if let Some(state) = &self.indexed_state {
            match state.lock() {
                Ok(mut state) => self.release_indexed_port(&mut state, port),
                Err(_) => error!("Indexed port pool lock poisoned while releasing {}", port),
            }

            debug!("Released port {} on {}", port, ip);
            return;
        }

        // Remove from allocated ports
        let removed = {
            let mut allocated = self.allocated_ports.lock().await;
            allocated.remove(&port)
        };

        if !removed {
            debug!("Ignored release for unallocated port {} on {}", port, ip);
            return;
        }

        if !self.config.prefer_port_reuse {
            debug!("Released port {} on {}", port, ip);
            return;
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
        let socket = UdpSocket::bind(addr)
            .await
            .map_err(|e| Error::Transport(format!("Failed to bind socket to {}: {}", addr, e)))?;

        // Apply platform-specific settings
        self.socket_strategy
            .apply_to_socket(&socket)
            .await
            .map_err(|e| Error::Transport(format!("Failed to apply socket settings: {}", e)))?;

        Ok(socket)
    }

    /// Get the total number of currently allocated ports
    pub async fn allocated_count(&self) -> usize {
        if let Some(state) = &self.indexed_state {
            return match state.lock() {
                Ok(state) => state.allocated_ports.len(),
                Err(_) => 0,
            };
        }

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
        let index = released
            .iter()
            .position(|p| p.ip == ip && now.duration_since(p.released_at) > reuse_delay);

        // If found, remove and return it
        if let Some(idx) = index {
            let port = released.swap_remove(idx).port;
            Some(port)
        } else {
            None
        }
    }

    /// Try to claim a port atomically
    async fn claim_port(&self, ip: IpAddr, port: u16) -> bool {
        // Check if port is in valid range
        if port < self.config.port_range_start || port > self.config.port_range_end {
            return false;
        }

        // Lock allocated ports ONCE and hold it through the entire operation
        let mut allocated = self.allocated_ports.lock().await;

        // Check if already allocated while holding the lock
        if allocated.contains(&port) {
            return false;
        }

        // If validation is required, try to bind the socket
        if self.config.validate_ports {
            let addr = SocketAddr::new(ip, port);

            // Mark as allocated BEFORE releasing the lock for binding test
            allocated.insert(port);

            // Temporarily release the lock for the bind operation
            drop(allocated);

            // Try binding
            match UdpSocket::bind(addr).await {
                Ok(socket) => {
                    // Get the local address for logging before we drop the socket
                    let local_addr = socket.local_addr().ok();

                    // Explicitly close the probe socket — UDP has no TIME_WAIT so the
                    // port is immediately reusable on macOS, Linux, and Windows. The
                    // legacy `rebind_wait_time_ms` sleep here was a TCP-style
                    // TIME_WAIT intuition wrongly applied to UDP.
                    drop(socket);

                    if let Some(local_addr) = local_addr {
                        debug!(
                            "Successfully validated port {} (bound to {})",
                            port, local_addr
                        );
                    }

                    true
                }
                Err(e) => {
                    debug!("Failed to bind to port {}: {}", port, e);

                    // Remove from allocated since we couldn't bind
                    let mut allocated = self.allocated_ports.lock().await;
                    allocated.remove(&port);

                    false
                }
            }
        } else {
            // No validation required, just mark as allocated atomically
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
    async fn track_allocation(&self, session_id: &str, ip: IpAddr, port: u16) -> Result<()> {
        let mut sessions = self.session_ports.lock().await;
        let session_ports = sessions
            .entry(session_id.to_string())
            .or_insert_with(Vec::new);
        session_ports.push((ip, port));
        Ok(())
    }

    /// Clean up ports that were released long ago
    async fn cleanup_released_ports(&self) {
        if !self.config.prefer_port_reuse {
            return;
        }

        let now = Instant::now();
        {
            let mut last_cleanup = self.last_released_cleanup.lock().await;
            if now.duration_since(*last_cleanup) < Duration::from_secs(1) {
                return;
            }
            *last_cleanup = now;
        }

        let mut released = self.released_ports.lock().await;

        // Calculate the cutoff time for cleanup
        let reuse_delay = Duration::from_millis(PORT_REUSE_DELAY_MS * 10); // Much longer than reuse delay

        // Remove old entries
        released.retain(|p| now.duration_since(p.released_at) <= reuse_delay);
    }
}

/// Singleton port allocator for the application
pub struct GlobalPortAllocator;

impl GlobalPortAllocator {
    /// Configure the global port allocator with a custom port range
    /// This must be called BEFORE the first call to instance() to take effect
    pub async fn configure(start_port: u16, end_port: u16) -> Result<()> {
        // Create the static Mutex if it doesn't exist
        static INSTANCE: once_cell::sync::OnceCell<Mutex<Option<Arc<PortAllocator>>>> =
            once_cell::sync::OnceCell::new();
        let static_mutex = INSTANCE.get_or_init(|| Mutex::new(None));

        // Lock the mutex and check if allocator already exists
        let mut allocator = static_mutex.lock().await;

        if allocator.is_some() {
            // Allocator already exists, we can't reconfigure it
            return Err(Error::Transport(
                "Cannot reconfigure GlobalPortAllocator after it has been initialized".to_string(),
            ));
        }

        // Create a new allocator with the specified range
        let mut config = PortAllocatorConfig::default();
        config.port_range_start = start_port;
        config.port_range_end = end_port;

        // Adjust platform-specific settings
        match PlatformType::current() {
            PlatformType::Windows => {
                config.allocation_retries = 15;
            }
            PlatformType::MacOS => {
                config.allocation_retries = 12;
            }
            PlatformType::Linux => {
                config.allocation_retries = 8;
            }
            _ => {}
        }

        let port_allocator = PortAllocator::with_config(config.clone());
        *allocator = Some(Arc::new(port_allocator));

        info!(
            "Configured global port allocator with range {}-{}",
            start_port, end_port
        );
        Ok(())
    }

    /// Get the global port allocator instance
    pub async fn instance() -> Arc<PortAllocator> {
        // Create the static Mutex if it doesn't exist
        static INSTANCE: once_cell::sync::OnceCell<Mutex<Option<Arc<PortAllocator>>>> =
            once_cell::sync::OnceCell::new();
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
                }
                PlatformType::MacOS => {
                    // macOS often needs a bit more time between rebinds
                    config.allocation_retries = 12;
                }
                PlatformType::Linux => {
                    // Linux is generally more flexible with port reuse
                    config.allocation_retries = 8;
                }
                _ => {}
            }

            // Create the allocator
            let port_allocator = PortAllocator::with_config(config.clone());
            *allocator = Some(Arc::new(port_allocator));

            // Log creation
            if let Some(ref alloc) = *allocator {
                info!(
                    "Created global port allocator with range {}-{}",
                    config.port_range_start, config.port_range_end
                );
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
        assert_eq!(
            allocator.config.port_range_start,
            DEFAULT_RTP_PORT_RANGE_START
        );
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
            capacity_hint: 0,
        };

        let allocator = PortAllocator::with_config(config.clone());
        assert_eq!(allocator.config.port_range_start, config.port_range_start);
        assert_eq!(allocator.config.port_range_end, config.port_range_end);
        assert_eq!(
            allocator.config.allocation_strategy,
            config.allocation_strategy
        );
    }

    #[test]
    fn test_release_skips_reuse_list_when_reuse_disabled() {
        let rt = Runtime::new().unwrap();

        rt.block_on(async {
            let config = PortAllocatorConfig {
                port_range_start: 10000,
                port_range_end: 10100,
                allocation_strategy: AllocationStrategy::Incremental,
                pairing_strategy: PairingStrategy::Muxed,
                prefer_port_reuse: false,
                default_ip: IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                allocation_retries: 20,
                validate_ports: false,
                capacity_hint: 128,
            };
            let allocator = PortAllocator::with_config(config);
            let ip = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);

            for i in 0..32 {
                let session_id = format!("session-{i}");
                let _ = allocator
                    .allocate_port_pair(&session_id, Some(ip))
                    .await
                    .expect("allocate port pair");
                allocator
                    .release_session(&session_id)
                    .await
                    .expect("release session");
            }

            let released = allocator.released_ports.lock().await;
            assert!(
                released.is_empty(),
                "reuse-disabled allocators should not accumulate released-port scan state"
            );
        });
    }

    #[test]
    fn test_indexed_unvalidated_allocation_skips_stale_entries() {
        let rt = Runtime::new().unwrap();

        rt.block_on(async {
            let config = PortAllocatorConfig {
                port_range_start: 10000,
                port_range_end: 10060,
                allocation_strategy: AllocationStrategy::Incremental,
                pairing_strategy: PairingStrategy::Muxed,
                prefer_port_reuse: false,
                default_ip: IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                allocation_retries: 61,
                validate_ports: false,
                capacity_hint: 61,
            };
            let allocator = PortAllocator::with_config(config);

            {
                let indexed_state = allocator
                    .indexed_state
                    .as_ref()
                    .expect("indexed state should be enabled");
                let mut state = indexed_state.lock().expect("indexed state lock");
                for port in 10000..=10054 {
                    state.available_ports.remove(&port);
                    state.allocated_ports.insert(port);
                }
                state.last_port = 10000;
            }

            let port = allocator
                .allocate_port(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))
                .await
                .expect("allocator should find the first indexed free port");

            assert_eq!(port, 10055);
        });
    }

    #[test]
    fn test_indexed_unvalidated_allocation_reuses_released_port() {
        let rt = Runtime::new().unwrap();

        rt.block_on(async {
            let config = PortAllocatorConfig {
                port_range_start: 10000,
                port_range_end: 10002,
                allocation_strategy: AllocationStrategy::Incremental,
                pairing_strategy: PairingStrategy::Muxed,
                prefer_port_reuse: false,
                default_ip: IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                allocation_retries: 3,
                validate_ports: false,
                capacity_hint: 3,
            };
            let allocator = PortAllocator::with_config(config);
            let ip = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);

            let first = allocator.allocate_port(ip).await.expect("first port");
            let second = allocator.allocate_port(ip).await.expect("second port");
            let third = allocator.allocate_port(ip).await.expect("third port");

            assert_eq!([first, second, third], [10000, 10001, 10002]);
            assert!(allocator.allocate_port(ip).await.is_err());

            allocator.release_port(ip, second).await;
            let reused = allocator
                .allocate_port(ip)
                .await
                .expect("released port should be available through the index");

            assert_eq!(reused, second);
        });
    }

    #[test]
    fn test_port_allocation() {
        let rt = Runtime::new().unwrap();

        rt.block_on(async {
            let allocator = PortAllocator::new();

            // Allocate a port
            let port = allocator
                .allocate_port(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))
                .await;
            assert!(port.is_ok());

            let port = port.unwrap();
            assert!(port >= DEFAULT_RTP_PORT_RANGE_START);
            assert!(port <= DEFAULT_RTP_PORT_RANGE_END);

            // Check that it's marked as allocated
            assert_eq!(allocator.allocated_count().await, 1);

            // Release the port
            allocator
                .release_port(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), port)
                .await;

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
                assert_eq!(
                    session_ports.len(),
                    2,
                    "Expected 2 ports allocated to the session"
                );
            } else {
                panic!("Session test-session not found");
            }
            drop(sessions); // Explicitly drop the lock

            // Release the session
            let result = allocator.release_session("test-session").await;
            assert!(result.is_ok());

            // After release, session should be removed from session_ports
            let sessions = allocator.session_ports.lock().await;
            assert!(
                !sessions.contains_key("test-session"),
                "Session should be removed after release"
            );
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
            let port = allocator1
                .allocate_port(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))
                .await;
            assert!(port.is_ok());

            // Check that it's marked as allocated in both instances (they're the same instance)
            let count1 = allocator1.allocated_count().await;
            let count2 = allocator2.allocated_count().await;
            assert_eq!(count1, count2);
            assert!(count1 > 0);
        });
    }
}
