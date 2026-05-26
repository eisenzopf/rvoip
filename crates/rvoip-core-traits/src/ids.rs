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
        #[derive(Clone, Debug, Eq, Hash, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
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
id_type!(TenantId, "tnt");
id_type!(RecordingId, "rec");
id_type!(ListenerId, "lstn");
id_type!(AttachmentId, "att");
id_type!(AiAttachmentId, "ai");
id_type!(PlaybackId, "play");
id_type!(TranscriptionId, "trn");
