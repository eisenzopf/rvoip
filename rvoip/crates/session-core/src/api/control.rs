//! Call Control Functions
//!
//! Simple functions for controlling active calls (hold, transfer, terminate, etc.).

use std::sync::Arc;
use crate::api::types::{CallSession, CallState};
use crate::manager::SessionManager;
use crate::Result;

/// Put a call on hold
/// 
/// # Arguments
/// * `session_manager` - The SessionManager instance
/// * `session` - The active call session to hold
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::api::*;
/// use std::sync::Arc;
/// use rvoip_session_core::{SessionManager, Result};
/// 
/// # async fn example(manager: Arc<SessionManager>, call: CallSession) -> Result<()> {
/// hold_call(&manager, &call).await?;
/// # Ok(())
/// # }
/// ```
pub async fn hold_call(session_manager: &Arc<SessionManager>, session: &CallSession) -> Result<()> {
    // Get the current session state from the manager to avoid stale state issues
    let current_session = session_manager.find_session(&session.id).await?
        .ok_or_else(|| crate::errors::SessionError::session_not_found(&session.id.0))?;
    
    if !current_session.is_active() {
        return Err(crate::errors::SessionError::InvalidState(
            format!("Cannot hold call in state: {:?}", current_session.state)
        ));
    }

    session_manager.hold_session(&session.id).await
}

/// Resume a call from hold
/// 
/// # Arguments
/// * `session_manager` - The SessionManager instance
/// * `session` - The held call session to resume
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::api::*;
/// use std::sync::Arc;
/// use rvoip_session_core::{SessionManager, Result};
/// 
/// # async fn example(manager: Arc<SessionManager>, call: CallSession) -> Result<()> {
/// // First hold the call
/// hold_call(&manager, &call).await?;
/// 
/// // Then resume it
/// resume_call(&manager, &call).await?;
/// # Ok(())
/// # }
/// ```
pub async fn resume_call(session_manager: &Arc<SessionManager>, session: &CallSession) -> Result<()> {
    // Get the current session state from the manager to avoid stale state issues
    let current_session = session_manager.find_session(&session.id).await?
        .ok_or_else(|| crate::errors::SessionError::session_not_found(&session.id.0))?;
    
    if !matches!(current_session.state, CallState::OnHold) {
        return Err(crate::errors::SessionError::InvalidState(
            format!("Cannot resume call not on hold: {:?}", current_session.state)
        ));
    }

    session_manager.resume_session(&session.id).await
}

/// Transfer a call to another destination
/// 
/// # Arguments
/// * `session_manager` - The SessionManager instance
/// * `session` - The active call session to transfer
/// * `target` - The destination URI to transfer to (e.g., "sip:bob@example.com")
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::api::*;
/// use std::sync::Arc;
/// use rvoip_session_core::{SessionManager, Result};
/// 
/// # async fn example(manager: Arc<SessionManager>, call: CallSession) -> Result<()> {
/// transfer_call(&manager, &call, "sip:transferee@example.com").await?;
/// # Ok(())
/// # }
/// ```
pub async fn transfer_call(session_manager: &Arc<SessionManager>, session: &CallSession, target: &str) -> Result<()> {
    // Get the current session state from the manager to avoid stale state issues
    let current_session = session_manager.find_session(&session.id).await?
        .ok_or_else(|| crate::errors::SessionError::session_not_found(&session.id.0))?;
    
    if !current_session.state.is_in_progress() {
        return Err(crate::errors::SessionError::InvalidState(
            format!("Cannot transfer call in state: {:?}", current_session.state)
        ));
    }

    session_manager.transfer_session(&session.id, target).await
}

/// Terminate a call
/// 
/// # Arguments
/// * `session_manager` - The SessionManager instance
/// * `session` - The call session to terminate
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::api::*;
/// use std::sync::Arc;
/// use rvoip_session_core::{SessionManager, Result};
/// 
/// # async fn example(manager: Arc<SessionManager>, call: CallSession) -> Result<()> {
/// terminate_call(&manager, &call).await?;
/// # Ok(())
/// # }
/// ```
pub async fn terminate_call(session_manager: &Arc<SessionManager>, session: &CallSession) -> Result<()> {
    if session.state.is_final() {
        return Ok(()); // Already terminated
    }

    session_manager.terminate_session(&session.id).await
}

