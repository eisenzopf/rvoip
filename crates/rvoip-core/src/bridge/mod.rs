//! Cross-transport bridge primitive.
//!
//! Per CARVE_PLAN §3 (BridgeManager row): in this PR series the stored handle
//! stays as `media-core`'s [`BridgeHandle`] — current shape, just relocated.
//! A `BridgeKind` enum (to discriminate SIP fast-path vs. cross-transport
//! bridge) is deferred until the cross-transport frame-pump (INTERFACE_DESIGN
//! §10.2) actually lands.
//!
//! The Phase-1 DashMap-of-bridges + by-owner-index shape from
//! `PERFORMANCE_PLAN.md` is preserved exactly. [`BridgeManager`] is generic
//! over the owner-key type so `orchestration-core` can keep using `CallId`
//! while a future cross-transport orchestrator uses [`crate::ConnectionId`].

use crate::ids::{BridgeId, ConnectionId};
use dashmap::DashMap;
use std::hash::Hash;
use std::sync::Arc;

pub use rvoip_media_core::relay::controller::{BridgeError, BridgeHandle};

pub mod cross_handle;
pub mod frame_pump;

pub use cross_handle::CrossBridgeHandle;

/// Map an `audio_codecs` codec name (per CONVERSATION_PROTOCOL.md §8)
/// to its standard RTP payload type.
///
/// Returns `None` for codec names not in the table so the bridge layer
/// can produce a clear "unsupported codec" diagnostic
/// ([`crate::RvoipError::UnsupportedCodec`]) instead of forwarding an
/// arbitrary dynamic PT (e.g. `96`) and getting a generic transcoder
/// error several layers down.
pub fn codec_to_pt(name: &str) -> Option<u8> {
    match name.to_ascii_lowercase().as_str() {
        "pcmu" | "g.711-mu" | "g711-mu" | "g711-u" => Some(0),
        "pcma" | "g.711-a" | "g711-a" => Some(8),
        "g729" | "g.729" => Some(18),
        "opus" => Some(111),
        _ => None,
    }
}

/// Per-process registry of active media bridges.
///
/// Backed by `DashMap` for lock-free concurrent reads/writes plus a secondary
/// `by_owner` index for O(1) "which bridge is this owner in?" lookups (used
/// by hangup / teardown paths). The primary map stores the handle alongside
/// the owning key so [`BridgeManager::remove`] can keep both indices coherent
/// without an extra parameter.
///
/// Generic parameters:
/// - `K`: owner key (e.g. `ConnectionId` for cross-transport, `CallId` for
///   SIP-only orchestration-core).
/// - `I`: bridge identifier type (defaults to rvoip-core's
///   [`BridgeId`]; orchestration-core overrides with its own `BridgeId` for
///   the carve transition).
pub struct BridgeManager<K = ConnectionId, I = BridgeId>
where
    K: Eq + Hash + Clone,
    I: Eq + Hash + Clone,
{
    bridges: Arc<DashMap<I, (BridgeHandle, K)>>,
    by_owner: Arc<DashMap<K, I>>,
}

impl<K, I> Clone for BridgeManager<K, I>
where
    K: Eq + Hash + Clone,
    I: Eq + Hash + Clone,
{
    fn clone(&self) -> Self {
        Self {
            bridges: Arc::clone(&self.bridges),
            by_owner: Arc::clone(&self.by_owner),
        }
    }
}

impl<K, I> Default for BridgeManager<K, I>
where
    K: Eq + Hash + Clone,
    I: Eq + Hash + Clone,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<K, I> BridgeManager<K, I>
where
    K: Eq + Hash + Clone,
    I: Eq + Hash + Clone,
{
    pub fn new() -> Self {
        Self {
            bridges: Arc::new(DashMap::new()),
            by_owner: Arc::new(DashMap::new()),
        }
    }

    pub fn insert(&self, bridge_id: I, owner: K, handle: BridgeHandle) {
        self.by_owner.insert(owner.clone(), bridge_id.clone());
        self.bridges.insert(bridge_id, (handle, owner));
    }

    /// Removes the bridge and returns the [`BridgeHandle`] (whose `Drop`
    /// tears the bridge down). Keeps the secondary index coherent.
    pub fn remove(&self, bridge_id: &I) -> Option<BridgeHandle> {
        let (_, (handle, owner)) = self.bridges.remove(bridge_id)?;
        self.by_owner
            .remove_if(&owner, |_, registered| registered == bridge_id);
        Some(handle)
    }

    /// O(1) lookup: which active bridge (if any) is this owner in?
    pub fn bridge_for_owner(&self, owner: &K) -> Option<I> {
        self.by_owner.get(owner).map(|entry| entry.value().clone())
    }

    pub fn len(&self) -> usize {
        self.bridges.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bridges.is_empty()
    }
}
