//! `capability.advertise` payload per CONVERSATION_PROTOCOL.md §8.4.
//!
//! The descriptor body itself lives in `crate::capability::UctpCapabilityDescriptor`
//! (filled in PR 3); this payload struct only carries the routing fields
//! and the typed descriptor.

use serde::{Deserialize, Serialize};
use std::fmt;

/// `capability.advertise` (bidi) payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct CapabilityAdvertise {
    pub by_participant: String,
    pub capabilities: serde_json::Value,
    pub trigger: String,
}

impl fmt::Debug for CapabilityAdvertise {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CapabilityAdvertise")
    }
}
