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
    // Media session should already be ready when this is called
    
    // Create channels for bidirectional audio with larger buffers
    let (tx_to_remote, mut rx_from_app) = mpsc::channel::<AudioFrame>(1000);
    let (tx_to_app, rx_from_remote) = mpsc::channel::<AudioFrame>(1000);
    
    // Subscribe to incoming audio from remote
    let mut audio_subscriber = MediaControl::subscribe_to_audio_frames(coordinator, session_id).await?;
    
    // Task to forward incoming audio to the application - made resilient
    let session_id_clone = session_id.clone();
    tokio::spawn(async move {
        tracing::info!("Audio receiver task started for session {}", session_id_clone);
        let mut frame_count = 0;
        let mut consecutive_failures = 0;
        
        loop {
            match tokio::time::timeout(tokio::time::Duration::from_secs(1), audio_subscriber.recv()).await {
                Ok(Some(frame)) => {
                    frame_count += 1;
                    consecutive_failures = 0;
                    if frame_count <= 5 || frame_count % 50 == 0 {
                        tracing::debug!("Received frame #{} for session {}", frame_count, session_id_clone);
                    }
                    if tx_to_app.send(frame).await.is_err() {
                        tracing::info!("Audio receiver ended - app closed channel after {} frames", frame_count);
                        break;
                    }
                }
                Ok(None) => {
                    tracing::warn!("Audio subscriber closed after {} frames for session {}", frame_count, session_id_clone);
                    break;
                }
                Err(_) => {
                    // Timeout - normal during silence or setup
                    consecutive_failures += 1;
                    if consecutive_failures > 30 {  // 30 seconds of no audio
                        tracing::info!("Audio receiver timeout after {} frames for session {}", frame_count, session_id_clone);
                        break;
                    }
                    // Continue waiting for frames
                }
            }
        }
        tracing::info!("Audio receiver task completed for session {} - total frames: {}", session_id_clone, frame_count);
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