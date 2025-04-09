use std::sync::Arc;

use tracing::{debug, warn};

use rvoip_session_core::dialog::{Dialog, DialogState};

use crate::error::{Error, Result};

use super::call_struct::Call;

impl Call {
    /// Save dialog information to the registry
    async fn save_dialog_to_registry(&self) -> Result<()> {
        if let Some(registry) = self.registry_ref().read().await.clone() {
            if let Some(dialog) = self.dialog_ref().read().await.clone() {
                debug!("Saving dialog {} to registry", dialog.id);
                
                // Get dialog sequence numbers and target
                let local_seq = *self.cseq_ref().lock().await;
                let remote_seq = 0; // TODO: Get remote sequence number from dialog
                
                // Use remote target from dialog or fall back to remote URI
                let remote_target = dialog.remote_uri.clone(); // No optional remote_target field

                // Find call registry interface to update dialog
                registry.update_dialog_info(
                    &dialog.id.to_string(),
                    Some(dialog.call_id.clone()),
                    Some(dialog.state.to_string()),
                    Some(dialog.local_tag.clone().unwrap_or_default()),
                    Some(dialog.remote_tag.clone().unwrap_or_default()),
                    Some(local_seq),
                    Some(remote_seq),
                    None, // route_set
                    Some(remote_target.to_string()),
                    Some(dialog.local_uri.scheme.to_string() == "sips")
                ).await?;
                
                return Ok(());
            }
            
            // No dialog found for this call
            debug!("No dialog found for call {}, not saving to registry", self.id());
            Ok(())
        } else {
            // No registry set
            debug!("No registry set for call {}, not saving dialog", self.id());
            Ok(())
        }
    }
    
    /// Set the dialog for this call and save it to the registry
    pub async fn set_dialog(&self, dialog: Dialog) -> Result<()> {
        // Update the dialog field
        *self.dialog_ref().write().await = Some(dialog);
        
        // Save dialog to registry
        self.save_dialog_to_registry().await?;
        
        Ok(())
    }
} 