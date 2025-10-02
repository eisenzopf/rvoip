//! NOTIFY message handler for transfer progress (RFC 3515)

use crate::adapters::dialog_adapter::DialogAdapter;
use crate::state_table::types::SessionId;
use crate::transfer::types::TransferProgress;
use std::sync::Arc;
use tracing::{debug, error, info};

/// Handler for sending NOTIFY messages during transfer
/// Per RFC 3515, the transferee should send NOTIFY messages
/// to the transferor reporting transfer progress
pub struct TransferNotifyHandler {
    dialog_adapter: Arc<DialogAdapter>,
}

impl TransferNotifyHandler {
    pub fn new(dialog_adapter: Arc<DialogAdapter>) -> Self {
        Self { dialog_adapter }
    }

    /// Send NOTIFY to transferor about transfer progress
    ///
    /// # Arguments
    /// * `transferor_session_id` - Session ID of the transferor (who sent REFER)
    /// * `progress` - Transfer progress to report
    ///
    /// # RFC 3515 Requirements
    /// The NOTIFY body should be "message/sipfrag" format:
    /// ```text
    /// NOTIFY sip:bob@example.com SIP/2.0
    /// Event: refer
    /// Subscription-State: active;expires=60
    /// Content-Type: message/sipfrag;version=2.0
    ///
    /// SIP/2.0 100 Trying
    /// ```
    pub async fn send_notify(
        &self,
        transferor_session_id: &SessionId,
        progress: TransferProgress,
    ) -> Result<(), String> {
        let sipfrag = progress.to_sipfrag();
        let status_code = progress.status_code();

        debug!(
            "Sending REFER NOTIFY to transferor session {} with progress: {} {}",
            transferor_session_id, status_code, sipfrag
        );

        // Send REFER NOTIFY via dialog adapter using RFC 3515 compliant method
        match self
            .dialog_adapter
            .send_refer_notify(transferor_session_id, status_code, &sipfrag)
            .await
        {
            Ok(_) => {
                info!(
                    "✅ Sent REFER NOTIFY to transferor: {} {} (with Subscription-State)",
                    status_code, sipfrag
                );
                Ok(())
            }
            Err(e) => {
                error!("Failed to send REFER NOTIFY to transferor: {}", e);
                Err(format!("REFER NOTIFY send failed: {}", e))
            }
        }
    }

    /// Send "100 Trying" NOTIFY
    pub async fn notify_trying(&self, transferor_session_id: &SessionId) -> Result<(), String> {
        self.send_notify(transferor_session_id, TransferProgress::Trying)
            .await
    }

    /// Send "180 Ringing" NOTIFY
    pub async fn notify_ringing(&self, transferor_session_id: &SessionId) -> Result<(), String> {
        self.send_notify(transferor_session_id, TransferProgress::Ringing)
            .await
    }

    /// Send "200 OK" NOTIFY (success)
    pub async fn notify_success(&self, transferor_session_id: &SessionId) -> Result<(), String> {
        self.send_notify(transferor_session_id, TransferProgress::Success)
            .await
    }

    /// Send failure NOTIFY
    pub async fn notify_failure(
        &self,
        transferor_session_id: &SessionId,
        status_code: u16,
        reason: &str,
    ) -> Result<(), String> {
        self.send_notify(
            transferor_session_id,
            TransferProgress::Failed(status_code, reason.to_string()),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transfer_progress_conversion() {
        assert_eq!(TransferProgress::Trying.to_sipfrag(), "SIP/2.0 100 Trying");
        assert_eq!(TransferProgress::Trying.status_code(), 100);

        assert_eq!(
            TransferProgress::Ringing.to_sipfrag(),
            "SIP/2.0 180 Ringing"
        );
        assert_eq!(TransferProgress::Ringing.status_code(), 180);

        assert_eq!(TransferProgress::Success.to_sipfrag(), "SIP/2.0 200 OK");
        assert_eq!(TransferProgress::Success.status_code(), 200);

        let failed = TransferProgress::Failed(404, "Not Found".to_string());
        assert_eq!(failed.to_sipfrag(), "SIP/2.0 404 Not Found");
        assert_eq!(failed.status_code(), 404);
    }
}
