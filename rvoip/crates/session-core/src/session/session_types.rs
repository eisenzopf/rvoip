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

impl SessionState {
    /// Check if a transition to another state is valid
    /// 
    /// This implements the complete session state machine according to the session lifecycle.
    /// 
    /// # Session State Transition Matrix
    /// 
    /// ```text
    /// From/To        │ Init │ Dial │ Ring │ Conn │ Hold │ Xfer │ Term │ End
    /// ───────────────┼──────┼──────┼──────┼──────┼──────┼──────┼──────┼─────
    /// Initializing   │  ❌  │  ✅  │  ✅  │  ❌  │  ❌  │  ❌  │  ✅  │  ✅
    /// Dialing        │  ❌  │  ❌  │  ✅  │  ✅  │  ❌  │  ❌  │  ✅  │  ✅
    /// Ringing        │  ❌  │  ❌  │  ❌  │  ✅  │  ❌  │  ❌  │  ✅  │  ✅
    /// Connected      │  ❌  │  ❌  │  ❌  │  ❌  │  ✅  │  ✅  │  ✅  │  ✅
    /// OnHold         │  ❌  │  ❌  │  ❌  │  ✅  │  ❌  │  ✅  │  ✅  │  ✅
    /// Transferring   │  ❌  │  ❌  │  ❌  │  ✅  │  ✅  │  ❌  │  ✅  │  ✅
    /// Terminating    │  ❌  │  ❌  │  ❌  │  ❌  │  ❌  │  ❌  │  ❌  │  ✅
    /// Terminated     │  ❌  │  ❌  │  ❌  │  ❌  │  ❌  │  ❌  │  ❌  │  ❌
    /// ```
    /// 
    /// # Arguments
    /// 
    /// * `target` - The target state to transition to
    /// 
    /// # Returns
    /// 
    /// `true` if the transition is valid, `false` otherwise
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_session_core::session::SessionState;
    /// 
    /// assert!(SessionState::Initializing.can_transition_to(SessionState::Dialing));
    /// assert!(SessionState::Connected.can_transition_to(SessionState::OnHold));
    /// assert!(!SessionState::Terminated.can_transition_to(SessionState::Connected));
    /// ```
    pub fn can_transition_to(&self, target: SessionState) -> bool {
        use SessionState::*;
        
        match (self, target) {
            // === From Initializing ===
            // Can start dialing (outgoing) or ringing (incoming), or terminate early
            (Initializing, Dialing) => true,      // Start outgoing call
            (Initializing, Ringing) => true,      // Incoming call received
            (Initializing, Terminating) => true,  // Early termination
            (Initializing, Terminated) => true,   // Direct termination (error scenarios)
            
            // === From Dialing ===
            // Outgoing call in progress - can receive responses or be cancelled
            (Dialing, Ringing) => true,          // 180 Ringing received
            (Dialing, Connected) => true,        // 200 OK received (direct connect)
            (Dialing, Terminating) => true,      // Cancel or error
            (Dialing, Terminated) => true,       // Direct termination (error scenarios)
            
            // === From Ringing ===
            // Incoming call ringing - can be answered or rejected
            (Ringing, Connected) => true,        // Call answered (200 OK sent)
            (Ringing, Terminating) => true,      // Call rejected or cancelled
            (Ringing, Terminated) => true,       // Direct termination (error scenarios)
            
            // === From Connected ===
            // Active call - can be put on hold, transferred, or terminated
            (Connected, OnHold) => true,         // Put call on hold (re-INVITE with a=sendonly/recvonly)
            (Connected, Transferring) => true,   // Start call transfer (REFER)
            (Connected, Terminating) => true,    // Hang up call (BYE)
            (Connected, Terminated) => true,     // Direct termination (error scenarios)
            
            // === From OnHold ===
            // Call on hold - can be resumed, transferred, or terminated
            (OnHold, Connected) => true,         // Resume call (re-INVITE with a=sendrecv)
            (OnHold, Transferring) => true,      // Transfer while on hold
            (OnHold, Terminating) => true,       // Hang up while on hold
            (OnHold, Terminated) => true,        // Direct termination (error scenarios)
            
            // === From Transferring ===
            // Call transfer in progress - can complete, fail, or be terminated
            (Transferring, Connected) => true,   // Transfer failed, back to connected
            (Transferring, OnHold) => true,      // Transfer to hold state
            (Transferring, Terminating) => true, // Transfer completed or cancelled
            (Transferring, Terminated) => true,  // Direct termination (error scenarios)
            
            // === From Terminating ===
            // Call termination in progress - can only go to terminated
            (Terminating, Terminated) => true,   // Termination completed (200 OK to BYE)
            
            // === From Terminated ===
            // Terminal state - no transitions allowed
            (Terminated, _) => false,
            
            // === Invalid Transitions ===
            // Any transition not explicitly allowed above
            _ => false,
        }
    }
    
