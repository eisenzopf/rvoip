//! Strongly-typed identifiers — the canonical home post-V2.A.
//!
//! Each ID is a newtype around a UUID-shaped string so cross-crate
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

id_type!(ConversationId, "conv");
id_type!(SessionId, "sess");
id_type!(ConnectionId, "conn");
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

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn identifier_debug_never_discloses_correlation_values() {
        const CANARY: &str = "id-diagnostic-canary\r\nAuthorization: exposed";
        let ids = [
            format!("{:?}", ConversationId::from_string(CANARY)),
            format!("{:?}", SessionId::from_string(CANARY)),
            format!("{:?}", ConnectionId::from_string(CANARY)),
            format!("{:?}", StreamId::from_string(CANARY)),
            format!("{:?}", MessageId::from_string(CANARY)),
            format!("{:?}", TenantId::from_string(CANARY)),
        ];
        for debug in ids {
            assert!(!debug.contains(CANARY));
            assert!(debug.contains("[redacted]"));
        }
        assert_eq!(SessionId::from_string(CANARY).to_string(), CANARY);
    }
}
