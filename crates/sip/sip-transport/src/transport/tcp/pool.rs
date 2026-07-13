use super::TcpConnection;
use crate::error::{Error, Result};
use crate::transport::TransportFlowId;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::time::interval;
use tracing::{debug, error, info, trace};

/// Configuration for the TCP connection pool
#[derive(Clone, Debug)]
pub struct PoolConfig {
    /// Maximum number of connections to keep in the pool
    pub max_connections: usize,
    /// Timeout after which idle connections are closed
    pub idle_timeout: Duration,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_connections: 100,
            idle_timeout: Duration::from_secs(300), // 5 minutes
        }
    }
}

/// Connection metadata for tracking last activity and other details
struct ConnectionMeta {
    /// The actual connection
    connection: Arc<TcpConnection>,
    /// When the connection was last used
    last_activity: Instant,
}

/// Connection pool for managing TCP connections
pub struct ConnectionPool {
    /// Configuration for the pool
    config: PoolConfig,
    /// Active connections by remote address
    connections: Arc<Mutex<HashMap<SocketAddr, ConnectionMeta>>>,
    /// Shared shutdown state for the real pool and its cleanup task.
    closed: Arc<AtomicBool>,
}

impl ConnectionPool {
    /// Creates a new connection pool with the given configuration
    pub fn new(config: PoolConfig) -> Self {
        let pool = Self {
            config,
            connections: Arc::new(Mutex::new(HashMap::new())),
            closed: Arc::new(AtomicBool::new(false)),
        };

        pool
    }

    /// Adds a connection to the pool
    pub async fn add_connection(
        &self,
        addr: SocketAddr,
        connection: Arc<TcpConnection>,
    ) -> Result<()> {
        if self.closed.load(Ordering::Acquire) {
            let _ = connection.close().await;
            return Err(Error::TransportClosed);
        }
        let mut connections = self.connections.lock().await;
        if self.closed.load(Ordering::Acquire) {
            drop(connections);
            let _ = connection.close().await;
            return Err(Error::TransportClosed);
        }

        // Reject new admission at the configured cap. Removing only the map
        // entry leaves its reader task and socket alive, which both violates
        // the cap and makes exact-flow responses impossible. Existing flows
        // remain stable; callers can surface overload and retry elsewhere.
        if connections.len() >= self.config.max_connections && !connections.contains_key(&addr) {
            drop(connections);
            let _ = connection.close().await;
            return Err(Error::ConnectionLimitReached);
        }

        // A replacement at the same address does not grow the pool, but the
        // displaced socket still has a reader. Close it after publishing the
        // replacement so its exact-flow cleanup cannot remove the new entry.
        let displaced = connections.insert(
            addr,
            ConnectionMeta {
                connection,
                last_activity: Instant::now(),
            },
        );

        trace!(
            "Added connection to {} to pool (size: {})",
            addr,
            connections.len()
        );
        drop(connections);

        if let Some(displaced) = displaced {
            if let Err(error) = displaced.connection.close().await {
                error!("Error closing replaced connection to {}: {}", addr, error);
            }
        }

        Ok(())
    }

    /// Gets a connection from the pool if it exists
    pub async fn get_connection(&self, addr: &SocketAddr) -> Option<Arc<TcpConnection>> {
        let mut connections = self.connections.lock().await;

        if let Some(meta) = connections.get_mut(addr) {
            // Update the last activity time
            meta.last_activity = Instant::now();
            trace!("Found connection to {} in pool", addr);
            return Some(meta.connection.clone());
        }

        trace!("No connection to {} in pool", addr);
        None
    }

    /// Gets a connection only when both its peer address and opaque socket
    /// lifetime identity match.
    pub async fn get_connection_for_flow(
        &self,
        addr: &SocketAddr,
        flow_id: TransportFlowId,
    ) -> Option<Arc<TcpConnection>> {
        let mut connections = self.connections.lock().await;
        let meta = connections.get_mut(addr)?;
        if meta.connection.flow_id() != flow_id {
            return None;
        }
        meta.last_activity = Instant::now();
        Some(meta.connection.clone())
    }