/// Send DTMF tones to the call
/// 
/// # Arguments
/// * `session_manager` - The SessionManager instance
/// * `session` - The active call session
/// * `digits` - The DTMF digits to send (e.g., "123*#")
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::api::*;
/// use std::sync::Arc;
/// use rvoip_session_core::{SessionManager, Result};
/// 
/// # async fn example(manager: Arc<SessionManager>, call: CallSession) -> Result<()> {
/// send_dtmf(&manager, &call, "123").await?;
/// # Ok(())
/// # }
/// ```
pub async fn send_dtmf(session_manager: &Arc<SessionManager>, session: &CallSession, digits: &str) -> Result<()> {
    // Get the current session state from the manager to avoid stale state issues
    let current_session = session_manager.find_session(&session.id).await?
        .ok_or_else(|| crate::errors::SessionError::session_not_found(&session.id.0))?;
    
    if !current_session.is_active() {
        return Err(crate::errors::SessionError::InvalidState(
            format!("Cannot send DTMF on inactive call: {:?}", current_session.state)
        ));
    }

    session_manager.send_dtmf(&session.id, digits).await
}

/// Mute the call (stop sending audio)
/// 
/// # Arguments
/// * `session_manager` - The SessionManager instance
/// * `session` - The active call session to mute
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::api::*;
/// use std::sync::Arc;
/// use rvoip_session_core::{SessionManager, Result};
/// 
/// # async fn example(manager: Arc<SessionManager>, call: CallSession) -> Result<()> {
/// mute_call(&manager, &call).await?;
/// # Ok(())
/// # }
/// ```
pub async fn mute_call(session_manager: &Arc<SessionManager>, session: &CallSession) -> Result<()> {
    // Get the current session state from the manager to avoid stale state issues
    let current_session = session_manager.find_session(&session.id).await?
        .ok_or_else(|| crate::errors::SessionError::session_not_found(&session.id.0))?;
    
    if !current_session.is_active() {
        return Err(crate::errors::SessionError::InvalidState(
            format!("Cannot mute inactive call: {:?}", current_session.state)
        ));
    }

    session_manager.mute_session(&session.id, true).await
}

/// Unmute the call (resume sending audio)
/// 
/// # Arguments
/// * `session_manager` - The SessionManager instance
/// * `session` - The muted call session to unmute
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::api::*;
/// use std::sync::Arc;
/// use rvoip_session_core::{SessionManager, Result};
/// 
/// # async fn example(manager: Arc<SessionManager>, call: CallSession) -> Result<()> {
/// // First mute
/// mute_call(&manager, &call).await?;
/// 
/// // Then unmute
/// unmute_call(&manager, &call).await?;
/// # Ok(())
/// # }
/// ```
pub async fn unmute_call(session_manager: &Arc<SessionManager>, session: &CallSession) -> Result<()> {
    // Get the current session state from the manager to avoid stale state issues
    let current_session = session_manager.find_session(&session.id).await?
        .ok_or_else(|| crate::errors::SessionError::session_not_found(&session.id.0))?;
    
    if !current_session.is_active() {
        return Err(crate::errors::SessionError::InvalidState(
            format!("Cannot unmute inactive call: {:?}", current_session.state)
        ));
    }

    session_manager.mute_session(&session.id, false).await
}

/// Get media information for the call
/// 
/// # Arguments
/// * `session_manager` - The SessionManager instance
/// * `session` - The call session to get media info for
/// 
/// # Returns
/// Media information including SDP, ports, and codec details
pub async fn get_media_info(session_manager: &Arc<SessionManager>, session: &CallSession) -> Result<crate::api::types::MediaInfo> {
    session_manager.get_media_info(&session.id).await
}

/// Update the media session (e.g., add/remove streams)
/// 
/// # Arguments
/// * `session_manager` - The SessionManager instance
/// * `session` - The call session to update
/// * `new_sdp` - The new SDP offer/answer
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::api::*;
/// use std::sync::Arc;
/// use rvoip_session_core::{SessionManager, Result};
/// 
/// # async fn example(manager: Arc<SessionManager>, call: CallSession, sdp: String) -> Result<()> {
/// update_media(&manager, &call, &sdp).await?;
/// # Ok(())
/// # }
/// ```
pub async fn update_media(session_manager: &Arc<SessionManager>, session: &CallSession, new_sdp: &str) -> Result<()> {
    // Get the current session state from the manager to avoid stale state issues
    let current_session = session_manager.find_session(&session.id).await?
        .ok_or_else(|| crate::errors::SessionError::session_not_found(&session.id.0))?;
    
    if !current_session.state.is_in_progress() {
        return Err(crate::errors::SessionError::InvalidState(
            format!("Cannot update media for call in state: {:?}", current_session.state)
        ));
    }

    session_manager.update_media(&session.id, new_sdp).await
} 