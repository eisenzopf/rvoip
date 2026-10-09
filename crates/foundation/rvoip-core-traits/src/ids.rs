//! Strongly-typed identifiers — the canonical home post-V2.A.
//!
//! Each ID holds an opaque, unique correlation string so cross-crate
//! consumers can pattern-match on the kind without confusing a
//! `SessionId` with a `ConnectionId`. `rvoip-core` re-exports this
//! whole module so `use rvoip_core::ids::ConnectionId` keeps working.

use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

macro_rules! id_type {
    ($name:ident, $prefix:expr) => {
        #[derive(Clone, Eq, Hash, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
        pub struct $name(pub String);

        impl $name {
            pub fn new() -> Self {
                Self(format!("{}_{}", $prefix, Uuid::new_v4().simple()))
            }

            pub fn from_string(s: impl Into<String>) -> Self {
                Self(s.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                // IDs are correlation material. Keep their functional Display
                // and wire forms intact, but make accidental structured-log
                // capture metadata-only.
                f.write_str(concat!(stringify!($name), "([redacted])"))
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

// Minted capabilities never cross a serde boundary. Correlation strings
// remain valid for lookups, but do not grant creation/revival authority.
struct LifecycleIdentityFence {
    state: std::sync::atomic::AtomicU8,
    cleanup_complete: std::sync::atomic::AtomicBool,
}

macro_rules! fenced_id_type {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone)]
        pub struct $name(String, Option<std::sync::Arc<LifecycleIdentityFence>>);

        impl $name {
            pub fn new() -> Self {
                use std::sync::{
                    atomic::{AtomicU64, Ordering},
                    OnceLock,
                };
                static INCARNATION: OnceLock<Uuid> = OnceLock::new();
                static SEQUENCE: AtomicU64 = AtomicU64::new(0);
                // Never wrap or reissue an identity, including on counter exhaustion.
                let sequence = SEQUENCE
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                    .expect("lifecycle identity sequence exhausted");
                Self(
                    format!(
                        "{}_{}_{sequence:016x}",
                        $prefix,
                        INCARNATION.get_or_init(Uuid::new_v4).simple()
                    ),
                    Some(std::sync::Arc::new(LifecycleIdentityFence {
                        state: std::sync::atomic::AtomicU8::new(0),
                        cleanup_complete: std::sync::atomic::AtomicBool::new(false),
                    })),
                )
            }

            pub fn from_string(s: impl Into<String>) -> Self {
                Self(s.into(), None)
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// SDK-internal capability: string/serde copies cannot forge this fence.
            #[doc(hidden)]
            pub fn has_lifecycle_fence(&self) -> bool {
                self.1.is_some()
            }

            /// Claim one freshly minted identity once across all orchestrators.
            #[doc(hidden)]
            pub fn claim_lifecycle(&self) -> bool {
                use std::sync::atomic::Ordering;
                self.1.as_ref().is_some_and(|f| {
                    f.state
                        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                })
            }

            /// Retirement follows every clone, including events still in a queue.
            #[doc(hidden)]
            pub fn retire_lifecycle(&self) {
                if let Some(f) = &self.1 {
                    f.state.store(2, std::sync::atomic::Ordering::Release);
                }
            }

            #[doc(hidden)]
            pub fn lifecycle_claimed(&self) -> bool {
                self.1
                    .as_ref()
                    .is_some_and(|f| f.state.load(std::sync::atomic::Ordering::Acquire) == 1)
            }

            #[doc(hidden)]
            pub fn lifecycle_retired(&self) -> bool {
                self.1
                    .as_ref()
                    .is_some_and(|f| f.state.load(std::sync::atomic::Ordering::Acquire) == 2)
            }

            #[doc(hidden)]
            pub fn complete_lifecycle_cleanup(&self) {
                if let Some(f) = &self.1 {
                    f.cleanup_complete
                        .store(true, std::sync::atomic::Ordering::Release);
                }
            }

            #[doc(hidden)]
            pub fn lifecycle_reclaimable(&self) -> bool {
                self.lifecycle_retired()
                    && self.1.as_ref().is_some_and(|f| {
                        f.cleanup_complete
                            .load(std::sync::atomic::Ordering::Acquire)
                    })
            }

            #[doc(hidden)]
            pub fn same_lifecycle_fence(&self, other: &Self) -> bool {
                match (&self.1, &other.1) {
                    (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
                    _ => false,
                }
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(concat!(stringify!($name), "([redacted])"))
            }
        }
        impl PartialEq for $name {
            fn eq(&self, other: &Self) -> bool {
                self.0 == other.0
            }
        }
        impl Eq for $name {}
        impl std::hash::Hash for $name {
            fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
                self.0.hash(state);
            }
        }
        impl PartialOrd for $name {
            fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }
        impl Ord for $name {
            fn cmp(&self, other: &Self) -> std::cmp::Ordering {
                self.0.cmp(&other.0)
            }
        }
        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.0)
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                String::deserialize(d).map(Self::from_string)
            }
        }
    };
}

