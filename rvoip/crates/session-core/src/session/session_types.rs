use std::fmt;
use serde::{Serialize, Deserialize};
use uuid::Uuid;

/// SIP session state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    /// Session is being initialized
    Initializing,
    
    /// Outgoing call is being established
    Dialing,
    
    /// Incoming call is being received
    Ringing,
    
    /// Call is connected and active
    Connected,
    
    /// Call is on hold
    OnHold,
    
    /// Call is being transferred
    Transferring,
    
    /// Call is being terminated
    Terminating,
    
    /// Call has ended
    Terminated,
}

impl fmt::Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionState::Initializing => write!(f, "Initializing"),
            SessionState::Dialing => write!(f, "Dialing"),
            SessionState::Ringing => write!(f, "Ringing"),
            SessionState::Connected => write!(f, "Connected"),
            SessionState::OnHold => write!(f, "OnHold"),
            SessionState::Transferring => write!(f, "Transferring"),
            SessionState::Terminating => write!(f, "Terminating"),
            SessionState::Terminated => write!(f, "Terminated"),
        }
    }
}

/// Session direction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionDirection {
    /// Outgoing call
    Outgoing,
    /// Incoming call
    Incoming,
}

/// Transaction types that can be used in a session
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionTransactionType {
    /// Initial INVITE transaction
    InitialInvite,
    /// Re-INVITE transaction
    ReInvite,
    /// BYE transaction
    Bye,
    /// CANCEL transaction
    Cancel,
    /// OPTIONS transaction
    Options,
    /// INFO transaction
    Info,
    /// MESSAGE transaction
    Message,
    /// REFER transaction
    Refer,
    /// NOTIFY transaction
    Notify,
    /// UPDATE transaction
    Update,
    /// REGISTER transaction (rare for sessions)
    Register,
    /// Other transaction type
    Other(String),
}

impl fmt::Display for SessionTransactionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionTransactionType::InitialInvite => write!(f, "InitialInvite"),
            SessionTransactionType::ReInvite => write!(f, "ReInvite"),
            SessionTransactionType::Bye => write!(f, "Bye"),
            SessionTransactionType::Cancel => write!(f, "Cancel"),
            SessionTransactionType::Options => write!(f, "Options"),
            SessionTransactionType::Info => write!(f, "Info"),
            SessionTransactionType::Message => write!(f, "Message"),
            SessionTransactionType::Refer => write!(f, "Refer"),
            SessionTransactionType::Notify => write!(f, "Notify"),
            SessionTransactionType::Update => write!(f, "Update"),
            SessionTransactionType::Register => write!(f, "Register"),
            SessionTransactionType::Other(s) => write!(f, "Other({})", s),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_session_state_display() {
        assert_eq!(SessionState::Initializing.to_string(), "Initializing");
        assert_eq!(SessionState::Dialing.to_string(), "Dialing");
        assert_eq!(SessionState::Ringing.to_string(), "Ringing");
        assert_eq!(SessionState::Connected.to_string(), "Connected");
        assert_eq!(SessionState::OnHold.to_string(), "OnHold");
        assert_eq!(SessionState::Transferring.to_string(), "Transferring");
        assert_eq!(SessionState::Terminating.to_string(), "Terminating");
        assert_eq!(SessionState::Terminated.to_string(), "Terminated");
    }
    
    #[test]
    fn test_session_transaction_type_display() {
        assert_eq!(SessionTransactionType::InitialInvite.to_string(), "InitialInvite");
        assert_eq!(SessionTransactionType::ReInvite.to_string(), "ReInvite");
        assert_eq!(SessionTransactionType::Bye.to_string(), "Bye");
        assert_eq!(SessionTransactionType::Cancel.to_string(), "Cancel");
        assert_eq!(SessionTransactionType::Other("Custom".to_string()).to_string(), "Other(Custom)");
    }
    
    #[test]
    fn test_transfer_state_display() {
        assert_eq!(TransferState::None.to_string(), "None");
        assert_eq!(TransferState::Initiated.to_string(), "Initiated");
        assert_eq!(TransferState::Accepted.to_string(), "Accepted");
        assert_eq!(TransferState::Confirmed.to_string(), "Confirmed");
        assert_eq!(TransferState::Failed("timeout".to_string()).to_string(), "Failed(timeout)");
    }
    
    #[test]
    fn test_transfer_type_display() {
        assert_eq!(TransferType::Blind.to_string(), "Blind");
        assert_eq!(TransferType::Attended.to_string(), "Attended");
        assert_eq!(TransferType::Consultative.to_string(), "Consultative");
    }
}

// ==== Call Transfer Support (REFER Method) ====

/// Unique identifier for call transfers
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TransferId(pub Uuid);

impl TransferId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl fmt::Display for TransferId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Transfer state according to RFC 3515
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferState {
    /// No transfer in progress
    None,
    /// Transfer request initiated (REFER sent/received)
    Initiated,
    /// Transfer accepted (202 Accepted received/sent)
    Accepted,
    /// Transfer confirmed (NOTIFY with 200 OK received)
    Confirmed,
    /// Transfer failed with reason
    Failed(String),
}

impl fmt::Display for TransferState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransferState::None => write!(f, "None"),
            TransferState::Initiated => write!(f, "Initiated"),
            TransferState::Accepted => write!(f, "Accepted"),
            TransferState::Confirmed => write!(f, "Confirmed"),
            TransferState::Failed(reason) => write!(f, "Failed({})", reason),
        }
    }
}

/// Types of call transfer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferType {
    /// Blind transfer - transfer without consultation
    Blind,
    /// Attended transfer - transfer after consultation
    Attended,
    /// Consultative transfer - transfer with confirmation
    Consultative,
}

impl fmt::Display for TransferType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransferType::Blind => write!(f, "Blind"),
            TransferType::Attended => write!(f, "Attended"),
            TransferType::Consultative => write!(f, "Consultative"),
        }
    }
}

/// Transfer context with all necessary information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferContext {
    /// Unique transfer identifier
    pub id: TransferId,
    
    /// Type of transfer
    pub transfer_type: TransferType,
    
    /// Current transfer state
    pub state: TransferState,
    
    /// Target URI for the transfer
    pub target_uri: String,
    
    /// Session being transferred FROM
    pub transferor_session_id: Option<super::SessionId>,
    
    /// Session being transferred TO
    pub transferee_session_id: Option<super::SessionId>,
    
    /// Consultation session (for attended transfers)
    pub consultation_session_id: Option<super::SessionId>,
    
    /// REFER-To header value
    pub refer_to: String,
    
    /// Optional Referred-By header
    pub referred_by: Option<String>,
    
    /// Transfer reason/description
    pub reason: Option<String>,
    
    /// Timestamp when transfer was initiated
    pub initiated_at: std::time::SystemTime,
    
    /// Timestamp when transfer completed (if any)
    pub completed_at: Option<std::time::SystemTime>,
} 