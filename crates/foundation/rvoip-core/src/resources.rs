//! Typed snapshots for application diagnostics and lifecycle assertions.
//! Counts come from ownership registries without connection, tenant, asset,
//! target, credential, or other peer-controlled labels.
pub use rvoip_core_traits::resources::*;
use serde::{Deserialize, Serialize};

/// A single registry partitioned by terminal state. `live` includes setup
/// and ending states; `retained_terminal` is ended/failed history still held
/// in memory. The sum is the retained registry size observed during the scan.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LifecycleResourceCounts {
    pub live: usize,
    pub retained_terminal: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CoreResourceCounts {
    pub conversations: LifecycleResourceCounts,
    pub sessions: LifecycleResourceCounts,
    /// Current routing entries, including connecting and ending routes.
    pub connection_routes: usize,
    /// All retained lifecycle identities, including active identities.
    pub retained_connection_ids: usize,
    /// Retained lifecycle identities already marked retired (a subset of
    /// `retained_connection_ids`), distinct from live routing entries. What a
    /// retired row means depends on the lifecycle retention mode: in
    /// compatibility mode it is a permanent anti-reuse tombstone, while in
    /// bounded lifecycle mode it is a retired row still awaiting reclaim
    /// (media shutdown or adapter cleanup), after which it leaves the count.
    pub retired_connection_ids: usize,
    /// Adapter cleanup failures/unconfirmed terminal compensation.
    pub adapter_cleanup_quarantines: usize,
    pub media_graphs: usize,
    pub cross_bridges: usize,
    pub recordings: usize,
    pub ai_attachments: usize,
    pub direct_listeners: usize,
    /// Prepared outbound admission reservations, including pending cleanup.
    pub prepared_outbound_reservations: usize,
    pub lifecycle_workers: usize,
    pub periodic_workers: usize,
}

/// A best-effort observation. Registries and adapters are read at different
/// instants; this is not atomic across them. Missing measurements stay
/// unavailable rather than becoming zero. Counts are not a complete memory
/// or task inventory.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ResourceSnapshot {
    pub core: CoreResourceCounts,
    pub adapters: Vec<AdapterResourceSnapshot>,
}
