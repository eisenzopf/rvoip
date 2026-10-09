//! Identifier-free resource observations. These counts describe local
//! ownership, not peer state, historical totals, or a transactional census.
use crate::connection::Transport;
use serde::{Deserialize, Serialize};

/// Counts of explicitly supervised task categories. `None` means this
/// adapter does not measure that category; `Some(0)` is a measured zero.
/// Categories are separate ownership registries and must not be summed to
/// infer a complete process task count.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AdapterTaskCounts {
    pub retained_workers: Option<usize>,
    pub peer_workers: Option<usize>,
    pub media_workers: Option<usize>,
    pub http_workers: Option<usize>,
    pub inbound_ws_workers: Option<usize>,
    pub outbound_signaling_workers: Option<usize>,
    pub outbound_ws_hub_workers: Option<usize>,
    pub inbound_admission_workers: Option<usize>,
}

/// Optional measurements from an adapter's local ownership registries.
/// Entries may include provisional or terminating owners until cleanup
/// removes them. They do not imply the connection is established.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AdapterResourceCounts {
    pub registered_connections: Option<usize>,
    pub outbound_owners: Option<usize>,
    pub media_streams: Option<usize>,
    /// Locally owned UDP media sockets/allocator reservations.
    pub allocated_media_ports: Option<usize>,
    pub http_resources: Option<usize>,
    pub tasks: AdapterTaskCounts,
}

/// Fixed statuses deliberately carry no adapter error text or identifiers.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", content = "counts", rename_all = "snake_case")]
#[non_exhaustive]
pub enum AdapterResourceObservation {
    Reported(AdapterResourceCounts),
    Unsupported,
    TimedOut,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AdapterResourceSnapshot {
    pub transport: Transport,
    pub observation: AdapterResourceObservation,
}

impl AdapterResourceSnapshot {
    #[must_use]
    pub fn new(transport: Transport, observation: AdapterResourceObservation) -> Self {
        Self {
            transport,
            observation,
        }
    }
}