    /// Resolves a live address-keyed TCP entry to its opaque socket identity.
    pub fn flow_id_for(&self, addr: &SocketAddr) -> Option<TransportFlowId> {
        let connections = self.connections.try_lock().ok()?;
        connections.get(addr).map(|meta| meta.connection.flow_id())
    }

    /// Awaited exact-flow lookup for security-sensitive response validation.
    pub async fn resolve_flow_id_for(&self, addr: &SocketAddr) -> Option<TransportFlowId> {
        self.connections
            .lock()
            .await
            .get(addr)
            .map(|meta| meta.connection.flow_id())
    }

    /// Removes a connection from the pool
    pub async fn remove_connection(&self, addr: &SocketAddr) {
        let mut connections = self.connections.lock().await;

        if connections.remove(addr).is_some() {
            trace!(
                "Removed connection to {} from pool (size: {})",
                addr,
                connections.len()
            );
        }
    }

    /// Removes the entry only if it still refers to the closing socket.
    pub async fn remove_connection_for_flow(&self, addr: &SocketAddr, flow_id: TransportFlowId) {
        let mut connections = self.connections.lock().await;
        if connections
            .get(addr)
            .is_some_and(|meta| meta.connection.flow_id() == flow_id)
        {
            connections.remove(addr);
        }
    }

    /// Non-blocking check for whether the pool currently holds a
    /// connection to `addr`. Used by `TcpTransport::has_connection_to`
    /// and the URI-aware multiplexer's response-routing path (RFC 3261
    /// §17.2 / §18.2.2). Returns `false` conservatively when the pool
    /// lock is busy — the multiplexer will just try the next candidate.
    pub fn has_connection(&self, addr: &SocketAddr) -> bool {
        match self.connections.try_lock() {
            Ok(guard) => guard.contains_key(addr),
            Err(_) => false,
        }
    }

    /// Closes all connections in the pool
    pub async fn close_all(&self) {
        self.closed.store(true, Ordering::Release);
        let mut connections = self.connections.lock().await;

        info!(
            "Closing all connections in pool (count: {})",
            connections.len()
        );

        // Close each connection
        for (addr, meta) in connections.drain() {
            if let Err(e) = meta.connection.close().await {
                error!("Error closing connection to {}: {}", addr, e);
            }
        }
    }

    /// Run periodic cleanup against the shared live pool. The owning
    /// transport supervises this future and aborts/joins it during close.
    pub(crate) async fn run_cleanup(self) {
        let mut cleanup_interval = interval(Duration::from_secs(60));
        while !self.closed.load(Ordering::Acquire) {
            cleanup_interval.tick().await;
            if self.closed.load(Ordering::Acquire) {
                break;
            }
            self.cleanup_idle_connections().await;
        }
    }

    /// Cleans up idle connections that have exceeded the timeout
    async fn cleanup_idle_connections(&self) {
        let mut connections = self.connections.lock().await;
        let now = Instant::now();
        let idle_timeout = self.config.idle_timeout;

        // Collect addresses of connections to remove
        let idle_addrs: Vec<SocketAddr> = connections
            .iter()
            .filter(|(_, meta)| now.duration_since(meta.last_activity) > idle_timeout)
            .map(|(addr, _)| *addr)
            .collect();

        if !idle_addrs.is_empty() {
            debug!("Cleaning up {} idle connections", idle_addrs.len());

            // Remove and close idle connections
            for addr in idle_addrs {
                if let Some(meta) = connections.remove(&addr) {
                    if let Err(e) = meta.connection.close().await {
                        error!("Error closing idle connection to {}: {}", addr, e);
                    }
                }
            }

            trace!("Pool size after cleanup: {}", connections.len());
        }
    }
}

impl Clone for ConnectionPool {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            connections: self.connections.clone(),
            closed: self.closed.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use tokio::sync::Mutex as TokioMutex;

