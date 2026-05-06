use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! id_type {
    ($name:ident, $prefix:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub struct $name(pub String);

        impl $name {
            pub fn new() -> Self {
                Self(format!("{}-{}", $prefix, uuid::Uuid::new_v4()))
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_type!(CallId, "call");
id_type!(CallLegId, "leg");
id_type!(AgentId, "agent");
id_type!(QueueId, "queue");
id_type!(AgentOfferId, "offer");
id_type!(ReservationId, "reservation");
id_type!(VoiceAiId, "voice-ai");
id_type!(VoiceAiSessionId, "voice-ai-session");
id_type!(BridgeId, "bridge");
id_type!(RecordingId, "recording");
id_type!(TranscriptId, "transcript");
id_type!(DialogSessionId, "dialog");
id_type!(EventId, "event");

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CallPriority(pub u8);

impl CallPriority {
    pub const HIGHEST: Self = Self(0);
    pub const NORMAL: Self = Self(5);
    pub const LOWEST: Self = Self(255);
}

impl Default for CallPriority {
    fn default() -> Self {
        Self::NORMAL
    }
}

impl From<u8> for CallPriority {
    fn from(value: u8) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Skill(pub String);

impl From<String> for Skill {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for Skill {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl AsRef<str> for Skill {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
