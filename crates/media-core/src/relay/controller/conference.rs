//! Conference mixing functionality
//!
//! This module provides conference audio mixing capabilities for
//! multi-party calls.

use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info};

use crate::error::{Error, Result};
use crate::types::{DialogId, AudioFrame};
use crate::types::conference::{
    ParticipantId, AudioStream, ConferenceMixingConfig, ConferenceMixingStats,
    ConferenceMixingEvent,
};
use crate::processing::audio::AudioMixer;

use super::{MediaSessionController, MediaSessionStatus};

impl MediaSessionController {
    /// Enable conference audio mixing with the given configuration
    pub async fn enable_conference_mixing(&mut self, config: ConferenceMixingConfig) -> Result<()> {
        info!("🎙️ Enabling conference audio mixing");
        
        if self.audio_mixer.is_some() {
            return Err(Error::config("Conference mixing already enabled"));
        }
        
        // Create audio mixer
        let audio_mixer = Arc::new(AudioMixer::new(config.clone()).await
            .map_err(|e| Error::config(format!("Failed to create audio mixer: {}", e)))?);
        
        // Set up event forwarding
        audio_mixer.set_event_sender(self.conference_event_tx.clone()).await;
        
        self.audio_mixer = Some(audio_mixer);
        self.conference_config = config;
        
        info!("✅ Conference audio mixing enabled");
        Ok(())
    }
    
    /// Disable conference audio mixing
    pub async fn disable_conference_mixing(&mut self) -> Result<()> {
        info!("🔇 Disabling conference audio mixing");
        
        if let Some(mixer) = &self.audio_mixer {
            // Clean up all participants
            let participants = mixer.get_active_participants().await
                .map_err(|e| Error::config(format!("Failed to get participants: {}", e)))?;
            
            for participant_id in participants {
                let _ = mixer.remove_audio_stream(&participant_id).await;
            }
        }
        
        self.audio_mixer = None;
        
        info!("✅ Conference audio mixing disabled");
        Ok(())
    }
    
    /// Add a dialog to the conference (participant joins)
    pub async fn add_to_conference(&self, dialog_id: &str) -> Result<()> {
        info!("🎤 Adding dialog {} to conference", dialog_id);
        
        let mixer = self.audio_mixer.as_ref()
            .ok_or_else(|| Error::config("Conference mixing not enabled"))?;
        
        // Convert to DialogId for session lookup
        let dialog_id_typed = DialogId::new(dialog_id);
        
        // Check if session exists
        let session_info = self.get_session_info(&dialog_id_typed).await
            .ok_or_else(|| Error::session_not_found(dialog_id.to_string()))?;
        
        if session_info.status != MediaSessionStatus::Active {
            return Err(Error::config(format!(
                "Cannot add inactive session to conference: {}", dialog_id
            )));
        }
        
        // Create audio stream for this participant
        let participant_id = ParticipantId::new(dialog_id);
        let audio_stream = AudioStream::new(
            participant_id.clone(),
            self.conference_config.output_sample_rate,
            self.conference_config.output_channels,
        );
        
        // Add to mixer
        mixer.add_audio_stream(participant_id, audio_stream).await
            .map_err(|e| Error::config(format!("Failed to add to conference: {}", e)))?;
        
        // Flush events to ensure synchronous delivery for testing
        mixer.flush_events().await;
        
        info!("✅ Added dialog {} to conference", dialog_id);
        Ok(())
    }
    
    /// Remove a dialog from the conference (participant leaves)
    pub async fn remove_from_conference(&self, dialog_id: &str) -> Result<()> {
        info!("�� Removing dialog {} from conference", dialog_id);
        
        let mixer = self.audio_mixer.as_ref()
            .ok_or_else(|| Error::config("Conference mixing not enabled"))?;
        
        let participant_id = ParticipantId::new(dialog_id);
        
        // Validate that participant exists before attempting removal
        let active_participants = mixer.get_active_participants().await
            .map_err(|e| Error::config(format!("Failed to get participants: {}", e)))?;
        
        if !active_participants.contains(&participant_id) {
            return Err(Error::config(format!(
                "Participant {} not found in conference", dialog_id
            )));
        }
        
        // Remove from mixer
        mixer.remove_audio_stream(&participant_id).await
            .map_err(|e| Error::config(format!("Failed to remove from conference: {}", e)))?;
        
        // Flush events to ensure synchronous delivery for testing
        mixer.flush_events().await;
        
        info!("✅ Removed dialog {} from conference", dialog_id);
        Ok(())
    }
    
