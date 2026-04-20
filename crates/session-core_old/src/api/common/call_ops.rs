//! Common call operations used by all API levels

use std::sync::Arc;
use crate::api::types::SessionId;
use crate::api::control::SessionControl;
use crate::api::media::MediaControl;
use crate::coordinator::SessionCoordinator;
use crate::errors::Result;

/// Put a call on hold
pub async fn hold(coordinator: &Arc<SessionCoordinator>, session_id: &SessionId) -> Result<()> {
    SessionControl::hold_session(coordinator, session_id).await
}

/// Resume a call from hold
pub async fn unhold(coordinator: &Arc<SessionCoordinator>, session_id: &SessionId) -> Result<()> {
    SessionControl::resume_session(coordinator, session_id).await
}

/// Mute audio transmission (stop sending audio)
pub async fn mute(coordinator: &Arc<SessionCoordinator>, session_id: &SessionId) -> Result<()> {
    MediaControl::stop_audio_transmission(coordinator, session_id).await
}

/// Unmute audio transmission (resume sending audio)
pub async fn unmute(coordinator: &Arc<SessionCoordinator>, session_id: &SessionId) -> Result<()> {
    MediaControl::start_audio_transmission(coordinator, session_id).await
}

/// Send DTMF digits
pub async fn send_dtmf(
    coordinator: &Arc<SessionCoordinator>,
    session_id: &SessionId,
    digits: &str,
) -> Result<()> {
    SessionControl::send_dtmf(coordinator, session_id, digits).await
}

/// Blind transfer a call
pub async fn transfer(
    coordinator: &Arc<SessionCoordinator>,
    session_id: &SessionId,
    target: &str,
) -> Result<()> {
    // Ensure the target is a proper SIP URI
    let formatted_target = if target.starts_with("sip:") || target.starts_with("tel:") {
        target.to_string()
    } else if target.contains('@') {
        format!("sip:{}", target)
    } else if target.starts_with('+') {
        // Phone number with + needs to be escaped or use tel: URI
        format!("tel:{}", target)
    } else {
        // Assume it's a phone number or username - need a host
        // For bare numbers/usernames, we need to add a default domain
        format!("sip:{}@localhost", target)
    };
    
    SessionControl::transfer_session(coordinator, session_id, &formatted_target).await
}

/// Bridge two calls together (3-way conference)
pub async fn bridge(
    coordinator: &Arc<SessionCoordinator>,
    session1: &SessionId,
    session2: &SessionId,
) -> Result<()> {
    // Use the existing bridge functionality
    let _bridge_id = super::create_bridge(coordinator, session1, session2).await?;
    tracing::info!("Successfully bridged sessions {} and {}", session1, session2);
    Ok(())
}

/// Terminate a call
pub async fn hangup(coordinator: &Arc<SessionCoordinator>, session_id: &SessionId) -> Result<()> {
    SessionControl::terminate_session(coordinator, session_id).await
}

/// Get packet loss rate for a call
pub async fn get_packet_loss_rate(
    coordinator: &Arc<SessionCoordinator>,
    session_id: &SessionId,
) -> Result<f32> {
    MediaControl::get_packet_loss_rate(coordinator, session_id)
        .await
        .map(|opt| opt.unwrap_or(0.0))
}

/// Get call quality score (MOS)
pub async fn get_quality_score(
    coordinator: &Arc<SessionCoordinator>,
    session_id: &SessionId,
) -> Result<f32> {
    MediaControl::get_call_quality_score(coordinator, session_id)
        .await
        .map(|opt| opt.unwrap_or(0.0))
}