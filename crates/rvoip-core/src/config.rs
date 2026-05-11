use std::sync::Arc;
use std::thread;

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
        }
    }
}
