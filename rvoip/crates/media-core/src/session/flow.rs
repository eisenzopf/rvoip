//! Media flow control
//!
//! This module handles the control flow for media streams,
//! including start/stop, pause/resume, and mute/unmute operations.

use std::sync::Arc;
use tokio::sync::RwLock;

use crate::error::{Error, Result};
use super::{MediaDirection, MediaState};

/// Media flow control
///
/// Controls the flow of media data in a session, including 
/// operations like start, stop, pause, resume, and mute.
#[derive(Debug)]
pub struct MediaFlow {
    /// Current media flow state
    state: RwLock<MediaFlowState>,
    
    /// Whether audio input is muted
    input_muted: RwLock<bool>,
    
    /// Whether audio output is muted
    output_muted: RwLock<bool>,
}

/// Media flow state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MediaFlowState {
    /// Media flow is stopped
    Stopped,
    
    /// Media flow is active
    Active,
    
    /// Media flow is paused
    Paused,
}

impl MediaFlow {
    /// Create a new media flow controller
    pub fn new() -> Self {
        Self {
            state: RwLock::new(MediaFlowState::Stopped),
            input_muted: RwLock::new(false),
            output_muted: RwLock::new(false),
        }
    }
    
    /// Start media flow
    pub async fn start(&self) -> Result<()> {
        let mut state = self.state.write().await;
        
        match *state {
            MediaFlowState::Stopped => {
                *state = MediaFlowState::Active;
                Ok(())
            },
            MediaFlowState::Paused => {
                *state = MediaFlowState::Active;
                Ok(())
            },
            MediaFlowState::Active => {
                // Already active, nothing to do
                Ok(())
            }
        }
    }
    
    /// Stop media flow
    pub async fn stop(&self) -> Result<()> {
        let mut state = self.state.write().await;
        *state = MediaFlowState::Stopped;
        Ok(())
    }
    
    /// Pause media flow
    pub async fn pause(&self) -> Result<()> {
        let mut state = self.state.write().await;
        
        match *state {
            MediaFlowState::Active => {
                *state = MediaFlowState::Paused;
                Ok(())
            },
            MediaFlowState::Paused => {
                // Already paused, nothing to do
                Ok(())
            },
            MediaFlowState::Stopped => {
                Err(Error::InvalidState("Cannot pause stopped media flow".into()))
            }
        }
    }
    
    /// Resume paused media flow
    pub async fn resume(&self) -> Result<()> {
        let mut state = self.state.write().await;
        
        match *state {
            MediaFlowState::Paused => {
                *state = MediaFlowState::Active;
                Ok(())
            },
            MediaFlowState::Active => {
                // Already active, nothing to do
                Ok(())
            },
            MediaFlowState::Stopped => {
                Err(Error::InvalidState("Cannot resume stopped media flow".into()))
            }
        }
    }
    
    /// Mute input (microphone)
    pub async fn mute_input(&self) -> Result<()> {
        let mut muted = self.input_muted.write().await;
        *muted = true;
        Ok(())
    }
    
    /// Unmute input (microphone)
    pub async fn unmute_input(&self) -> Result<()> {
        let mut muted = self.input_muted.write().await;
        *muted = false;
        Ok(())
    }
    
    /// Mute output (speaker)
    pub async fn mute_output(&self) -> Result<()> {
        let mut muted = self.output_muted.write().await;
        *muted = true;
        Ok(())
    }
    
    /// Unmute output (speaker)
    pub async fn unmute_output(&self) -> Result<()> {
        let mut muted = self.output_muted.write().await;
        *muted = false;
        Ok(())
    }
    
    /// Get the current flow state
    pub async fn is_active(&self) -> bool {
        *self.state.read().await == MediaFlowState::Active
    }
    
    /// Get the current flow state
    pub async fn is_paused(&self) -> bool {
        *self.state.read().await == MediaFlowState::Paused
    }
    
    /// Get the current flow state
    pub async fn is_stopped(&self) -> bool {
        *self.state.read().await == MediaFlowState::Stopped
    }
    
    /// Check if input is muted
    pub async fn is_input_muted(&self) -> bool {
        *self.input_muted.read().await
    }
    
    /// Check if output is muted
    pub async fn is_output_muted(&self) -> bool {
        *self.output_muted.read().await
    }
    
    /// Convert flow state to media direction
    pub async fn to_direction(&self) -> MediaDirection {
        let state = *self.state.read().await;
        let input_muted = *self.input_muted.read().await;
        let output_muted = *self.output_muted.read().await;
        
        match state {
            MediaFlowState::Stopped => MediaDirection::Inactive,
            MediaFlowState::Paused => MediaDirection::Inactive,
            MediaFlowState::Active => {
                match (input_muted, output_muted) {
                    (true, true) => MediaDirection::Inactive,
                    (true, false) => MediaDirection::RecvOnly,
                    (false, true) => MediaDirection::SendOnly,
                    (false, false) => MediaDirection::SendRecv,
                }
            }
        }
    }
} 