//! Port allocation system for RTP/RTCP
//!
//! This module provides a port allocator that manages a pool of available ports
//! for RTP and RTCP sessions. It ensures efficient port usage, handles conflicts,
//! and provides platform-specific optimizations.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, Weak};
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tracing::{debug, error, info};

use super::validation::{PlatformSocketStrategy, PlatformType};
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

/// Delay before retrying a port whose authoritative socket bind failed.
const PORT_BIND_FAILURE_QUARANTINE_MS: u64 = 1000;

static NEXT_ALLOCATOR_ID: AtomicU64 = AtomicU64::new(1);

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct IndexedPoolKey {
    ip: IpAddr,
    port_range_start: u16,
    port_range_end: u16,
    pairing_strategy: PairingStrategy,
}

struct SharedIndexedAllocatorState {
    available_ports: BTreeSet<u16>,
    allocated_ports: HashMap<u16, u64>,
    quarantined_ports: HashMap<u16, Instant>,
    last_port: u16,
}

struct IndexedAllocatorState {
    pools: HashMap<IpAddr, Arc<StdMutex<SharedIndexedAllocatorState>>>,
    session_ports: HashMap<String, Vec<(IpAddr, u16)>>,
}

type SharedIndexedPool = Arc<StdMutex<SharedIndexedAllocatorState>>;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum BindAddressFamily {
    Ipv4,
    Ipv6,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct AuthoritativePortKey {
    family: BindAddressFamily,
    port: u16,
}

#[derive(Clone, Copy, Debug)]
struct AuthoritativePortClaim {
    ip: IpAddr,
    owner_id: u64,
}

#[derive(Clone, Copy, Debug)]
struct AuthoritativePortQuarantine {
    ip: IpAddr,
    deadline: Instant,
}

#[derive(Default)]
struct AuthoritativePortState {
    claims: HashMap<AuthoritativePortKey, Vec<AuthoritativePortClaim>>,
    quarantines: HashMap<AuthoritativePortKey, Vec<AuthoritativePortQuarantine>>,
}

fn indexed_pool_registry(
) -> &'static StdMutex<HashMap<IndexedPoolKey, Weak<StdMutex<SharedIndexedAllocatorState>>>> {
    static REGISTRY: once_cell::sync::OnceCell<
        StdMutex<HashMap<IndexedPoolKey, Weak<StdMutex<SharedIndexedAllocatorState>>>>,
    > = once_cell::sync::OnceCell::new();
    REGISTRY.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn authoritative_port_registry() -> &'static StdMutex<AuthoritativePortState> {
    static REGISTRY: once_cell::sync::OnceCell<StdMutex<AuthoritativePortState>> =
        once_cell::sync::OnceCell::new();
    REGISTRY.get_or_init(|| StdMutex::new(AuthoritativePortState::default()))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PortClaimOutcome {
    Claimed,
    Unavailable,
    BindCollision,
}

/// Aggregate allocator diagnostics safe for logs and health endpoints.
/// Session identifiers, IP addresses, and exact port numbers are omitted.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PortAllocatorDiagnostics {
    pub capacity_ports: usize,
    pub allocated_ports: usize,
    pub active_sessions: usize,
    pub recently_released_ports: usize,
}

/// Port allocation manager
pub struct PortAllocator {
    /// Stable owner identity for reservations held in process-shared pools.
    allocator_id: u64,

    /// Allocator configuration
    config: PortAllocatorConfig,

    /// Per-allocator session ownership plus handles to process-shared pools for
    /// high-capacity unvalidated allocators.
    indexed_state: Option<Arc<StdMutex<IndexedAllocatorState>>>,

    /// Currently allocated ports
    allocated_ports: Arc<Mutex<HashSet<u16>>>,

    /// Recently released ports that can be reused
    released_ports: Arc<Mutex<Vec<ReleasedPort>>>,