fenced_id_type!(ConversationId, "conv");
id_type!(SessionId, "sess");
id_type!(AiSessionId, "aisess");
fenced_id_type!(ConnectionId, "conn");
id_type!(StreamId, "strm");
id_type!(MessageId, "msg");
id_type!(ParticipantId, "part");
id_type!(IdentityId, "id");
id_type!(DeviceId, "dev");
id_type!(BridgeId, "brdg");
id_type!(MediaRouteId, "route");
id_type!(TenantId, "tnt");
id_type!(RecordingId, "rec");
id_type!(ListenerId, "lstn");
id_type!(AttachmentId, "att");
id_type!(AiAttachmentId, "ai");
id_type!(PlaybackId, "play");
id_type!(TranscriptionId, "trn");
id_type!(TransferAttemptId, "xfer");

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn identifier_debug_never_discloses_correlation_values() {
        const CANARY: &str = "id-diagnostic-canary\r\nAuthorization: exposed";
        let ids = [
            format!("{:?}", ConversationId::from_string(CANARY)),
            format!("{:?}", SessionId::from_string(CANARY)),
            format!("{:?}", AiSessionId::from_string(CANARY)),
            format!("{:?}", ConnectionId::from_string(CANARY)),
            format!("{:?}", StreamId::from_string(CANARY)),
            format!("{:?}", MessageId::from_string(CANARY)),
            format!("{:?}", TenantId::from_string(CANARY)),
            format!("{:?}", TransferAttemptId::from_string(CANARY)),
        ];
        for debug in ids {
            assert!(!debug.contains(CANARY));
            assert!(debug.contains("[redacted]"));
        }
        assert_eq!(SessionId::from_string(CANARY).to_string(), CANARY);
    }
}

#[cfg(test)]
mod lifecycle_fence_tests {
    use super::*;
    #[test]
    fn serialization_preserves_text_but_cannot_clone_authority() {
        let id = ConnectionId::new();
        let queued = id.clone();
        let wire = serde_json::to_string(&id).unwrap();
        assert_eq!(wire, format!("\"{}\"", id.as_str()));
        let reconstructed: ConnectionId = serde_json::from_str(&wire).unwrap();
        assert_eq!(id, reconstructed);
        assert!(!reconstructed.has_lifecycle_fence());
        assert!(id.claim_lifecycle());
        assert!(!queued.claim_lifecycle());
        id.retire_lifecycle();
        assert!(queued.lifecycle_retired());
        assert!(!queued.lifecycle_reclaimable());
        id.complete_lifecycle_cleanup();
        assert!(queued.lifecycle_reclaimable());
        let weak = std::sync::Arc::downgrade(id.1.as_ref().unwrap());
        drop(id);
        drop(queued);
        assert!(weak.upgrade().is_none());
    }
    #[test]
    fn minted_identity_is_unique_across_concurrent_issuers() {
        let threads: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| {
                    (0..10_000)
                        .map(|_| ConnectionId::new().to_string())
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let ids: std::collections::HashSet<_> = threads
            .into_iter()
            .flat_map(|t| t.join().unwrap())
            .collect();
        assert_eq!(ids.len(), 80_000);
    }
}
