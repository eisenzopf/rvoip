//! `capability.advertise` payload per CONVERSATION_PROTOCOL.md §8.4.
//!
//! The descriptor body itself lives in `crate::capability::UctpCapabilityDescriptor`
//! (filled in PR 3); this payload struct only carries the routing fields
//! and the typed descriptor.

use serde::{Deserialize, Serialize};

/// `capability.advertise` (bidi) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CapabilityAdvertise {
    pub by_participant: String,
    pub capabilities: serde_json::Value,
    pub trigger: String,
}