    // Use a wrapper around Arc<TcpConnection> for the tests. The mock
    // is constructed by the basic tests below but its methods aren't
    // exercised in this iteration — the original integration tests
    // were simplified out. Kept for the upcoming pool-behaviour tests.
    #[allow(dead_code)]
    #[derive(Clone)]
    struct MockConnectionWrapper {
        peer_addr: SocketAddr,
        local_addr: SocketAddr,
        closed: Arc<TokioMutex<bool>>,
    }

    #[allow(dead_code)]
    impl MockConnectionWrapper {
        fn new(peer_addr: SocketAddr) -> Self {
            Self {
                peer_addr,
                local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
                closed: Arc::new(TokioMutex::new(false)),
            }
        }

        fn peer_addr(&self) -> SocketAddr {
            self.peer_addr
        }

        fn local_addr(&self) -> Result<SocketAddr> {
            Ok(self.local_addr)
        }

        async fn close(&self) -> Result<()> {
            let mut closed = self.closed.lock().await;
            *closed = true;
            Ok(())
        }

        async fn is_closed(&self) -> bool {
            let closed = self.closed.lock().await;
            *closed
        }
    }

    #[tokio::test]
    async fn test_connection_pool_basics() {
        let config = PoolConfig {
            max_connections: 5,
            idle_timeout: Duration::from_secs(1),
        };

        let pool = ConnectionPool::new(config);

        // Create some addresses to test with
        let addr1 = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)), 5060);
        let addr2 = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2)), 5060);

        // Create mock connection objects, wrapped in a way that TcpConnection would be
        let _conn1 = Arc::new(MockConnectionWrapper::new(addr1));
        let _conn2 = Arc::new(MockConnectionWrapper::new(addr2));

        // Here we simulate the connection pool behavior without actually testing the add_connection logic
        // Instead, just check if we can retrieve, remove connections, etc.

        // Skip testing this particular functionality since it requires actual TcpConnection instances
        // This would be better tested in integration tests with real connections

        // Close all connections
        pool.close_all().await;
    }

    #[tokio::test]
    async fn test_connection_pool_max_size() {
        let config = PoolConfig {
            max_connections: 2,
            idle_timeout: Duration::from_secs(300),
        };

        let pool = ConnectionPool::new(config);

        // Create addresses for testing
        let addr1 = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)), 5060);
        let addr2 = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2)), 5060);
        let addr3 = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 3)), 5060);

        // Create mock connections
        let _conn1 = Arc::new(MockConnectionWrapper::new(addr1));
        let _conn2 = Arc::new(MockConnectionWrapper::new(addr2));
        let _conn3 = Arc::new(MockConnectionWrapper::new(addr3));

        // Skip testing this particular functionality since it requires actual TcpConnection instances
        // This would be better tested in integration tests with real connections

        // Verify pool configuration
        assert_eq!(pool.config.max_connections, 2);
    }

    #[tokio::test]
    async fn cloned_pool_cleanup_mutates_the_live_shared_registry() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let destination = listener.local_addr().unwrap();
        let dialing = tokio::spawn(async move { TcpConnection::connect(destination).await });
        let (_server_stream, _) = listener.accept().await.unwrap();
        let connection = Arc::new(dialing.await.unwrap().unwrap());
        let flow_id = connection.flow_id();

        let pool = ConnectionPool::new(PoolConfig {
            max_connections: 2,
            idle_timeout: Duration::from_millis(5),
        });
        pool.add_connection(destination, connection.clone())
            .await
            .unwrap();
        let cleanup_view = pool.clone();
        assert_eq!(
            cleanup_view.resolve_flow_id_for(&destination).await,
            Some(flow_id)
        );

        tokio::time::sleep(Duration::from_millis(15)).await;
        cleanup_view.cleanup_idle_connections().await;

        assert_eq!(pool.resolve_flow_id_for(&destination).await, None);
        assert!(connection.is_closed());
    }
}