    /// Process incoming audio for conference mixing
    pub async fn process_conference_audio(&self, dialog_id: &str, audio_frame: AudioFrame) -> Result<()> {
        let mixer = self.audio_mixer.as_ref()
            .ok_or_else(|| Error::config("Conference mixing not enabled"))?;
        
        let participant_id = ParticipantId::new(dialog_id);
        
        // Validate that participant exists in conference
        let active_participants = mixer.get_active_participants().await
            .map_err(|e| Error::config(format!("Failed to get participants: {}", e)))?;
        
        if !active_participants.contains(&participant_id) {
            return Err(Error::config(format!(
                "Participant {} not found in conference", dialog_id
            )));
        }
        
        // Process audio through mixer
        mixer.process_audio_frame(&participant_id, audio_frame).await
            .map_err(|e| Error::config(format!("Failed to process conference audio: {}", e)))?;
        
        // Trigger mixing if we have enough participants
        if active_participants.len() >= 2 {
            let empty_inputs = Vec::new(); // AudioMixer gets its inputs from stream manager
            let _mixed_outputs = mixer.mix_participants(&empty_inputs).await
                .map_err(|e| Error::config(format!("Failed to perform mixing: {}", e)))?;
        }
        
        Ok(())
    }
    
    /// Get mixed audio for a specific participant (everyone except themselves)
    pub async fn get_conference_mixed_audio(&self, dialog_id: &str) -> Result<Option<AudioFrame>> {
        let mixer = self.audio_mixer.as_ref()
            .ok_or_else(|| Error::config("Conference mixing not enabled"))?;
        
        let participant_id = ParticipantId::new(dialog_id);
        
        // Validate that participant exists in conference
        let active_participants = mixer.get_active_participants().await
            .map_err(|e| Error::config(format!("Failed to get participants: {}", e)))?;
        
        if !active_participants.contains(&participant_id) {
            return Err(Error::config(format!(
                "Participant {} not found in conference", dialog_id
            )));
        }
        
        // Get mixed audio from mixer
        mixer.get_mixed_audio(&participant_id).await
            .map_err(|e| Error::config(format!("Failed to get mixed audio: {}", e)))
    }
    
    /// Get list of conference participants
    pub async fn get_conference_participants(&self) -> Result<Vec<String>> {
        let mixer = self.audio_mixer.as_ref()
            .ok_or_else(|| Error::config("Conference mixing not enabled"))?;
        
        let participants = mixer.get_active_participants().await
            .map_err(|e| Error::config(format!("Failed to get participants: {}", e)))?;
        
        Ok(participants.into_iter().map(|p| p.0).collect())
    }
    
    /// Get conference mixing statistics
    pub async fn get_conference_stats(&self) -> Result<ConferenceMixingStats> {
        let mixer = self.audio_mixer.as_ref()
            .ok_or_else(|| Error::config("Conference mixing not enabled"))?;
        
        mixer.get_mixing_stats().await
            .map_err(|e| Error::config(format!("Failed to get conference stats: {}", e)))
    }
    
    /// Get conference event receiver (can only be called once)
    pub async fn take_conference_event_receiver(&self) -> Option<mpsc::UnboundedReceiver<ConferenceMixingEvent>> {
        let mut event_rx = self.conference_event_rx.write().await;
        event_rx.take()
    }
    
    /// Check if conference mixing is enabled
    pub fn is_conference_mixing_enabled(&self) -> bool {
        self.audio_mixer.is_some()
    }
    
    /// Clean up inactive conference participants
    pub async fn cleanup_conference_participants(&self) -> Result<Vec<String>> {
        let mixer = self.audio_mixer.as_ref()
            .ok_or_else(|| Error::config("Conference mixing not enabled"))?;
        
        let removed = mixer.cleanup_inactive_participants().await
            .map_err(|e| Error::config(format!("Failed to cleanup participants: {}", e)))?;
        
        Ok(removed.into_iter().map(|p| p.0).collect())
    }
} 