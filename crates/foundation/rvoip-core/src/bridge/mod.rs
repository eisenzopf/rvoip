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

/// Enabled media directions for one two-Connection bridge.
///
/// The names are relative to the exact `a` and `b` Connection arguments. A
/// disabled direction neither acquires that source's single-consumer receiver
/// nor installs a sink route. Application-data bridging remains independent
/// and bidirectional.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectionalMediaBridgePlan {
    a_to_b: bool,
    b_to_a: bool,
}

impl DirectionalMediaBridgePlan {
    /// Build a validated directional plan. A bridge with no media direction
    /// is rejected rather than reserving both Connections without a route.
    pub fn new(a_to_b: bool, b_to_a: bool) -> crate::Result<Self> {
        if !a_to_b && !b_to_a {
            return Err(crate::RvoipError::AdmissionRejected(
                "media bridge must enable at least one direction",
            ));
        }
        Ok(Self { a_to_b, b_to_a })
    }

    #[must_use]
    pub const fn bidirectional() -> Self {
        Self {
            a_to_b: true,
            b_to_a: true,
        }
    }

    #[must_use]
    pub const fn a_to_b(self) -> bool {
        self.a_to_b
    }

    #[must_use]
    pub const fn b_to_a(self) -> bool {
        self.b_to_a
    }
}

/// Map an `audio_codecs` codec name (per CONVERSATION_PROTOCOL.md §8)
/// to its media-graph codec key.
///
/// Most keys are the codec's conventional RTP payload type. `pcm_s16le`
/// uses the internal-only key 120; transports must not advertise or emit it.
///
/// Returns `None` for codec names not in the table so the bridge layer
/// can produce a clear "unsupported codec" diagnostic
/// ([`crate::RvoipError::UnsupportedCodec`]) instead of forwarding an
/// arbitrary dynamic PT (e.g. `96`) and getting a generic transcoder
/// error several layers down.
///
/// # AMR is absent by decision, not by oversight
///
/// AMR-NB and AMR-WB are implemented in `rvoip-codec-core`, reachable through
/// media-core, and carried end to end on the SIP media path — and they are
/// deliberately not in this table, so the media graph refuses them.
///
/// A name-to-key function cannot express AMR. One session routinely negotiates
/// the same AMR variant under two payload types that differ only in
/// `octet-align` (the SIP integration test uses 106 and 107), so there is no
/// single key to return; and the key this function produces is stamped onto
/// outgoing frames, so inventing one puts a wrong payload type on the wire.
/// AMR must key on its negotiated payload type instead, which means reaching
/// the graph through a payload-type-carrying entry point that does not exist
/// yet.
///
/// See `docs/MEDIA_GRAPH_CODECS.md` for the full decision and what wiring AMR
/// in would require. Do not add AMR here without reading it.
pub fn codec_to_pt(name: &str) -> Option<u8> {
    match name.to_ascii_lowercase().as_str() {
        "pcmu" | "g.711-mu" | "g711-mu" | "g711-u" => Some(0),
        "pcma" | "g.711-a" | "g711-a" => Some(8),
        "g729" | "g.729" => Some(18),
        "opus" => Some(111),
        "pcm_s16le" | "pcm-s16le" => Some(rvoip_media_core::codec::audio::payload_type::PCM_S16LE),
        _ => None,
    }
}

#[cfg(test)]
mod codec_mapping_tests {
    use super::codec_to_pt;
    use rvoip_media_core::codec::audio::payload_type::PCM_S16LE;

    #[test]
    fn maps_internal_pcm_name_to_reserved_key() {
        assert_eq!(codec_to_pt("pcm_s16le"), Some(PCM_S16LE));
        assert_eq!(codec_to_pt("PCM_S16LE"), Some(PCM_S16LE));
        assert_eq!(PCM_S16LE, 120);
    }

    /// AMR's absence is a decision, and this pins it so it cannot be undone by
    /// pattern-matching on the Opus row.
    ///
    /// Giving AMR a conventional key here would compile and would look like
    /// the `opus => 111` line above, but the key this function returns is
    /// stamped onto emitted frames, and AMR's payload type is negotiated per
    /// call — one session commonly carries the same variant at two payload
    /// types that differ only in `octet-align`. If AMR is ever wired into the
    /// media graph it must arrive with its negotiated payload type, which
    /// means a new entry point rather than a new row here.
    ///
    /// See `docs/MEDIA_GRAPH_CODECS.md` before changing this.
    #[test]
    fn amr_has_no_conventional_key_by_decision() {
        for name in ["AMR", "amr", "AMR-WB", "amr-wb"] {
            assert_eq!(
                codec_to_pt(name),
                None,
                "{name} must stay absent so the graph refuses it loudly \
                 rather than stamping a fabricated payload type on the wire"
            );
        }
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