    /// Ports rejected by an authoritative socket bind. Unlike ordinary
    /// releases, these are not candidates for immediate reuse.
    quarantined_ports: Arc<Mutex<HashMap<(IpAddr, u16), Instant>>>,

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
                pools: HashMap::new(),
                session_ports: HashMap::with_capacity(config.capacity_hint),
            }))
        });

        Self {
            allocator_id: NEXT_ALLOCATOR_ID.fetch_add(1, Ordering::Relaxed),
            config: config.clone(),
            indexed_state,
            allocated_ports: Arc::new(Mutex::new(HashSet::with_capacity(config.capacity_hint))),
            released_ports: Arc::new(Mutex::new(Vec::with_capacity(
                config.capacity_hint.min(1024),
            ))),
            quarantined_ports: Arc::new(Mutex::new(HashMap::new())),
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

    /// Return aggregate-only allocator state without exposing tenant/session
    /// identifiers or routable addresses.
    pub async fn diagnostics(&self) -> Result<PortAllocatorDiagnostics> {
        let capacity_ports = usize::from(
            self.config
                .port_range_end
                .saturating_sub(self.config.port_range_start),
        ) + 1;

        if let Some(state) = &self.indexed_state {
            let state = state
                .lock()
                .map_err(|_| Error::Transport("Indexed port pool lock poisoned".to_string()))?;
            let allocated_ports = state.pools.values().try_fold(0usize, |count, pool| {
                let pool = pool
                    .lock()
                    .map_err(|_| Error::Transport("Shared port pool lock poisoned".to_string()))?;
                Ok::<_, Error>(
                    count
                        + pool
                            .allocated_ports
                            .values()
                            .filter(|owner| **owner == self.allocator_id)
                            .count(),
                )
            })?;
            return Ok(PortAllocatorDiagnostics {
                capacity_ports,
                allocated_ports,
                active_sessions: state.session_ports.len(),
                recently_released_ports: 0,
            });
        }

        Ok(PortAllocatorDiagnostics {
            capacity_ports,
            allocated_ports: self.allocated_ports.lock().await.len(),
            active_sessions: self.session_ports.lock().await.len(),
            recently_released_ports: self.released_ports.lock().await.len(),
        })
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
            return self.allocate_indexed_unvalidated(ip);
        }

        let mut retries = 0;
        let mut bind_collisions = 0;

        while retries < self.config.allocation_retries {
            // Try to use a recently released port first if preferred
            if self.config.prefer_port_reuse {
                if let Some(port) = self.find_reusable_port(ip).await {
                    match self.claim_port(ip, port).await {
                        PortClaimOutcome::Claimed => return Ok(port),
                        PortClaimOutcome::BindCollision => bind_collisions += 1,
                        PortClaimOutcome::Unavailable => {}
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
            match self.claim_port(ip, port).await {
                PortClaimOutcome::Claimed => return Ok(port),
                PortClaimOutcome::BindCollision => bind_collisions += 1,
                PortClaimOutcome::Unavailable => {}
            }

            retries += 1;
        }

        if bind_collisions > 0 {
            Err(Error::Transport(format!(
                "RTP port allocation exhausted after {} attempts ({} OS bind collisions)",
                self.config.allocation_retries, bind_collisions
            )))
        } else {
            Err(Error::Transport(format!(
                "RTP port pool exhausted after {} allocation attempts",
                self.config.allocation_retries
            )))
        }
    }

    fn allocate_indexed_port_pair(
        &self,
        session_id: &str,
        ip: IpAddr,
    ) -> Result<(SocketAddr, Option<SocketAddr>)> {
        let local_state = self
            .indexed_state
            .as_ref()
            .ok_or_else(|| Error::Transport("Indexed port pool is not configured".to_string()))?;
        let mut local_state = local_state
            .lock()
            .map_err(|_| Error::Transport("Indexed port pool lock poisoned".to_string()))?;
        let pool = self.indexed_pool_for(&mut local_state, ip)?;
        let mut pool = pool
            .lock()
            .map_err(|_| Error::Transport("Shared port pool lock poisoned".to_string()))?;

        match self.config.pairing_strategy {
            PairingStrategy::Muxed => {
                let port = self.take_indexed_port(&mut pool, ip)?;
                self.track_indexed_allocation(&mut local_state, session_id, ip, port);
                Ok((SocketAddr::new(ip, port), None))
            }
            PairingStrategy::Separate => {
                let rtp_port = self.take_indexed_port(&mut pool, ip)?;
                let rtcp_port = match self.take_indexed_port(&mut pool, ip) {
                    Ok(port) => port,
                    Err(e) => {
                        self.release_indexed_port(&mut pool, ip, rtp_port)?;
                        return Err(e);
                    }
                };
                self.track_indexed_allocation(&mut local_state, session_id, ip, rtp_port);
                self.track_indexed_allocation(&mut local_state, session_id, ip, rtcp_port);
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

    fn allocate_indexed_unvalidated(&self, ip: IpAddr) -> Result<u16> {
        let local_state = self
            .indexed_state
            .as_ref()
            .ok_or_else(|| Error::Transport("Indexed port pool is not configured".to_string()))?;
        let mut local_state = local_state
            .lock()
            .map_err(|_| Error::Transport("Indexed port pool lock poisoned".to_string()))?;
        let pool = self.indexed_pool_for(&mut local_state, ip)?;
        let mut pool = pool
            .lock()
            .map_err(|_| Error::Transport("Shared port pool lock poisoned".to_string()))?;

        self.take_indexed_port(&mut pool, ip)
    }

    fn indexed_pool_for(
        &self,
        local_state: &mut IndexedAllocatorState,
        ip: IpAddr,
    ) -> Result<SharedIndexedPool> {
        if let Some(pool) = local_state.pools.get(&ip) {
            return Ok(pool.clone());
        }

        let key = IndexedPoolKey {
            ip,
            port_range_start: self.config.port_range_start,
            port_range_end: self.config.port_range_end,
            pairing_strategy: self.config.pairing_strategy,
        };
        let mut registry = indexed_pool_registry()
            .lock()
            .map_err(|_| Error::Transport("Shared port pool registry lock poisoned".to_string()))?;
        registry.retain(|_, pool| pool.strong_count() > 0);

        let pool = registry
            .get(&key)
            .and_then(Weak::upgrade)
            .unwrap_or_else(|| {
                let pool = Arc::new(StdMutex::new(SharedIndexedAllocatorState {
                    available_ports: (self.config.port_range_start..=self.config.port_range_end)
                        .collect(),
                    allocated_ports: HashMap::with_capacity(self.config.capacity_hint),
                    quarantined_ports: HashMap::new(),
                    last_port: self.config.port_range_start,
                }));
                registry.insert(key, Arc::downgrade(&pool));
                pool
            });
        local_state.pools.insert(ip, pool.clone());
        Ok(pool)
    }

    fn bind_address_family(ip: IpAddr) -> BindAddressFamily {
        match ip {
            IpAddr::V4(_) => BindAddressFamily::Ipv4,
            IpAddr::V6(_) => BindAddressFamily::Ipv6,
        }
    }

    fn bind_ips_conflict(first: IpAddr, second: IpAddr) -> bool {
        Self::bind_address_family(first) == Self::bind_address_family(second)
            && (first == second || first.is_unspecified() || second.is_unspecified())
    }

    fn authoritative_port_key(ip: IpAddr, port: u16) -> AuthoritativePortKey {
        AuthoritativePortKey {
            family: Self::bind_address_family(ip),
            port,
        }
    }

    fn try_claim_authoritative_port(&self, ip: IpAddr, port: u16) -> Result<bool> {
        let key = Self::authoritative_port_key(ip, port);
        let mut registry = authoritative_port_registry().lock().map_err(|_| {
            Error::Transport("Authoritative port claim registry lock poisoned".to_string())
        })?;

        let now = Instant::now();
        let (quarantine_conflict, quarantine_empty) = registry
            .quarantines
            .get_mut(&key)
            .map(|quarantines| {
                quarantines.retain(|quarantine| quarantine.deadline > now);
                (
                    quarantines
                        .iter()
                        .any(|quarantine| Self::bind_ips_conflict(ip, quarantine.ip)),
                    quarantines.is_empty(),
                )
            })
            .unwrap_or((false, false));
        if quarantine_empty {
            registry.quarantines.remove(&key);
        }
        if quarantine_conflict {
            return Ok(false);
        }

        let claims = registry.claims.entry(key).or_default();
        if claims
            .iter()
            .any(|claim| Self::bind_ips_conflict(ip, claim.ip))
        {
            return Ok(false);
        }
        claims.push(AuthoritativePortClaim {
            ip,
            owner_id: self.allocator_id,
        });
        Ok(true)
    }

    fn release_authoritative_port(&self, ip: IpAddr, port: u16) -> Result<()> {
        let key = Self::authoritative_port_key(ip, port);
        let mut registry = authoritative_port_registry().lock().map_err(|_| {
            Error::Transport("Authoritative port claim registry lock poisoned".to_string())
        })?;
        let remove_bucket = registry
            .claims
            .get_mut(&key)
            .map(|claims| {
                claims.retain(|claim| claim.owner_id != self.allocator_id || claim.ip != ip);
                claims.is_empty()
            })
            .unwrap_or(false);
        if remove_bucket {
            registry.claims.remove(&key);
        }
        Ok(())
    }

    fn quarantine_authoritative_port(&self, ip: IpAddr, port: u16) -> Result<()> {
        let key = Self::authoritative_port_key(ip, port);
        let mut registry = authoritative_port_registry().lock().map_err(|_| {
            Error::Transport("Authoritative port claim registry lock poisoned".to_string())
        })?;
        let remove_claim_bucket = registry
            .claims
            .get_mut(&key)
            .map(|claims| {
                claims.retain(|claim| claim.owner_id != self.allocator_id || claim.ip != ip);
                claims.is_empty()
            })
            .unwrap_or(false);
        if remove_claim_bucket {
            registry.claims.remove(&key);
        }

        let deadline = Instant::now() + Duration::from_millis(PORT_BIND_FAILURE_QUARANTINE_MS);
        let quarantines = registry.quarantines.entry(key).or_default();
        if let Some(existing) = quarantines
            .iter_mut()
            .find(|quarantine| quarantine.ip == ip)
        {
            existing.deadline = deadline;
        } else {
            quarantines.push(AuthoritativePortQuarantine { ip, deadline });
        }
        Ok(())
    }

    fn release_all_authoritative_ports(&self) {
        let Ok(mut registry) = authoritative_port_registry().lock() else {
            return;
        };
        registry.claims.retain(|_, claims| {
            claims.retain(|claim| claim.owner_id != self.allocator_id);
            !claims.is_empty()
        });
    }

    fn take_indexed_port(
        &self,
        state: &mut SharedIndexedAllocatorState,
        ip: IpAddr,
    ) -> Result<u16> {
        self.release_expired_indexed_quarantine(state);
        let mut scan_cursor = state.last_port;
        let mut process_conflicts = 0usize;
        for _ in 0..state.available_ports.len() {
            let Some(port) = state
                .available_ports
                .range(scan_cursor..)
                .next()
                .copied()
                .or_else(|| state.available_ports.iter().next().copied())
            else {
                break;
            };
            scan_cursor = if port >= self.config.port_range_end {
                self.config.port_range_start
            } else {
                port + 1
            };
            if state.allocated_ports.contains_key(&port) {
                continue;
            }
            if !self.try_claim_authoritative_port(ip, port)? {
                process_conflicts += 1;
                continue;
            }

            state.available_ports.remove(&port);
            state.allocated_ports.insert(port, self.allocator_id);
            state.last_port = scan_cursor;
            return Ok(port);
        }

        Err(Error::Transport(format!(
            "RTP port pool exhausted ({} active reservations, {} temporarily quarantined, {} process bind conflicts)",
            state.allocated_ports.len(),
            state.quarantined_ports.len(),
            process_conflicts
        )))
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

    fn release_expired_indexed_quarantine(&self, state: &mut SharedIndexedAllocatorState) {
        let now = Instant::now();
        let expired: Vec<u16> = state
            .quarantined_ports
            .iter()
            .filter_map(|(port, deadline)| (*deadline <= now).then_some(*port))
            .collect();
        for port in expired {
            state.quarantined_ports.remove(&port);
            if !state.allocated_ports.contains_key(&port) {
                state.available_ports.insert(port);
            }
        }
    }

    fn release_indexed_port(
        &self,
        state: &mut SharedIndexedAllocatorState,
        ip: IpAddr,
        port: u16,
    ) -> Result<()> {
        if state.allocated_ports.get(&port) == Some(&self.allocator_id)
            && port >= self.config.port_range_start
            && port <= self.config.port_range_end
        {
            self.release_authoritative_port(ip, port)?;
            state.allocated_ports.remove(&port);
            state.available_ports.insert(port);
        }
        Ok(())
    }

    fn quarantine_indexed_port(
        &self,
        state: &mut SharedIndexedAllocatorState,
        ip: IpAddr,
        port: u16,
    ) -> Result<()> {
        if state.allocated_ports.get(&port) != Some(&self.allocator_id) {
            return Ok(());
        }

        self.quarantine_authoritative_port(ip, port)?;
        state.allocated_ports.remove(&port);
        state.available_ports.remove(&port);
        state.quarantined_ports.insert(
            port,
            Instant::now() + Duration::from_millis(PORT_BIND_FAILURE_QUARANTINE_MS),
        );
        Ok(())
    }

    /// Release all ports associated with a session
    pub async fn release_session(&self, session_id: &str) -> Result<()> {
        if let Some(local_state) = &self.indexed_state {
            let mut local_state = local_state
                .lock()
                .map_err(|_| Error::Transport("Indexed port pool lock poisoned".to_string()))?;

            if let Some(ports) = local_state.session_ports.remove(session_id) {
                for (ip, port) in ports {
                    if let Some(pool) = local_state.pools.get(&ip) {
                        let mut pool = pool.lock().map_err(|_| {
                            Error::Transport("Shared port pool lock poisoned".to_string())
                        })?;
                        self.release_indexed_port(&mut pool, ip, port)?;
                    }
                }
                return Ok(());
            }

            return Err(Error::Transport(format!(
                "No session found with ID: {}",
                session_id
            )));
        }

        let ports = self.session_ports.lock().await.remove(session_id);
        if let Some(ports) = ports {
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

    /// Release a session after an authoritative RTP/RTCP socket bind failed.
    ///
    /// Unlike [`Self::release_session`], the rejected ports are quarantined for
    /// a short bounded interval so an immediate retry advances to a different
    /// candidate. This is intended for callers that reserve without a probe
    /// bind and then discover an OS-level collision while creating the real
    /// transport socket.
    pub async fn quarantine_session(&self, session_id: &str) -> Result<()> {
        if let Some(local_state) = &self.indexed_state {
            let mut local_state = local_state
                .lock()
                .map_err(|_| Error::Transport("Indexed port pool lock poisoned".to_string()))?;
            let ports = local_state
                .session_ports
                .remove(session_id)
                .ok_or_else(|| {
                    Error::Transport(format!("No session found with ID: {}", session_id))
                })?;

            for (ip, port) in ports {
                if let Some(pool) = local_state.pools.get(&ip) {
                    let mut pool = pool.lock().map_err(|_| {
                        Error::Transport("Shared port pool lock poisoned".to_string())
                    })?;
                    self.quarantine_indexed_port(&mut pool, ip, port)?;
                }
            }
            return Ok(());
        }

        let ports = self
            .session_ports
            .lock()
            .await
            .remove(session_id)
            .ok_or_else(|| Error::Transport(format!("No session found with ID: {}", session_id)))?;
        for (ip, port) in ports {
            self.quarantine_port(ip, port).await;
        }
        Ok(())
    }

    /// Release a specific port
    pub async fn release_port(&self, ip: IpAddr, port: u16) {
        if let Some(local_state) = &self.indexed_state {
            match local_state.lock() {
                Ok(local_state) => {
                    if let Some(pool) = local_state.pools.get(&ip) {
                        match pool.lock() {
                            Ok(mut pool) => {
                                if let Err(error) = self.release_indexed_port(&mut pool, ip, port) {
                                    error!("Failed to release authoritative port claim: {}", error);
                                }
                            }
                            Err(_) => {
                                error!("Shared port pool lock poisoned while releasing {}", port)
                            }
                        }
                    }
                }
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

    async fn quarantine_port(&self, ip: IpAddr, port: u16) {
        let removed = self.allocated_ports.lock().await.remove(&port);
        if !removed {
            return;
        }

        self.released_ports
            .lock()
            .await
            .retain(|released| released.ip != ip || released.port != port);
        self.quarantined_ports.lock().await.insert(
            (ip, port),
            Instant::now() + Duration::from_millis(PORT_BIND_FAILURE_QUARANTINE_MS),
        );
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
                Ok(state) => state
                    .pools
                    .values()
                    .filter_map(|pool| pool.lock().ok())
                    .map(|pool| {
                        pool.allocated_ports
                            .values()
                            .filter(|owner| **owner == self.allocator_id)
                            .count()
                    })
                    .sum(),
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
    async fn claim_port(&self, ip: IpAddr, port: u16) -> PortClaimOutcome {
        // Check if port is in valid range
        if port < self.config.port_range_start || port > self.config.port_range_end {
            return PortClaimOutcome::Unavailable;
        }

        let now = Instant::now();
        {
            let mut quarantined = self.quarantined_ports.lock().await;
            match quarantined.get(&(ip, port)).copied() {
                Some(deadline) if deadline > now => return PortClaimOutcome::Unavailable,
                Some(_) => {
                    quarantined.remove(&(ip, port));
                }
                None => {}
            }
        }

        // Lock allocated ports ONCE and hold it through the entire operation
        let mut allocated = self.allocated_ports.lock().await;

        // Check if already allocated while holding the lock
        if allocated.contains(&port) {
            return PortClaimOutcome::Unavailable;
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

                    PortClaimOutcome::Claimed
                }
                Err(e) => {
                    debug!("Failed to bind to port {}: {}", port, e);

                    // Remove from allocated since we couldn't bind
                    let mut allocated = self.allocated_ports.lock().await;
                    allocated.remove(&port);
                    drop(allocated);

                    if e.kind() == std::io::ErrorKind::AddrInUse {
                        self.quarantined_ports.lock().await.insert(
                            (ip, port),
                            Instant::now() + Duration::from_millis(PORT_BIND_FAILURE_QUARANTINE_MS),
                        );
                        PortClaimOutcome::BindCollision
                    } else {
                        PortClaimOutcome::Unavailable
                    }
                }
            }
        } else {
            // No validation required, just mark as allocated atomically
            allocated.insert(port);
            PortClaimOutcome::Claimed
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

impl Drop for PortAllocator {
    fn drop(&mut self) {
        if let Some(local_state) = &self.indexed_state {
            if let Ok(mut local_state) = local_state.lock() {
                for (ip, pool) in &local_state.pools {
                    let Ok(mut pool) = pool.lock() else {
                        continue;
                    };
                    let owned_ports: Vec<u16> = pool
                        .allocated_ports
                        .iter()
                        .filter_map(|(port, owner)| (*owner == self.allocator_id).then_some(*port))
                        .collect();
                    for port in owned_ports {
                        let _ = self.release_authoritative_port(*ip, port);
                        pool.allocated_ports.remove(&port);
                        if !pool.quarantined_ports.contains_key(&port) {
                            pool.available_ports.insert(port);
                        }
                    }
                }
                local_state.session_ports.clear();
            }
        }
        self.release_all_authoritative_ports();
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
            if allocator.is_some() {
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

    fn indexed_test_config(
        port_range_start: u16,
        port_range_end: u16,
        pairing_strategy: PairingStrategy,
        default_ip: IpAddr,
    ) -> PortAllocatorConfig {
        PortAllocatorConfig {
            port_range_start,
            port_range_end,
            allocation_strategy: AllocationStrategy::Incremental,
            pairing_strategy,
            prefer_port_reuse: false,
            default_ip,
            allocation_retries: u32::from(port_range_end - port_range_start) + 1,
            validate_ports: false,
            capacity_hint: usize::from(port_range_end - port_range_start) + 1,
        }
    }

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
            port_range_start: 11000,
            port_range_end: 11999,
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
                port_range_start: 10200,
                port_range_end: 10260,
                allocation_strategy: AllocationStrategy::Incremental,
                pairing_strategy: PairingStrategy::Muxed,
                prefer_port_reuse: false,
                default_ip: IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                allocation_retries: 61,
                validate_ports: false,
                capacity_hint: 61,
            };
            let allocator = PortAllocator::with_config(config);
            let ip = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);

            {
                let indexed_state = allocator
                    .indexed_state
                    .as_ref()
                    .expect("indexed state should be enabled");
                let mut local_state = indexed_state.lock().expect("indexed state lock");
                let pool = allocator
                    .indexed_pool_for(&mut local_state, ip)
                    .expect("shared indexed pool");
                let mut state = pool.lock().expect("shared indexed pool lock");
                for port in 10200..=10254 {
                    state.available_ports.remove(&port);
                    state.allocated_ports.insert(port, allocator.allocator_id);
                }
                state.last_port = 10200;
            }

            let port = allocator
                .allocate_port(ip)
                .await
                .expect("allocator should find the first indexed free port");

            assert_eq!(port, 10255);
        });
    }

    #[test]
    fn test_indexed_unvalidated_allocation_reuses_released_port() {
        let rt = Runtime::new().unwrap();

        rt.block_on(async {
            let config = PortAllocatorConfig {
                port_range_start: 10300,
                port_range_end: 10302,
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

            assert_eq!([first, second, third], [10300, 10301, 10302]);
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
    fn bounded_pool_exhaustion_release_and_diagnostics_are_deterministic() {
        let rt = Runtime::new().unwrap();

        rt.block_on(async {
            let allocator = PortAllocator::with_config(PortAllocatorConfig {
                port_range_start: 12_000,
                port_range_end: 12_001,
                allocation_strategy: AllocationStrategy::Incremental,
                pairing_strategy: PairingStrategy::Muxed,
                prefer_port_reuse: false,
                default_ip: IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                allocation_retries: 2,
                validate_ports: false,
                capacity_hint: 2,
            });
            let ip = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);

            let first = allocator
                .allocate_port_pair("tenant-secret-call-a", Some(ip))
                .await
                .expect("first reservation")
                .0;
            allocator
                .allocate_port_pair("tenant-secret-call-b", Some(ip))
                .await
                .expect("second reservation");
            let exhaustion = allocator
                .allocate_port_pair("tenant-secret-call-c", Some(ip))
                .await
                .expect_err("a bounded pool must fail closed when exhausted");
            assert!(exhaustion.to_string().contains("RTP port pool exhausted"));

            let full = allocator.diagnostics().await.expect("diagnostics");
            assert_eq!(full.capacity_ports, 2);
            assert_eq!(full.allocated_ports, 2);
            assert_eq!(full.active_sessions, 2);
            let rendered = format!("{full:?}");
            assert!(!rendered.contains("tenant-secret"));
            assert!(!rendered.contains("127.0.0.1"));
            assert!(!rendered.contains("12000"));

            allocator
                .release_session("tenant-secret-call-a")
                .await
                .expect("release first session");
            let reused = allocator
                .allocate_port_pair("replacement", Some(ip))
                .await
                .expect("released capacity is reusable")
                .0;
            assert_eq!(reused, first);
        });
    }

    #[test]
    fn indexed_allocators_share_reservations_without_sharing_session_namespaces() {
        let rt = Runtime::new().unwrap();

        rt.block_on(async {
            let config = PortAllocatorConfig {
                port_range_start: 15_600,
                port_range_end: 15_601,
                allocation_strategy: AllocationStrategy::Incremental,
                pairing_strategy: PairingStrategy::Muxed,
                prefer_port_reuse: false,
                default_ip: IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                allocation_retries: 2,
                validate_ports: false,
                capacity_hint: 2,
            };
            let first_allocator = PortAllocator::with_config(config.clone());
            let second_allocator = PortAllocator::with_config(config.clone());
            let ip = config.default_ip;

            // Deliberately use the same session ID in two allocator instances.
            // Reservation ownership must be shared while release ownership stays
            // local to each controller/allocator namespace.
            let first = first_allocator
                .allocate_port_pair("same-dialog", Some(ip))
                .await
                .expect("first reservation")
                .0;
            let second = second_allocator
                .allocate_port_pair("same-dialog", Some(ip))
                .await
                .expect("second reservation")
                .0;
            assert_ne!(first, second);
            assert_eq!(first_allocator.allocated_count().await, 1);
            assert_eq!(second_allocator.allocated_count().await, 1);

            first_allocator
                .release_session("same-dialog")
                .await
                .expect("release first owner");
            assert_eq!(second_allocator.allocated_count().await, 1);

            let replacement_allocator = PortAllocator::with_config(config);
            let replacement = replacement_allocator
                .allocate_port_pair("same-dialog", Some(ip))
                .await
                .expect("replacement reservation")
                .0;
            assert_eq!(replacement, first);

            second_allocator
                .release_session("same-dialog")
                .await
                .expect("release second owner");
            assert_eq!(replacement_allocator.allocated_count().await, 1);
            replacement_allocator
                .release_session("same-dialog")
                .await
                .expect("release replacement owner");
        });
    }

    #[test]
    fn authoritative_claims_coordinate_overlapping_ranges() {
        let rt = Runtime::new().unwrap();

        rt.block_on(async {
            let ip = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
            let first_allocator = PortAllocator::with_config(indexed_test_config(
                19_600,
                19_601,
                PairingStrategy::Muxed,
                ip,
            ));
            let overlapping_allocator = PortAllocator::with_config(indexed_test_config(
                19_601,
                19_602,
                PairingStrategy::Muxed,
                ip,
            ));

            first_allocator
                .allocate_port_pair("first", Some(ip))
                .await
                .expect("first reservation");
            let overlap = first_allocator
                .allocate_port_pair("overlap", Some(ip))
                .await
                .expect("overlap reservation")
                .0;
            assert_eq!(overlap.port(), 19_601);

            let nonconflicting = overlapping_allocator
                .allocate_port_pair("other-range", Some(ip))
                .await
                .expect("overlapping range should advance past process claim")
                .0;
            assert_eq!(nonconflicting.port(), 19_602);

            first_allocator
                .release_session("overlap")
                .await
                .expect("release overlap");
            let reclaimed = overlapping_allocator
                .allocate_port_pair("reclaimed-overlap", Some(ip))
                .await
                .expect("previously conflicted candidate should remain reusable")
                .0;
            assert_eq!(reclaimed.port(), 19_601);

            first_allocator
                .release_session("first")
                .await
                .expect("release first");
            overlapping_allocator
                .release_session("other-range")
                .await
                .expect("release other range");
            overlapping_allocator
                .release_session("reclaimed-overlap")
                .await
                .expect("release reclaimed overlap");
        });
    }

    #[test]
    fn authoritative_claims_coordinate_differing_pairing_policies() {
        let rt = Runtime::new().unwrap();

        rt.block_on(async {
            let ip = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
            let muxed_allocator = PortAllocator::with_config(indexed_test_config(
                19_610,
                19_612,
                PairingStrategy::Muxed,
                ip,
            ));
            let separate_allocator = PortAllocator::with_config(indexed_test_config(
                19_610,
                19_612,
                PairingStrategy::Separate,
                ip,
            ));

            let muxed = muxed_allocator
                .allocate_port_pair("muxed", Some(ip))
                .await
                .expect("muxed reservation")
                .0;
            assert_eq!(muxed.port(), 19_610);

            let (rtp, rtcp) = separate_allocator
                .allocate_port_pair("separate", Some(ip))
                .await
                .expect("separate policy should honor muxed process claim");
            assert_eq!(rtp.port(), 19_611);
            assert_eq!(rtcp.expect("separate RTCP address").port(), 19_612);

            muxed_allocator
                .release_session("muxed")
                .await
                .expect("release muxed");
            separate_allocator
                .release_session("separate")
                .await
                .expect("release separate");
        });
    }

    #[test]
    fn authoritative_claims_apply_wildcard_bind_semantics() {
        let rt = Runtime::new().unwrap();

        rt.block_on(async {
            let wildcard = IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED);
            let localhost = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
            let alternate_loopback = IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 2));

            let wildcard_first = PortAllocator::with_config(indexed_test_config(
                19_620,
                19_621,
                PairingStrategy::Muxed,
                wildcard,
            ));
            let concrete_second = PortAllocator::with_config(indexed_test_config(
                19_620,
                19_621,
                PairingStrategy::Muxed,
                localhost,
            ));
            let wildcard_port = wildcard_first
                .allocate_port_pair("wildcard-first", Some(wildcard))
                .await
                .expect("wildcard reservation")
                .0;
            let concrete_port = concrete_second
                .allocate_port_pair("concrete-second", Some(localhost))
                .await
                .expect("concrete bind should advance past wildcard claim")
                .0;
            assert_eq!(wildcard_port.port(), 19_620);
            assert_eq!(concrete_port.port(), 19_621);

            let concrete_first = PortAllocator::with_config(indexed_test_config(
                19_622,
                19_623,
                PairingStrategy::Muxed,
                localhost,
            ));
            let wildcard_second = PortAllocator::with_config(indexed_test_config(
                19_622,
                19_623,
                PairingStrategy::Muxed,
                wildcard,
            ));
            let concrete_port = concrete_first
                .allocate_port_pair("concrete-first", Some(localhost))
                .await
                .expect("concrete reservation")
                .0;
            let wildcard_port = wildcard_second
                .allocate_port_pair("wildcard-second", Some(wildcard))
                .await
                .expect("wildcard bind should advance past concrete claim")
                .0;
            assert_eq!(concrete_port.port(), 19_622);
            assert_eq!(wildcard_port.port(), 19_623);

            let localhost_allocator = PortAllocator::with_config(indexed_test_config(
                19_624,
                19_624,
                PairingStrategy::Muxed,
                localhost,
            ));
            let alternate_allocator = PortAllocator::with_config(indexed_test_config(
                19_624,
                19_624,
                PairingStrategy::Muxed,
                alternate_loopback,
            ));
            let localhost_port = localhost_allocator
                .allocate_port_pair("localhost", Some(localhost))
                .await
                .expect("localhost reservation")
                .0;
            let alternate_port = alternate_allocator
                .allocate_port_pair("alternate", Some(alternate_loopback))
                .await
                .expect("distinct concrete IP may share a port")
                .0;
            assert_eq!(localhost_port.port(), alternate_port.port());

            wildcard_first
                .release_session("wildcard-first")
                .await
                .expect("release wildcard first");
            concrete_second
                .release_session("concrete-second")
                .await
                .expect("release concrete second");
            concrete_first
                .release_session("concrete-first")
                .await
                .expect("release concrete first");
            wildcard_second
                .release_session("wildcard-second")
                .await
                .expect("release wildcard second");
            localhost_allocator
                .release_session("localhost")
                .await
                .expect("release localhost");
            alternate_allocator
                .release_session("alternate")
                .await
                .expect("release alternate");
        });
    }

    #[test]
    fn shared_indexed_pool_handles_concurrent_capacity_and_churn() {
        let rt = Runtime::new().unwrap();

        rt.block_on(async {
            const ACTIVE_PER_ALLOCATOR: usize = 500;
            const CHURN_PER_ALLOCATOR: usize = 5_000;
            let config = PortAllocatorConfig {
                port_range_start: 18_000,
                port_range_end: 19_023,
                allocation_strategy: AllocationStrategy::Incremental,
                pairing_strategy: PairingStrategy::Muxed,
                prefer_port_reuse: false,
                default_ip: IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                allocation_retries: 1_024,
                validate_ports: false,
                capacity_hint: ACTIVE_PER_ALLOCATOR,
            };
            let first_allocator = Arc::new(PortAllocator::with_config(config.clone()));
            let second_allocator = Arc::new(PortAllocator::with_config(config));
            let ip = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);

            let allocate_many = |allocator: Arc<PortAllocator>, prefix: &'static str| {
                tokio::spawn(async move {
                    let mut ports = Vec::with_capacity(ACTIVE_PER_ALLOCATOR);
                    for index in 0..ACTIVE_PER_ALLOCATOR {
                        let session_id = format!("{prefix}-{index}");
                        let address = allocator
                            .allocate_port_pair(&session_id, Some(ip))
                            .await
                            .expect("shared capacity reservation")
                            .0;
                        ports.push((session_id, address.port()));
                        if index % 32 == 0 {
                            tokio::task::yield_now().await;
                        }
                    }
                    ports
                })
            };
            let first_active = allocate_many(first_allocator.clone(), "first");
            let second_active = allocate_many(second_allocator.clone(), "second");
            let first_active = first_active.await.expect("first allocation task");
            let second_active = second_active.await.expect("second allocation task");

            let unique_ports: HashSet<u16> = first_active
                .iter()
                .chain(&second_active)
                .map(|(_, port)| *port)
                .collect();
            assert_eq!(unique_ports.len(), ACTIVE_PER_ALLOCATOR * 2);

            for (session_id, _) in first_active {
                first_allocator
                    .release_session(&session_id)
                    .await
                    .expect("release first active reservation");
            }
            for (session_id, _) in second_active {
                second_allocator
                    .release_session(&session_id)
                    .await
                    .expect("release second active reservation");
            }

            let churn = |allocator: Arc<PortAllocator>| {
                tokio::spawn(async move {
                    for index in 0..CHURN_PER_ALLOCATOR {
                        // The two allocators deliberately reuse the same local
                        // session IDs while churning the shared port domain.
                        let session_id = format!("churn-{index}");
                        allocator
                            .allocate_port_pair(&session_id, Some(ip))
                            .await
                            .expect("churn reservation");
                        allocator
                            .release_session(&session_id)
                            .await
                            .expect("churn release");
                        if index % 64 == 0 {
                            tokio::task::yield_now().await;
                        }
                    }
                })
            };
            let (first_churn, second_churn) = tokio::join!(
                churn(first_allocator.clone()),
                churn(second_allocator.clone())
            );
            first_churn.expect("first churn task");
            second_churn.expect("second churn task");

            assert_eq!(first_allocator.allocated_count().await, 0);
            assert_eq!(second_allocator.allocated_count().await, 0);
            assert_eq!(
                first_allocator
                    .diagnostics()
                    .await
                    .expect("first diagnostics")
                    .active_sessions,
                0
            );
            assert_eq!(
                second_allocator
                    .diagnostics()
                    .await
                    .expect("second diagnostics")
                    .active_sessions,
                0
            );
        });
    }

    #[test]
    fn bind_failure_quarantine_advances_shared_pool_candidate() {
        let rt = Runtime::new().unwrap();

        rt.block_on(async {
            let config = PortAllocatorConfig {
                port_range_start: 15_610,
                port_range_end: 15_611,
                allocation_strategy: AllocationStrategy::Incremental,
                pairing_strategy: PairingStrategy::Muxed,
                prefer_port_reuse: false,
                default_ip: IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                allocation_retries: 2,
                validate_ports: false,
                capacity_hint: 2,
            };
            let rejected_allocator = PortAllocator::with_config(config.clone());
            let retry_allocator = PortAllocator::with_config(config.clone());
            let ip = config.default_ip;

            let rejected = rejected_allocator
                .allocate_port_pair("bind-failure", Some(ip))
                .await
                .expect("initial reservation")
                .0;
            rejected_allocator
                .quarantine_session("bind-failure")
                .await
                .expect("quarantine rejected reservation");

            let retry = retry_allocator
                .allocate_port_pair("retry", Some(ip))
                .await
                .expect("retry reservation")
                .0;
            assert_ne!(retry, rejected);
            retry_allocator
                .release_session("retry")
                .await
                .expect("release retry");

            // Even after ordinary capacity is released, the rejected candidate
            // remains out of circulation during its quarantine interval.
            let replacement_allocator = PortAllocator::with_config(config);
            let replacement = replacement_allocator
                .allocate_port_pair("replacement", Some(ip))
                .await
                .expect("replacement reservation")
                .0;
            assert_eq!(replacement, retry);
            replacement_allocator
                .release_session("replacement")
                .await
                .expect("release replacement");
        });
    }

    #[test]
    fn validated_allocator_reports_os_bind_collision_exhaustion() {
        let rt = Runtime::new().unwrap();

        rt.block_on(async {
            let held = std::net::UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                .expect("bind occupied port");
            let occupied_port = held.local_addr().expect("occupied address").port();
            let allocator = PortAllocator::with_config(PortAllocatorConfig {
                port_range_start: occupied_port,
                port_range_end: occupied_port,
                allocation_strategy: AllocationStrategy::Incremental,
                pairing_strategy: PairingStrategy::Muxed,
                prefer_port_reuse: false,
                default_ip: IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                allocation_retries: 1,
                validate_ports: true,
                capacity_hint: 1,
            });

            let error = allocator
                .allocate_port(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))
                .await
                .expect_err("occupied port must fail validation");
            assert!(error.to_string().contains("OS bind collisions"));
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

            let (_rtp_addr, rtcp_addr) = result.unwrap();

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
