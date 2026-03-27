//! NOTIFY message handler for transfer progress (RFC 3515) (v2)

use crate::adapters_v2::dialog_adapter::DialogAdapter;
use crate::state_table::types::SessionId;
use crate::transfer_v2::types::TransferProgress;
use std::sync::Arc;
use tracing::{debug, error, info};

/// Handler for sending NOTIFY messages during transfer
pub struct TransferNotifyHandler {
    dialog_adapter: Arc<DialogAdapter>,
}

impl TransferNotifyHandler {
    pub fn new(dialog_adapter: Arc<DialogAdapter>) -> Self {
        Self { dialog_adapter }
    }

    pub async fn send_notify(
        &self,
        transferor_session_id: &SessionId,
        progress: TransferProgress,
    ) -> Result<(), String> {
        let sipfrag = progress.to_sipfrag();
        let status_code = progress.status_code();

        debug!("Sending REFER NOTIFY to transferor session {} with progress: {} {}",
            transferor_session_id, status_code, sipfrag);

        match self.dialog_adapter.send_refer_notify(transferor_session_id, status_code, &sipfrag).await {
            Ok(_) => {
                info!("Sent REFER NOTIFY to transferor: {} {}", status_code, sipfrag);
                Ok(())
            }
            Err(e) => {
                error!("Failed to send REFER NOTIFY to transferor: {}", e);
                Err(format!("REFER NOTIFY send failed: {}", e))
            }
        }
    }

    pub async fn notify_trying(&self, transferor_session_id: &SessionId) -> Result<(), String> {
        self.send_notify(transferor_session_id, TransferProgress::Trying).await
    }

    pub async fn notify_ringing(&self, transferor_session_id: &SessionId) -> Result<(), String> {
        self.send_notify(transferor_session_id, TransferProgress::Ringing).await
    }

    pub async fn notify_success(&self, transferor_session_id: &SessionId) -> Result<(), String> {
        self.send_notify(transferor_session_id, TransferProgress::Success).await
    }

    pub async fn notify_failure(
        &self,
        transferor_session_id: &SessionId,
        status_code: u16,
        reason: &str,
    ) -> Result<(), String> {
        self.send_notify(
            transferor_session_id,
            TransferProgress::Failed(status_code, reason.to_string()),
        ).await
    }
}
