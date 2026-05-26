//! Stream envelope payloads per CONVERSATION_PROTOCOL.md §7.4 + §7.7.
//!
//! v0 parses subscribe/unsubscribe/active-speaker so the wire format is
//! stable; the routing implementation lands in rvoip-core in v0.x. See
//! `UCTP_IMPLEMENTATION_PLAN.md` §7 ("Known tensions") and §1.4.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// `stream.opened` (S→C) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StreamOpened {
    pub stream: StreamInfo,
}

/// `stream.closed` (S→C) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StreamClosed {
    pub strm_id: String,
    pub closed_at: DateTime<Utc>,
    pub reason_code: u16,
    pub reason: String,
}

/// `stream.subscribe` (bidi) payload.
///
/// Multi-party: v0 parses but does not route. Receiving servers return
/// `error` 501 `multi-party-routing-not-implemented` per
/// CONVERSATION_PROTOCOL.md §11.2 and `UCTP_IMPLEMENTATION_PLAN.md` §7.
/// (Pre-v0.x servers used 503 here; v0.x distinguishes 501 from 503.)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StreamSubscribe {
    pub by_participant: String,
    pub subscriptions: Vec<StreamSubscription>,
}

/// `stream.unsubscribe` (bidi) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StreamUnsubscribe {
    pub strm_ids: Vec<String>,
}

/// `stream.active-speaker` (S→C, advisory) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StreamActiveSpeaker {
    #[serde(default)]
    pub active_participant: Option<String>,
    pub strm_id: String,
    pub changed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StreamInfo {
    pub strm_id: String,
    pub kind: String,
    pub codec: serde_json::Value,
    pub direction: String,
    pub stream_local_id: u16,
    pub opened_at: DateTime<Utc>,
}

/// A subscription targets either a specific stream, or all streams from
/// a participant (optionally filtered by kind). The three shapes from
/// the spec map to these three optional fields; consumers check which
/// is set.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StreamSubscription {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub strm_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub from_participant: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<String>,
}
