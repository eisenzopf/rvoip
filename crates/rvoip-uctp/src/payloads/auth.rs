//! Auth envelope payloads per CONVERSATION_PROTOCOL.md §5.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// `auth.hello` (C→S) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthHello {
    pub device: Device,
    pub auth_methods: Vec<String>,
    /// CapabilityDescriptor JSON — typed at the negotiation layer.
    #[serde(default)]
    pub capabilities: serde_json::Value,
}

/// `auth.challenge` (S→C) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthChallenge {
    pub nonce: String,
    pub accepted_methods: Vec<String>,
    #[serde(default)]
    pub server_capabilities: serde_json::Value,
}

/// `auth.response` (C→S) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthResponse {
    /// Auth method name (`bearer`, `oauth2-dpop`, `passkey`, ...).
    pub method: String,
    /// Opaque credential body. Shape depends on `method`.
    pub credential: String,
}

/// `auth.session` (S→C) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthSession {
    pub identity_id: String,
    pub participant_id: String,
    /// Opaque server-issued token; used for reconnect.
    pub session_token: String,
    pub expires_at: DateTime<Utc>,
    /// IdentityAssurance level (§5.6) — serialized as the kebab-case
    /// wire string. The state machine maps this to
    /// `rvoip_core::IdentityAssurance` via a typed deserializer.
    pub assurance: String,
    #[serde(default)]
    pub reachability: Vec<ReachabilityHint>,
}

/// `auth.keepalive` (C→S, periodic) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthKeepalive {
    pub session_token: String,
}

/// `auth.bye` (bidi) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthBye {
    pub reason: String,
}

/// `auth.refresh` (C→S) payload — plan D4. Sent by the peer before its
/// current bearer token expires; the coordinator validates the new
/// credential and, on success, updates `PeerAuthState` and replies
/// with a fresh `auth.session` envelope. On validation failure the
/// existing session is preserved (the peer can retry until the old
/// token's `expires_at` actually passes).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthRefresh {
    /// Auth method name — typically matches whatever was used at the
    /// initial `auth.response` (`bearer`, `oauth2-dpop`, ...). The
    /// coordinator routes to the same validator either way.
    pub method: String,
    /// The new credential body. Replaces the prior one on success.
    pub credential: String,
}

/// Device descriptor sent in `auth.hello`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub kind: String,
    pub platform: String,
    pub sdk_version: String,
}

/// One reachability hint advertised in `auth.session`.
///
/// CONVERSATION_PROTOCOL.md §5.3.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReachabilityHint {
    pub transport: String,
    pub address: String,
    pub expires_at: DateTime<Utc>,
    /// Lower = preferred.
    pub priority: u32,
    pub device_id: String,
}
