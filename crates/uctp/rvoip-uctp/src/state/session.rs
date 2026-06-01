//! `UctpSessionState` + transitions per CONVERSATION_PROTOCOL.md §7.2/§7.3.

use crate::errors::UctpError;

/// One Session's lifecycle position. See `UCTP_IMPLEMENTATION_PLAN.md` §3.5.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UctpSessionState {
    /// `session.invite` sent, awaiting `session.accept`.
    Inviting,
    /// At least one Connection has fired `connection.ready`.
    Active,
    /// `session.end` issued, awaiting last `connection.end` (or grace window).
    Ending,
    /// Terminal.
    Ended,
}

/// Inputs that drive Session state transitions. Each variant maps 1:1 to
/// an envelope arrival or a local action.
#[derive(Clone, Copy, Debug)]
pub enum SessionInput {
    InviteSent,
    InviteReceived,
    AcceptSent,
    AcceptReceived,
    CancelSent,
    CancelReceived,
    ConnectionReady,
    EndSent,
    EndReceived,
    LastConnectionEnded,
}

/// Per-Session state machine. Holds the state plus any
/// transition-specific fields (currently none — the state alone is
/// enough for v0).
#[derive(Clone, Debug)]
pub struct SessionMachine {
    state: UctpSessionState,
}

impl SessionMachine {
    pub fn new_inviting() -> Self {
        Self {
            state: UctpSessionState::Inviting,
        }
    }

    pub fn state(&self) -> UctpSessionState {
        self.state
    }

    /// Apply an input. Returns the new state on success, or
    /// [`UctpError::IllegalTransition`] if the input doesn't fit.
    pub fn apply(&mut self, input: SessionInput) -> Result<UctpSessionState, UctpError> {
        let next = match (self.state, input) {
            // Inviting → Active on AcceptReceived + ConnectionReady.
            // The plan's §7.3 says Active starts when ≥1 connection.ready
            // fires; we accept either AcceptReceived (incoming accept) or
            // ConnectionReady (first ready) as the trigger.
            (UctpSessionState::Inviting, SessionInput::AcceptReceived)
            | (UctpSessionState::Inviting, SessionInput::AcceptSent)
            | (UctpSessionState::Inviting, SessionInput::ConnectionReady) => {
                UctpSessionState::Active
            }

            // Inviting → Ended on Cancel.
            (UctpSessionState::Inviting, SessionInput::CancelSent)
            | (UctpSessionState::Inviting, SessionInput::CancelReceived) => {
                UctpSessionState::Ended
            }

            // Active → Ending on either side's End.
            (UctpSessionState::Active, SessionInput::EndSent)
            | (UctpSessionState::Active, SessionInput::EndReceived) => UctpSessionState::Ending,

            // Active stays Active on additional ConnectionReady (multi-Connection sessions).
            (UctpSessionState::Active, SessionInput::ConnectionReady) => UctpSessionState::Active,

            // Ending → Ended once the last Connection has ended.
            (UctpSessionState::Ending, SessionInput::LastConnectionEnded) => {
                UctpSessionState::Ended
            }

            // Ending tolerates further connection.end without re-firing.
            (UctpSessionState::Ending, SessionInput::EndReceived) => UctpSessionState::Ending,

            (state, input) => {
                return Err(UctpError::IllegalTransition {
                    state: state_name(state),
                    event: input_name(input),
                });
            }
        };
        self.state = next;
        Ok(next)
    }
}

fn state_name(s: UctpSessionState) -> &'static str {
    match s {
        UctpSessionState::Inviting => "Inviting",
        UctpSessionState::Active => "Active",
        UctpSessionState::Ending => "Ending",
        UctpSessionState::Ended => "Ended",
    }
}

fn input_name(i: SessionInput) -> &'static str {
    match i {
        SessionInput::InviteSent => "InviteSent",
        SessionInput::InviteReceived => "InviteReceived",
        SessionInput::AcceptSent => "AcceptSent",
        SessionInput::AcceptReceived => "AcceptReceived",
        SessionInput::CancelSent => "CancelSent",
        SessionInput::CancelReceived => "CancelReceived",
        SessionInput::ConnectionReady => "ConnectionReady",
        SessionInput::EndSent => "EndSent",
        SessionInput::EndReceived => "EndReceived",
        SessionInput::LastConnectionEnded => "LastConnectionEnded",
    }
}
