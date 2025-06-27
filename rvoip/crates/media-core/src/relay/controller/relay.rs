//! Media relay functionality
//!
//! This module handles the creation and management of media relay sessions
//! between dialogs for RTP packet forwarding.

use std::sync::Arc;
use tracing::info;

use crate::error::{Error, Result};
use crate::types::DialogId;
use crate::relay::{MediaRelay, generate_session_id, create_relay_config, MediaSessionStatus};

use super::MediaSessionController;

impl MediaSessionController {
    /// Create relay between two dialogs
    pub async fn create_relay(&self, dialog_a: String, dialog_b: String) -> Result<()> {
        info!("Creating relay between dialogs: {} <-> {}", dialog_a, dialog_b);

        // Verify both sessions exist and get their configs
        let (session_a_config, session_b_config) = {
            let sessions = self.sessions.read().await;
            let dialog_a_id = DialogId::new(dialog_a.clone());
            let dialog_b_id = DialogId::new(dialog_b.clone());
            let session_a = sessions.get(&dialog_a_id)
                .ok_or_else(|| Error::session_not_found(dialog_a.clone()))?;
            let session_b = sessions.get(&dialog_b_id)
                .ok_or_else(|| Error::session_not_found(dialog_b.clone()))?;
            (session_a.config.clone(), session_b.config.clone())
        };
        
        // Generate relay session IDs
        let relay_session_a = generate_session_id();
        let relay_session_b = generate_session_id();
        
        // Create relay configuration
        let relay_config = create_relay_config(
            relay_session_a.clone(),
            relay_session_b.clone(),
            session_a_config.local_addr,
            session_b_config.local_addr,
        );
        
        // Create the relay session pair if relay is available
        if let Some(relay) = &self.relay {
            relay.create_session_pair(relay_config).await?;
        }
        
        // Update session infos with relay session IDs
        {
            let mut sessions = self.sessions.write().await;
            let dialog_a_id = DialogId::new(dialog_a.clone());
            let dialog_b_id = DialogId::new(dialog_b.clone());
            if let Some(session_a_info) = sessions.get_mut(&dialog_a_id) {
                session_a_info.relay_session_ids = Some((relay_session_a.clone(), relay_session_b.clone()));
                session_a_info.status = MediaSessionStatus::Active;
            }
            if let Some(session_b_info) = sessions.get_mut(&dialog_b_id) {
                session_b_info.relay_session_ids = Some((relay_session_b, relay_session_a));
                session_b_info.status = MediaSessionStatus::Active;
            }
        }
        
        info!("Media relay created between dialogs: {} <-> {}", dialog_a, dialog_b);
        Ok(())
    }
    
    /// Get media relay reference (for advanced usage)
    pub fn relay(&self) -> Option<&Arc<MediaRelay>> {
        self.relay.as_ref()
    }
} 