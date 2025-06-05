//! Call Control Functions
//!
//! Simple functions for controlling active calls (hold, transfer, terminate, etc.).

use crate::api::types::{CallSession, CallState};
use crate::errors::Result;

/// Put a call on hold
/// 
/// # Arguments
/// * `session` - The active call session to hold
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::api::*;
/// 
/// # async fn example(call: CallSession) -> Result<()> {
/// hold_call(&call).await?;
/// assert_eq!(call.state(), &CallState::OnHold);
/// # Ok(())
/// # }
/// ```
pub async fn hold_call(session: &CallSession) -> Result<()> {
    if !session.is_active() {
        return Err(crate::errors::SessionError::InvalidState(
            format!("Cannot hold call in state: {:?}", session.state)
        ));
    }

    session.manager.hold_session(&session.id).await
}

/// Resume a call from hold
/// 
/// # Arguments
/// * `session` - The held call session to resume
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::api::*;
/// 
/// # async fn example(call: CallSession) -> Result<()> {
/// // First hold the call
/// hold_call(&call).await?;
/// 
/// // Then resume it
/// resume_call(&call).await?;
/// assert_eq!(call.state(), &CallState::Active);
/// # Ok(())
/// # }
/// ```
pub async fn resume_call(session: &CallSession) -> Result<()> {
    if !matches!(session.state, CallState::OnHold) {
        return Err(crate::errors::SessionError::InvalidState(
            format!("Cannot resume call not on hold: {:?}", session.state)
        ));
    }

    session.manager.resume_session(&session.id).await
}

/// Transfer a call to another destination
/// 
/// # Arguments
/// * `session` - The active call session to transfer
/// * `target` - The destination URI to transfer to (e.g., "sip:bob@example.com")
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::api::*;
/// 
/// # async fn example(call: CallSession) -> Result<()> {
/// transfer_call(&call, "sip:transferee@example.com").await?;
/// # Ok(())
/// # }
/// ```
pub async fn transfer_call(session: &CallSession, target: &str) -> Result<()> {
    if !session.state.is_in_progress() {
        return Err(crate::errors::SessionError::InvalidState(
            format!("Cannot transfer call in state: {:?}", session.state)
        ));
    }

    session.manager.transfer_session(&session.id, target).await
}

/// Terminate a call
/// 
/// # Arguments
/// * `session` - The call session to terminate
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::api::*;
/// 
/// # async fn example(call: CallSession) -> Result<()> {
/// terminate_call(&call).await?;
/// assert!(call.state().is_final());
/// # Ok(())
/// # }
/// ```
pub async fn terminate_call(session: &CallSession) -> Result<()> {
    if session.state.is_final() {
        return Ok(()); // Already terminated
    }

    session.manager.terminate_session(&session.id).await
}

/// Send DTMF tones to the call
/// 
/// # Arguments
/// * `session` - The active call session
/// * `digits` - The DTMF digits to send (e.g., "123*#")
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::api::*;
/// 
/// # async fn example(call: CallSession) -> Result<()> {
/// send_dtmf(&call, "123").await?;
/// # Ok(())
/// # }
/// ```
pub async fn send_dtmf(session: &CallSession, digits: &str) -> Result<()> {
    if !session.is_active() {
        return Err(crate::errors::SessionError::InvalidState(
            format!("Cannot send DTMF on inactive call: {:?}", session.state)
        ));
    }

    session.manager.send_dtmf(&session.id, digits).await
}

/// Mute the call (stop sending audio)
/// 
/// # Arguments
/// * `session` - The active call session to mute
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::api::*;
/// 
/// # async fn example(call: CallSession) -> Result<()> {
/// mute_call(&call).await?;
/// # Ok(())
/// # }
/// ```
pub async fn mute_call(session: &CallSession) -> Result<()> {
    if !session.is_active() {
        return Err(crate::errors::SessionError::InvalidState(
            format!("Cannot mute inactive call: {:?}", session.state)
        ));
    }

    session.manager.mute_session(&session.id, true).await
}

/// Unmute the call (resume sending audio)
/// 
/// # Arguments
/// * `session` - The muted call session to unmute
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::api::*;
/// 
/// # async fn example(call: CallSession) -> Result<()> {
/// // First mute
/// mute_call(&call).await?;
/// 
/// // Then unmute
/// unmute_call(&call).await?;
/// # Ok(())
/// # }
/// ```
pub async fn unmute_call(session: &CallSession) -> Result<()> {
    if !session.is_active() {
        return Err(crate::errors::SessionError::InvalidState(
            format!("Cannot unmute inactive call: {:?}", session.state)
        ));
    }

    session.manager.mute_session(&session.id, false).await
}

/// Get media information for the call
/// 
/// # Arguments
/// * `session` - The call session to get media info for
/// 
/// # Returns
/// Media information including SDP, ports, and codec details
pub async fn get_media_info(session: &CallSession) -> Result<crate::api::types::MediaInfo> {
    session.manager.get_media_info(&session.id).await
}

/// Update the media session (e.g., add/remove streams)
/// 
/// # Arguments
/// * `session` - The call session to update
/// * `new_sdp` - The new SDP offer/answer
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::api::*;
/// 
/// # async fn example(call: CallSession, sdp: String) -> Result<()> {
/// update_media(&call, &sdp).await?;
/// # Ok(())
/// # }
/// ```
pub async fn update_media(session: &CallSession, new_sdp: &str) -> Result<()> {
    if !session.state.is_in_progress() {
        return Err(crate::errors::SessionError::InvalidState(
            format!("Cannot update media for call in state: {:?}", session.state)
        ));
    }

    session.manager.update_media(&session.id, new_sdp).await
} 