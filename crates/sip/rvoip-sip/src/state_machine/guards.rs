use crate::{session_store::SessionState, state_table::Guard};

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
        Guard::HasPendingReinvite => session.pending_reinvite.is_some(),
        Guard::Custom(name) => {
            // Custom guards can be implemented here
            tracing::warn!("Custom guard '{}' not implemented", name);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_store::state::PendingReinvite;
    use crate::state_table::types::{EventType, Role, StateKey};
    use crate::state_table::{yaml_loader::YamlTableLoader, Action};
    use crate::types::CallState;

    fn fresh_active_session() -> SessionState {
        let id = crate::state_table::types::SessionId("glare-guard-test".into());
        let mut session = SessionState::new(id, Role::UAC);
        session.call_state = CallState::Active;
        session
    }

    /// RFC 3261 §14.1 — the `HasPendingReinvite` guard MUST detect any
    /// kind of in-flight outbound re-INVITE, including the
    /// builder-API (`SdpUpdate`) path closed by C1.3. Hold/Resume have
    /// always set this; the bug being fixed here was that the builder
    /// API did not.
    #[tokio::test]
    async fn pending_reinvite_guard_fires_for_builder_api_in_flight() {
        let mut session = fresh_active_session();

        assert!(
            !check_guard(&Guard::HasPendingReinvite, &session).await,
            "guard must be false on a fresh Active session"
        );

        // Builder-API path stashes SdpUpdate; the regression we're
        // protecting against was that this never happened, so the
        // UAS-side glare row in the YAML table never matched.
        session.pending_reinvite = Some(PendingReinvite::SdpUpdate("v=0\r\n".into()));
        assert!(
            check_guard(&Guard::HasPendingReinvite, &session).await,
            "guard must fire while a builder-API re-INVITE is in flight"
        );

        // Hold/Resume paths must still fire — these have always
        // worked via Action::HoldCall / Action::ResumeCall.
        session.pending_reinvite = Some(PendingReinvite::Hold);
        assert!(check_guard(&Guard::HasPendingReinvite, &session).await);
        session.pending_reinvite = Some(PendingReinvite::Resume);
        assert!(check_guard(&Guard::HasPendingReinvite, &session).await);
    }

    /// The YAML state table's `Active + ReinviteReceived` row is the
    /// unguarded 200 OK transition (the HashMap-backed table stores
    /// only one transition per `{role,state,event}` key — last-write
    /// wins, so the guarded 491 row is dead). UAS-side glare for
    /// builder-API re-INVITEs is therefore handled in
    /// `executor::process_event` as a pre-table short-circuit guarded
    /// on `pending_reinvite.is_some()`. This test pins the YAML row's
    /// content so a future re-org that re-introduces a guarded row
    /// (or removes the unguarded one) gets caught.
    #[test]
    fn active_reinvite_received_table_row_is_200_ok_unguarded() {
        let table = YamlTableLoader::load_default().expect("default state table loads");

        let key = StateKey {
            role: Role::UAC,
            state: CallState::Active,
            event: EventType::ReinviteReceived { sdp: None },
        };

        let transition = table
            .get(&key)
            .expect("Active + ReinviteReceived must have a transition row");

        assert!(
            transition.guards.is_empty(),
            "Active + ReinviteReceived must be the unguarded 200 OK row \
             (HasPendingReinvite-guarded row was dead-code per HashMap \
             last-write-wins semantics); UAS glare lives in the executor"
        );
        assert!(
            transition
                .actions
                .iter()
                .any(|a| matches!(a, Action::SendSIPResponse(200, _))),
            "Active + ReinviteReceived must send 200 OK; got actions: {:?}",
            transition.actions
        );
    }
}