    /// Get valid next states from current state
    /// 
    /// Returns a vector of all states that this state can transition to.
    /// Useful for UI state management and validation.
    /// 
    /// # Returns
    /// 
    /// Vector of valid target states
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_session_core::session::SessionState;
    /// 
    /// let valid_states = SessionState::Connected.valid_next_states();
    /// assert!(valid_states.contains(&SessionState::OnHold));
    /// assert!(valid_states.contains(&SessionState::Transferring));
    /// assert!(valid_states.contains(&SessionState::Terminating));
    /// ```
    pub fn valid_next_states(&self) -> Vec<SessionState> {
        use SessionState::*;
        
        [Initializing, Dialing, Ringing, Connected, OnHold, Transferring, Terminating, Terminated]
            .iter()
            .filter(|&state| self.can_transition_to(*state))
            .copied()
            .collect()
    }
    
    /// Check if this state represents an active call
    /// 
    /// Active states are those where media may be flowing and the call is established.
    /// 
    /// # Returns
    /// 
    /// `true` if the state represents an active call
    pub fn is_active(&self) -> bool {
        matches!(self, SessionState::Connected | SessionState::OnHold | SessionState::Transferring)
    }
    
    /// Check if this state represents a call in progress
    /// 
    /// In-progress states include all states from initialization to termination,
    /// excluding only the final terminated state.
    /// 
    /// # Returns
    /// 
    /// `true` if the call is in progress
    pub fn is_in_progress(&self) -> bool {
        !matches!(self, SessionState::Terminated)
    }
    
    /// Check if this state is a terminal state
    /// 
    /// Terminal states are those from which no further transitions are possible.
    /// 
    /// # Returns
    /// 
    /// `true` if this is a terminal state
    pub fn is_terminal(&self) -> bool {
        matches!(self, SessionState::Terminated)
    }
    
    /// Check if media should be active in this state
    /// 
    /// Returns true for states where RTP media should be flowing.
    /// 
    /// # Returns
    /// 
    /// `true` if media should be active
    pub fn should_have_active_media(&self) -> bool {
        matches!(self, SessionState::Connected)
    }
    
