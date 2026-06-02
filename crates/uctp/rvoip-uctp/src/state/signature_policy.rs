//! Gap plan §5.2 v1 punch list — coordinator-side signature policy.
//!
//! [`Sig9421Policy`] tells the coordinator's dispatch gate which
//! envelope `MessageType`s **require** an inline RFC 9421 signature
//! (per CONVERSATION_PROTOCOL.md §5.5.1). The policy is opt-in: it
//! only takes effect on coordinators built via
//! `UctpCoordinator::start_full_with_sig9421`.
//!
//! Decision matrix at dispatch time:
//!
//! | `env.signature` | `policy.requires(env.msg_type)` | outcome                                                 |
//! |-----------------|---------------------------------|---------------------------------------------------------|
//! | Some(_)         | any                             | verify; reject with `401-1 invalid-signature` on fail   |
//! | None            | false                           | pass — type does not require a signature                |
//! | None            | true                            | reject with `401-1 signature-required`                  |

use std::collections::HashSet;

use crate::types::MessageType;

/// Per-deployment policy: which envelope `MessageType`s **must** carry
/// an inline RFC 9421 signature.
#[derive(Clone, Debug, Default)]
pub struct Sig9421Policy {
    required_types: HashSet<MessageType>,
}

impl Sig9421Policy {
    /// Empty policy — no `MessageType` is mandatory-signed. Envelopes
    /// with a `signature` field still get verified; envelopes without
    /// one always pass.
    pub fn opportunistic() -> Self {
        Self::default()
    }

    /// Most conservative policy: only the auth handshake envelopes
    /// (`auth.hello`, `auth.response`) require signatures. Useful when
    /// rolling out RFC 9421 incrementally without breaking existing
    /// in-call traffic.
    pub fn auth_envelopes_only() -> Self {
        let mut required = HashSet::new();
        required.insert(MessageType::AuthHello);
        required.insert(MessageType::AuthResponse);
        Self {
            required_types: required,
        }
    }

    /// Require signatures on every post-session control envelope. The
    /// auth handshake is intentionally exempt — peers couldn't sign a
    /// `auth.hello` before keys are exchanged.
    pub fn all_post_session_types() -> Self {
        let mut required = HashSet::new();
        required.insert(MessageType::SessionInvite);
        required.insert(MessageType::SessionAccept);
        required.insert(MessageType::SessionCancel);
        required.insert(MessageType::SessionEnd);
        required.insert(MessageType::ConnectionOffer);
        required.insert(MessageType::ConnectionAnswer);
        required.insert(MessageType::ConnectionReady);
        required.insert(MessageType::ConnectionEnd);
        required.insert(MessageType::ConnectionUpdate);
        required.insert(MessageType::StreamSubscribe);
        required.insert(MessageType::StreamUnsubscribe);
        required.insert(MessageType::DtmfSend);
        required.insert(MessageType::AuthRefresh);
        Self {
            required_types: required,
        }
    }

    /// Add a `MessageType` to the required set.
    pub fn with_required(mut self, msg_type: MessageType) -> Self {
        self.required_types.insert(msg_type);
        self
    }

    /// True if `msg_type` requires an inline signature under this
    /// policy.
    pub fn requires(&self, msg_type: &MessageType) -> bool {
        self.required_types.contains(msg_type)
    }
}
