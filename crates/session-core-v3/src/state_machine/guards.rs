use crate::{
    state_table::Guard,
    session_store::SessionState,
};

/// Check if a guard condition is satisfied
pub async fn check_guard(guard: &Guard, session: &SessionState) -> bool {
    use crate::types::CallState;
    
    match guard {
        Guard::HasLocalSDP => session.local_sdp.is_some(),
        Guard::HasRemoteSDP => session.remote_sdp.is_some(),
        Guard::HasNegotiatedConfig => session.negotiated_config.is_some(),
        Guard::AllConditionsMet => session.all_conditions_met(),
        Guard::DialogEstablished => session.dialog_established,
        Guard::MediaReady => session.media_session_ready,
        Guard::SDPNegotiated => session.sdp_negotiated,
        Guard::IsIdle => matches!(session.call_state, CallState::Idle),
        Guard::InActiveCall => matches!(session.call_state, CallState::Active),
        Guard::IsRegistered => matches!(session.call_state, CallState::Registered),
        Guard::IsSubscribed => matches!(session.call_state, CallState::Subscribed),
        Guard::HasActiveSubscription => matches!(session.call_state, CallState::Subscribed),
        Guard::Custom(name) => {
            // Custom guards can be implemented here
            tracing::warn!("Custom guard '{}' not implemented", name);
            false
        }
    }
}