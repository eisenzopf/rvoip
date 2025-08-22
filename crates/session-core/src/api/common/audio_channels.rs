//! Audio channel setup utilities for UAC/UAS APIs

use std::sync::Arc;
use tokio::sync::mpsc;
use crate::api::types::{SessionId, AudioFrame};
use crate::api::media::MediaControl;
use crate::coordinator::SessionCoordinator;
use crate::errors::Result;

/// Setup bidirectional audio channels for a call session
/// 
/// Returns (tx, rx) where:
/// - tx: Send audio frames to remote party
/// - rx: Receive audio frames from remote party
pub async fn setup_audio_channels(
    coordinator: &Arc<SessionCoordinator>,
    session_id: &SessionId,
) -> Result<(mpsc::Sender<AudioFrame>, mpsc::Receiver<AudioFrame>)> {
    // Create channels for bidirectional audio
    let (tx_to_remote, mut rx_from_app) = mpsc::channel::<AudioFrame>(100);
    let (tx_to_app, rx_from_remote) = mpsc::channel::<AudioFrame>(100);
    
    // Subscribe to incoming audio from remote
    let mut audio_subscriber = MediaControl::subscribe_to_audio_frames(coordinator, session_id).await?;
    
    // Task to forward incoming audio to the application
    let session_id_clone = session_id.clone();
    tokio::spawn(async move {
        tracing::debug!("Audio receiver task started for session {}", session_id_clone);
        while let Some(frame) = audio_subscriber.recv().await {
            if tx_to_app.send(frame).await.is_err() {
                tracing::debug!("Audio receiver task ended for session {} - channel closed", session_id_clone);
                break;
            }
        }
    });
    
    // Task to forward outgoing audio from the application
    let coordinator_clone = coordinator.clone();
    let session_id_clone = session_id.clone();
    tokio::spawn(async move {
        tracing::debug!("Audio sender task started for session {}", session_id_clone);
        while let Some(frame) = rx_from_app.recv().await {
            if let Err(e) = MediaControl::send_audio_frame(&coordinator_clone, &session_id_clone, frame).await {
                tracing::warn!("Failed to send audio frame for session {}: {}", session_id_clone, e);
            }
        }
        tracing::debug!("Audio sender task ended for session {}", session_id_clone);
    });
    
    Ok((tx_to_remote, rx_from_remote))
}

/// Cleanup audio channels when call ends
pub async fn cleanup_audio_channels(
    coordinator: &Arc<SessionCoordinator>,
    session_id: &SessionId,
) -> Result<()> {
    // Stop audio streaming
    MediaControl::stop_audio_stream(coordinator, session_id).await?;
    Ok(())
}