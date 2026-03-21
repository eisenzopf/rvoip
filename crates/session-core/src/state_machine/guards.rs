//! Guard evaluation for state machine transitions
//!
//! Guards are conditions that must be satisfied before a transition can occur.
//! The full guard evaluation requires SessionState from the session_store module,
//! which will be integrated in a future phase.
//!
//! This module currently provides the guard evaluation trait and type definitions.

use crate::state_table::Guard;

/// Trait for types that can be evaluated against guards.
///
/// Implement this trait on your session state type to enable guard
/// checking in the state machine.
pub trait GuardEvaluable {
    /// Whether a local SDP has been set
    fn has_local_sdp(&self) -> bool;
    /// Whether a remote SDP has been set
    fn has_remote_sdp(&self) -> bool;
    /// Whether a negotiated config exists
    fn has_negotiated_config(&self) -> bool;
    /// Whether all coordination conditions are met
    fn all_conditions_met(&self) -> bool;
    /// Whether the dialog has been established
    fn dialog_established(&self) -> bool;
    /// Whether the media session is ready
    fn media_ready(&self) -> bool;
    /// Whether SDP has been negotiated
    fn sdp_negotiated(&self) -> bool;
    /// Whether the session is in the Idle state
    fn is_idle(&self) -> bool;
    /// Whether the session is in an active call
    fn in_active_call(&self) -> bool;
    /// Whether the session is registered
    fn is_registered(&self) -> bool;
    /// Whether the session is subscribed
    fn is_subscribed(&self) -> bool;
}

/// Check if a guard condition is satisfied against any type that implements GuardEvaluable
pub fn check_guard(guard: &Guard, session: &dyn GuardEvaluable) -> bool {
    match guard {
        Guard::HasLocalSDP => session.has_local_sdp(),
        Guard::HasRemoteSDP => session.has_remote_sdp(),
        Guard::HasNegotiatedConfig => session.has_negotiated_config(),
        Guard::AllConditionsMet => session.all_conditions_met(),
        Guard::DialogEstablished => session.dialog_established(),
        Guard::MediaReady => session.media_ready(),
        Guard::SDPNegotiated => session.sdp_negotiated(),
        Guard::IsIdle => session.is_idle(),
        Guard::InActiveCall => session.in_active_call(),
        Guard::IsRegistered => session.is_registered(),
        Guard::IsSubscribed => session.is_subscribed(),
        Guard::HasActiveSubscription => session.is_subscribed(),
        Guard::Custom(name) => {
            tracing::warn!("Custom guard '{}' not implemented", name);
            false
        }
    }
}