    /// Get the session direction that typically leads to this state
    /// 
    /// Some states are more common for incoming vs outgoing calls.
    /// 
    /// # Returns
    /// 
    /// Optional session direction if this state is direction-specific
    pub fn typical_direction(&self) -> Option<SessionDirection> {
        match self {
            SessionState::Dialing => Some(SessionDirection::Outgoing),
            SessionState::Ringing => Some(SessionDirection::Incoming),
            _ => None, // Other states can occur in both directions
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

    // === NEW: Comprehensive Session State Machine Tests ===

    #[test]
    fn test_session_state_transitions_from_initializing() {
        use SessionState::*;
        
        // Valid transitions from Initializing
        assert!(Initializing.can_transition_to(Dialing), "Should be able to start outgoing call");
        assert!(Initializing.can_transition_to(Ringing), "Should be able to receive incoming call");
        assert!(Initializing.can_transition_to(Terminating), "Should be able to terminate early");
        assert!(Initializing.can_transition_to(Terminated), "Should be able to terminate directly on error");
        
        // Invalid transitions from Initializing
        assert!(!Initializing.can_transition_to(Initializing), "Should not stay in same state");
        assert!(!Initializing.can_transition_to(Connected), "Cannot directly connect from initializing");
        assert!(!Initializing.can_transition_to(OnHold), "Cannot go on hold without being connected");
        assert!(!Initializing.can_transition_to(Transferring), "Cannot transfer without being connected");
    }

    #[test]
    fn test_session_state_transitions_from_dialing() {
        use SessionState::*;
        
        // Valid transitions from Dialing
        assert!(Dialing.can_transition_to(Ringing), "Should receive 180 Ringing");
        assert!(Dialing.can_transition_to(Connected), "Should receive 200 OK directly");
        assert!(Dialing.can_transition_to(Terminating), "Should be able to cancel");
        assert!(Dialing.can_transition_to(Terminated), "Should be able to terminate on error");
        
        // Invalid transitions from Dialing
        assert!(!Dialing.can_transition_to(Initializing), "Cannot go back to initializing");
        assert!(!Dialing.can_transition_to(Dialing), "Should not stay in same state");
        assert!(!Dialing.can_transition_to(OnHold), "Cannot go on hold without being connected");
        assert!(!Dialing.can_transition_to(Transferring), "Cannot transfer without being connected");
    }

    #[test]
    fn test_session_state_transitions_from_ringing() {
        use SessionState::*;
        
        // Valid transitions from Ringing
        assert!(Ringing.can_transition_to(Connected), "Should be able to answer call");
        assert!(Ringing.can_transition_to(Terminating), "Should be able to reject call");
        assert!(Ringing.can_transition_to(Terminated), "Should be able to terminate on error");
        
        // Invalid transitions from Ringing
        assert!(!Ringing.can_transition_to(Initializing), "Cannot go back to initializing");
        assert!(!Ringing.can_transition_to(Dialing), "Cannot go back to dialing");
        assert!(!Ringing.can_transition_to(Ringing), "Should not stay in same state");
        assert!(!Ringing.can_transition_to(OnHold), "Cannot go on hold without being connected");
        assert!(!Ringing.can_transition_to(Transferring), "Cannot transfer without being connected");
    }

    #[test]
    fn test_session_state_transitions_from_connected() {
        use SessionState::*;
        
        // Valid transitions from Connected
        assert!(Connected.can_transition_to(OnHold), "Should be able to put call on hold");
        assert!(Connected.can_transition_to(Transferring), "Should be able to start transfer");
        assert!(Connected.can_transition_to(Terminating), "Should be able to hang up");
        assert!(Connected.can_transition_to(Terminated), "Should be able to terminate on error");
        
        // Invalid transitions from Connected
        assert!(!Connected.can_transition_to(Initializing), "Cannot go back to initializing");
        assert!(!Connected.can_transition_to(Dialing), "Cannot go back to dialing");
        assert!(!Connected.can_transition_to(Ringing), "Cannot go back to ringing");
        assert!(!Connected.can_transition_to(Connected), "Should not stay in same state");
    }

    #[test]
    fn test_session_state_transitions_from_onhold() {
        use SessionState::*;
        
        // Valid transitions from OnHold
        assert!(OnHold.can_transition_to(Connected), "Should be able to resume call");
        assert!(OnHold.can_transition_to(Transferring), "Should be able to transfer while on hold");
        assert!(OnHold.can_transition_to(Terminating), "Should be able to hang up while on hold");
        assert!(OnHold.can_transition_to(Terminated), "Should be able to terminate on error");
        
        // Invalid transitions from OnHold
        assert!(!OnHold.can_transition_to(Initializing), "Cannot go back to initializing");
        assert!(!OnHold.can_transition_to(Dialing), "Cannot go back to dialing");
        assert!(!OnHold.can_transition_to(Ringing), "Cannot go back to ringing");
        assert!(!OnHold.can_transition_to(OnHold), "Should not stay in same state");
    }

    #[test]
    fn test_session_state_transitions_from_transferring() {
        use SessionState::*;
        
        // Valid transitions from Transferring
        assert!(Transferring.can_transition_to(Connected), "Transfer can fail, return to connected");
        assert!(Transferring.can_transition_to(OnHold), "Transfer can result in hold");
        assert!(Transferring.can_transition_to(Terminating), "Transfer can complete/cancel");
        assert!(Transferring.can_transition_to(Terminated), "Should be able to terminate on error");
        
        // Invalid transitions from Transferring
        assert!(!Transferring.can_transition_to(Initializing), "Cannot go back to initializing");
        assert!(!Transferring.can_transition_to(Dialing), "Cannot go back to dialing");
        assert!(!Transferring.can_transition_to(Ringing), "Cannot go back to ringing");
        assert!(!Transferring.can_transition_to(Transferring), "Should not stay in same state");
    }

    #[test]
    fn test_session_state_transitions_from_terminating() {
        use SessionState::*;
        
        // Valid transitions from Terminating
        assert!(Terminating.can_transition_to(Terminated), "Should complete termination");
        
        // Invalid transitions from Terminating (only Terminated is valid)
        assert!(!Terminating.can_transition_to(Initializing), "Cannot go back from terminating");
        assert!(!Terminating.can_transition_to(Dialing), "Cannot go back from terminating");
        assert!(!Terminating.can_transition_to(Ringing), "Cannot go back from terminating");
        assert!(!Terminating.can_transition_to(Connected), "Cannot go back from terminating");
        assert!(!Terminating.can_transition_to(OnHold), "Cannot go back from terminating");
        assert!(!Terminating.can_transition_to(Transferring), "Cannot go back from terminating");
        assert!(!Terminating.can_transition_to(Terminating), "Should not stay in same state");
    }

    #[test]
    fn test_session_state_transitions_from_terminated() {
        use SessionState::*;
        
        // No valid transitions from Terminated (terminal state)
        assert!(!Terminated.can_transition_to(Initializing), "Cannot transition from terminal state");
        assert!(!Terminated.can_transition_to(Dialing), "Cannot transition from terminal state");
        assert!(!Terminated.can_transition_to(Ringing), "Cannot transition from terminal state");
        assert!(!Terminated.can_transition_to(Connected), "Cannot transition from terminal state");
        assert!(!Terminated.can_transition_to(OnHold), "Cannot transition from terminal state");
        assert!(!Terminated.can_transition_to(Transferring), "Cannot transition from terminal state");
        assert!(!Terminated.can_transition_to(Terminating), "Cannot transition from terminal state");
        assert!(!Terminated.can_transition_to(Terminated), "Should not stay in same state");
    }

    #[test]
    fn test_valid_next_states() {
        use SessionState::*;
        
        // Test Initializing valid next states
        let states = Initializing.valid_next_states();
        assert_eq!(states.len(), 4);
        assert!(states.contains(&Dialing));
        assert!(states.contains(&Ringing));
        assert!(states.contains(&Terminating));
        assert!(states.contains(&Terminated));
        
        // Test Connected valid next states
        let states = Connected.valid_next_states();
        assert_eq!(states.len(), 4);
        assert!(states.contains(&OnHold));
        assert!(states.contains(&Transferring));
        assert!(states.contains(&Terminating));
        assert!(states.contains(&Terminated));
        
        // Test Terminated valid next states (none)
        let states = Terminated.valid_next_states();
        assert_eq!(states.len(), 0);
    }

    #[test]
    fn test_session_state_properties() {
        use SessionState::*;
        
        // Test is_active()
        assert!(!Initializing.is_active(), "Initializing should not be active");
        assert!(!Dialing.is_active(), "Dialing should not be active");
        assert!(!Ringing.is_active(), "Ringing should not be active");
        assert!(Connected.is_active(), "Connected should be active");
        assert!(OnHold.is_active(), "OnHold should be active");
        assert!(Transferring.is_active(), "Transferring should be active");
        assert!(!Terminating.is_active(), "Terminating should not be active");
        assert!(!Terminated.is_active(), "Terminated should not be active");
        
        // Test is_in_progress()
        assert!(Initializing.is_in_progress(), "Initializing should be in progress");
        assert!(Dialing.is_in_progress(), "Dialing should be in progress");
        assert!(Ringing.is_in_progress(), "Ringing should be in progress");
        assert!(Connected.is_in_progress(), "Connected should be in progress");
        assert!(OnHold.is_in_progress(), "OnHold should be in progress");
        assert!(Transferring.is_in_progress(), "Transferring should be in progress");
        assert!(Terminating.is_in_progress(), "Terminating should be in progress");
        assert!(!Terminated.is_in_progress(), "Terminated should not be in progress");
        
        // Test is_terminal()
        assert!(!Initializing.is_terminal(), "Initializing should not be terminal");
        assert!(!Dialing.is_terminal(), "Dialing should not be terminal");
        assert!(!Ringing.is_terminal(), "Ringing should not be terminal");
        assert!(!Connected.is_terminal(), "Connected should not be terminal");
        assert!(!OnHold.is_terminal(), "OnHold should not be terminal");
        assert!(!Transferring.is_terminal(), "Transferring should not be terminal");
        assert!(!Terminating.is_terminal(), "Terminating should not be terminal");
        assert!(Terminated.is_terminal(), "Terminated should be terminal");
        
        // Test should_have_active_media()
        assert!(!Initializing.should_have_active_media(), "Initializing should not have active media");
        assert!(!Dialing.should_have_active_media(), "Dialing should not have active media");
        assert!(!Ringing.should_have_active_media(), "Ringing should not have active media");
        assert!(Connected.should_have_active_media(), "Connected should have active media");
        assert!(!OnHold.should_have_active_media(), "OnHold should not have active media (on hold)");
        assert!(!Transferring.should_have_active_media(), "Transferring should not have active media");
        assert!(!Terminating.should_have_active_media(), "Terminating should not have active media");
        assert!(!Terminated.should_have_active_media(), "Terminated should not have active media");
    }

    #[test]
    fn test_typical_direction() {
        use SessionState::*;
        
        // Test typical directions
        assert_eq!(Initializing.typical_direction(), None, "Initializing has no typical direction");
        assert_eq!(Dialing.typical_direction(), Some(SessionDirection::Outgoing), "Dialing is typically outgoing");
        assert_eq!(Ringing.typical_direction(), Some(SessionDirection::Incoming), "Ringing is typically incoming");
        assert_eq!(Connected.typical_direction(), None, "Connected has no typical direction");
        assert_eq!(OnHold.typical_direction(), None, "OnHold has no typical direction");
        assert_eq!(Transferring.typical_direction(), None, "Transferring has no typical direction");
        assert_eq!(Terminating.typical_direction(), None, "Terminating has no typical direction");
        assert_eq!(Terminated.typical_direction(), None, "Terminated has no typical direction");
    }

    #[test]
    fn test_complete_call_flow_outgoing() {
        use SessionState::*;
        
        // Test complete outgoing call flow
        let mut state = Initializing;
        
        // Start outgoing call
        assert!(state.can_transition_to(Dialing));
        state = Dialing;
        
        // Receive 180 Ringing
        assert!(state.can_transition_to(Ringing));
        state = Ringing;
        
        // Call answered (200 OK)
        assert!(state.can_transition_to(Connected));
        state = Connected;
        
        // Put on hold
        assert!(state.can_transition_to(OnHold));
        state = OnHold;
        
        // Resume
        assert!(state.can_transition_to(Connected));
        state = Connected;
        
        // Hang up
        assert!(state.can_transition_to(Terminating));
        state = Terminating;
        
        // Complete termination
        assert!(state.can_transition_to(Terminated));
        state = Terminated;
        
        // No further transitions possible
        assert_eq!(state.valid_next_states().len(), 0);
    }

    #[test]
    fn test_complete_call_flow_incoming() {
        use SessionState::*;
        
        // Test complete incoming call flow
        let mut state = Initializing;
        
        // Incoming call received
        assert!(state.can_transition_to(Ringing));
        state = Ringing;
        
        // Answer call
        assert!(state.can_transition_to(Connected));
        state = Connected;
        
        // Start transfer
        assert!(state.can_transition_to(Transferring));
        state = Transferring;
        
        // Transfer fails, back to connected
        assert!(state.can_transition_to(Connected));
        state = Connected;
        
        // Hang up
        assert!(state.can_transition_to(Terminating));
        state = Terminating;
        
        // Complete termination
        assert!(state.can_transition_to(Terminated));
        state = Terminated;
        
        // No further transitions possible
        assert!(state.is_terminal());
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