use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::store::{ConversationStore, MemoryConversationStore, MemoryVconStore, VconStore};

/// Orchestrator configuration.
///
/// Phase-2 admission semaphore default per `PERFORMANCE_PLAN.md`:
/// `max_concurrent_setups = 256 * available_parallelism()`.
#[derive(Clone)]
pub struct Config {
    pub max_concurrent_setups: usize,
    pub conversation_store: Arc<dyn ConversationStore>,
    pub vcon_store: Arc<dyn VconStore>,
    /// How long `bridge_connections` waits for both peers' audio streams
    /// to appear before failing the admission check. Adapters populate
    /// streams lazily (typically on `connection.ready`), so a caller
    /// that triggers a bridge from `Event::ConnectionInbound` may race
    /// the stream registration. Setting this to zero disables the wait
    /// (strict legacy behavior). Default: 2 seconds.
    pub bridge_stream_deadline: Duration,
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
            bridge_stream_deadline: Duration::from_secs(2),
        }
    }
}
