//! Auth envelope payloads per CONVERSATION_PROTOCOL.md §5.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

/// `auth.hello` (C→S) payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct AuthHello {
    pub device: Device,
    pub auth_methods: Vec<String>,
    /// CapabilityDescriptor JSON — typed at the negotiation layer.
    #[serde(default)]
    pub capabilities: serde_json::Value,
}

impl fmt::Debug for AuthHello {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthHello")
            .field("device", &self.device)
            .field("auth_method_count", &self.auth_methods.len())
            .field("capability_kind", &json_kind(&self.capabilities))
            .finish()
    }
}

/// `auth.challenge` (S→C) payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct AuthChallenge {
    pub nonce: String,
    pub accepted_methods: Vec<String>,
    #[serde(default)]
    pub server_capabilities: serde_json::Value,
}

impl fmt::Debug for AuthChallenge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthChallenge")
            .field("nonce_present", &!self.nonce.is_empty())
            .field("nonce_bytes", &self.nonce.len())
            .field("accepted_method_count", &self.accepted_methods.len())
            .field("capability_kind", &json_kind(&self.server_capabilities))
            .finish()
    }
}

/// `auth.response` (C→S) payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    /// Auth method name (`bearer`, `oauth2-dpop`, `passkey`, `aauth`, ...).
    pub method: String,
    /// Opaque credential body. Shape depends on `method`. For
    /// `method = "aauth"` this is the subject token.
    pub credential: String,
    /// Actor token, present only for AAuth (`method = "aauth"`). The
    /// actor token identifies the agent (bot, assistant, service)
    /// acting on behalf of the subject; the combined pair maps to
    /// `IdentityAssurance::UserAuthorized { user_id: subject,
    /// identity: actor }`. See CONVERSATION_PROTOCOL.md §5.6 and
    /// `rvoip_auth_core::aauth`. Gap plan §5.1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_token: Option<String>,
}

impl fmt::Debug for AuthResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthResponse")
            .field("method_present", &!self.method.is_empty())
            .field("method_bytes", &self.method.len())
            .field("credential_present", &!self.credential.is_empty())
            .field("credential_bytes", &self.credential.len())
            .field("actor_token_present", &self.actor_token.is_some())
            .field(
                "actor_token_bytes",
                &self.actor_token.as_ref().map_or(0, String::len),
            )
            .finish()
    }
}

/// `auth.session` (S→C) payload.
#[derive(Clone, Serialize, Deserialize)]
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

impl fmt::Debug for AuthSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthSession")
            .field("identity_present", &!self.identity_id.is_empty())
            .field("participant_present", &!self.participant_id.is_empty())
            .field("session_token_present", &!self.session_token.is_empty())
            .field("session_token_bytes", &self.session_token.len())
            .field("expires_at_present", &true)
            .field("assurance_present", &!self.assurance.is_empty())
            .field("assurance_bytes", &self.assurance.len())
            .field("reachability_count", &self.reachability.len())
            .finish()
    }
}

/// `auth.keepalive` (C→S, periodic) payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct AuthKeepalive {
    pub session_token: String,
}

impl fmt::Debug for AuthKeepalive {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthKeepalive")
            .field("session_token_present", &!self.session_token.is_empty())
            .field("session_token_bytes", &self.session_token.len())
            .finish()
    }
}

/// `auth.bye` (bidi) payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct AuthBye {
    pub reason: String,
}

impl fmt::Debug for AuthBye {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthBye")
            .field("reason_present", &!self.reason.is_empty())
            .field("reason_bytes", &self.reason.len())
            .finish()
    }
}

/// `auth.refresh` (C→S) payload — plan D4. Sent by the peer before its
/// current bearer token expires; the coordinator validates the new
/// credential and, on success, updates `PeerAuthState` and replies
/// with a fresh `auth.session` envelope. On validation failure the
/// existing session is preserved (the peer can retry until the old
/// token's `expires_at` actually passes).
#[derive(Clone, Serialize, Deserialize)]
pub struct AuthRefresh {
    /// Auth method name — typically matches whatever was used at the
    /// initial `auth.response` (`bearer`, `oauth2-dpop`, ...). The
    /// coordinator routes to the same validator either way.
    pub method: String,
    /// The new credential body. Replaces the prior one on success.
    pub credential: String,
    /// Replacement actor token when `method` is `aauth`. Both credentials are
    /// validated as a pair and are discarded immediately after validation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_token: Option<String>,
}

impl fmt::Debug for AuthRefresh {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthRefresh")
            .field("method_present", &!self.method.is_empty())
            .field("method_bytes", &self.method.len())
            .field("credential_present", &!self.credential.is_empty())
            .field("credential_bytes", &self.credential.len())
            .field("actor_token_present", &self.actor_token.is_some())
            .field(
                "actor_token_bytes",
                &self.actor_token.as_ref().map_or(0, String::len),
            )
            .finish()
    }
}

/// Device descriptor sent in `auth.hello`.
#[derive(Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub kind: String,
    pub platform: String,
    pub sdk_version: String,
}

impl fmt::Debug for Device {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Device")
            .field("id_present", &!self.id.is_empty())
            .field("id_bytes", &self.id.len())
            .field("kind_bytes", &self.kind.len())
            .field("platform_bytes", &self.platform.len())
            .field("sdk_version_bytes", &self.sdk_version.len())
            .finish()
    }
}

/// One reachability hint advertised in `auth.session`.
///
/// CONVERSATION_PROTOCOL.md §5.3.
#[derive(Clone, Serialize, Deserialize)]
pub struct ReachabilityHint {
    pub transport: String,
    pub address: String,
    pub expires_at: DateTime<Utc>,
    /// Lower = preferred.
    pub priority: u32,
    pub device_id: String,
}

impl fmt::Debug for ReachabilityHint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReachabilityHint")
            .field("transport_bytes", &self.transport.len())
            .field("address_present", &!self.address.is_empty())
            .field("address_bytes", &self.address.len())
            .field("expires_at_present", &true)
            .field("priority", &self.priority)
            .field("device_id_present", &!self.device_id.is_empty())
            .field("device_id_bytes", &self.device_id.len())
            .finish()
    }
}

fn json_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}
