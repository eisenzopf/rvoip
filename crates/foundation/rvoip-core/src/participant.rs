use crate::ids::{ConversationId, IdentityId, ParticipantId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ParticipantKind {
    Human,
    Ai,
    System,
    External,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum ParticipantRole {
    Customer,
    Agent,
    Supervisor,
    Observer,
    Custom(String),
}

impl fmt::Debug for ParticipantRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Customer => "Customer",
            Self::Agent => "Agent",
            Self::Supervisor => "Supervisor",
            Self::Observer => "Observer",
            Self::Custom(_) => "Custom",
        })
    }
}

#[derive(Clone)]
pub struct Participant {
    pub id: ParticipantId,
    pub conversation_id: ConversationId,
    pub identity_ref: Option<IdentityId>,
    pub kind: ParticipantKind,
    pub role: ParticipantRole,
    pub display_name: Option<String>,
    pub joined_at: DateTime<Utc>,
    pub left_at: Option<DateTime<Utc>>,
}

impl fmt::Debug for Participant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Participant")
            .field("identity_ref_present", &self.identity_ref.is_some())
            .field("kind", &self.kind)
            .field("role", &self.role)
            .field("display_name_present", &self.display_name.is_some())
            .field("joined_at", &self.joined_at)
            .field("left_at", &self.left_at)
            .finish()
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn participant_debug_redacts_identity_and_custom_values() {
        let secret = "credential-canary-value";
        let participant = Participant {
            id: ParticipantId::new(),
            conversation_id: ConversationId::new(),
            identity_ref: Some(IdentityId::new()),
            kind: ParticipantKind::External,
            role: ParticipantRole::Custom(secret.into()),
            display_name: Some(secret.into()),
            joined_at: Utc::now(),
            left_at: None,
        };
        assert!(!format!("{participant:?}").contains(secret));
        assert!(!format!("{:?}", participant.role).contains(secret));
    }
}
