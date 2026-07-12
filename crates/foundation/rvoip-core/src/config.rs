use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::store::{
    ConversationStore, MemoryConversationStore, MemoryMessageStore, MemoryVconStore, MessageStore,
    VconStore,
};

/// P6 — per-tenant quota envelope. Each `None` means "unlimited".
#[derive(Clone, Copy, Debug, Default)]
pub struct TenantQuotas {
    pub max_concurrent_sessions: Option<usize>,
    pub max_concurrent_recordings: Option<usize>,
    pub max_concurrent_ai_sessions: Option<usize>,
}

/// Orchestrator configuration.
///
/// Phase-2 admission semaphore default per `PERFORMANCE_PLAN.md`:
/// `max_concurrent_setups = 256 * available_parallelism()`.
#[derive(Clone)]
pub struct Config {
    pub max_concurrent_setups: usize,
    pub conversation_store: Arc<dyn ConversationStore>,
    pub vcon_store: Arc<dyn VconStore>,
    /// P4 — message log + history pager. Default in-memory.
    pub message_store: Arc<dyn MessageStore>,
    /// How long `bridge_connections` waits for both peers' audio streams
    /// to appear before failing the admission check. Adapters populate
    /// streams lazily (typically on `connection.ready`), so a caller
    /// that triggers a bridge from `Event::ConnectionInbound` may race
    /// the stream registration. Setting this to zero disables the wait
    /// (strict legacy behavior).
    ///
    /// **Default raised to 5 seconds** (plan §7 architectural concern
    /// #7 / D3): the previous 2 seconds was tight for cold WebTransport
    /// dials on a high-latency mobile link, where the TLS + HTTP/3 +
    /// extended-CONNECT handshake routinely exceeds 2s. 5s covers
    /// realistic mobile network jitter without holding admission for
    /// pathologically dead peers.
    pub bridge_stream_deadline: Duration,
    /// Maximum time an outbound route may remain prepared but uncommitted.
    ///
    /// A prepared route is deliberately invisible to Sessions and event
    /// consumers while an application durably records its Connection ID.
    /// Core aborts and closes the provisional adapter route when this
    /// deadline expires. A finite default prevents abandoned durable-bind
    /// attempts from retaining adapter or admission capacity indefinitely.
    pub outbound_preparation_timeout: Duration,
    /// P6 — `Event::CapacityReport` emit cadence. None disables the
    /// scheduler entirely.
    pub capacity_report_interval: Option<Duration>,
}

impl Default for Config {
    fn default() -> Self {
        let cpus = thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        Self {
            max_concurrent_setups: 256 * cpus,
            conversation_store: Arc::new(MemoryConversationStore::new()),
            vcon_store: Arc::new(MemoryVconStore::new()),
            message_store: Arc::new(MemoryMessageStore::new()),
            bridge_stream_deadline: Duration::from_secs(5),
            outbound_preparation_timeout: Duration::from_secs(30),
            capacity_report_interval: Some(Duration::from_secs(30)),
        }
    }
}
