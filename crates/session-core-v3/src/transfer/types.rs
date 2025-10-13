//! Transfer types and options for all transfer implementations

use crate::state_table::types::SessionId;
use crate::types::DialogId;
use serde::{Deserialize, Serialize};

/// Options for configuring transfer behavior
/// Used by TransferCoordinator to handle all three transfer types
#[derive(Debug, Clone, Default)]
pub struct TransferOptions {
    /// For attended transfer - include Replaces header in INVITE
    /// Format: "call-id;to-tag=x;from-tag=y"
    pub replaces_header: Option<String>,

    /// Wait for new call to reach Active state before proceeding
    /// - Blind transfer: false (fire and forget)
    /// - Attended transfer: true (wait for confirmation)
    /// - Managed transfer: true (need consultation call established)
    pub wait_for_establishment: bool,

    /// Terminate the old call after transfer completes
    /// - Blind transfer: true
    /// - Attended transfer: true
    /// - Managed transfer: false (keep for conference)
    pub terminate_old_call: bool,

    /// Send NOTIFY messages to transferor about progress
    /// Per RFC 3515, should send NOTIFY with SIP fragments
    pub send_notify: bool,

    /// Session ID of the transferor (to send NOTIFY messages to)
    /// Only needed if send_notify is true
    pub transferor_session_id: Option<SessionId>,

    /// Maximum time to wait for call establishment (milliseconds)
    /// Default: 30 seconds
    pub establishment_timeout_ms: u64,
}

impl TransferOptions {
    /// Create options for blind transfer (immediate, no waiting)
    pub fn blind() -> Self {
        Self {
            replaces_header: None,
            wait_for_establishment: false,
            terminate_old_call: true,
            send_notify: true,
            transferor_session_id: None,
            establishment_timeout_ms: 30000,
        }
    }

    /// Create options for attended transfer (wait for establishment)
    pub fn attended(replaces_header: String) -> Self {
        Self {
            replaces_header: Some(replaces_header),
            wait_for_establishment: true,
            terminate_old_call: true,
            send_notify: true,
            transferor_session_id: None,
            establishment_timeout_ms: 30000,
        }
    }

    /// Create options for managed transfer consultation call
    pub fn managed_consultation() -> Self {
        Self {
            replaces_header: None,
            wait_for_establishment: true,
            terminate_old_call: false, // Keep original call for conference
            send_notify: false,
            transferor_session_id: None,
            establishment_timeout_ms: 30000,
        }
    }

    /// Set the transferor session ID for NOTIFY messages
    pub fn with_transferor_session(mut self, session_id: SessionId) -> Self {
        self.transferor_session_id = Some(session_id);
        self
    }

    /// Set custom establishment timeout
    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.establishment_timeout_ms = timeout_ms;
        self
    }
}

/// Result of a transfer operation
#[derive(Debug, Clone)]
pub struct TransferResult {
    /// New session ID created for transfer target
    pub new_session_id: SessionId,

    /// New dialog ID (if dialog was created)
    pub new_dialog_id: Option<DialogId>,

    /// Transfer success/failure
    pub success: bool,

    /// Human-readable status message
    pub status_message: String,

    /// SIP status code (for NOTIFY sipfrag)
    pub sip_status_code: Option<u16>,
}

impl TransferResult {
    /// Create success result
    pub fn success(new_session_id: SessionId, new_dialog_id: Option<DialogId>) -> Self {
        Self {
            new_session_id,
            new_dialog_id,
            success: true,
            status_message: "Transfer completed successfully".to_string(),
            sip_status_code: Some(200),
        }
    }

    /// Create failure result
    pub fn failure(new_session_id: SessionId, error: String, status_code: Option<u16>) -> Self {
        Self {
            new_session_id,
            new_dialog_id: None,
            success: false,
            status_message: error,
            sip_status_code: status_code,
        }
    }

    /// Create in-progress result
    pub fn in_progress(new_session_id: SessionId, message: String) -> Self {
        Self {
            new_session_id,
            new_dialog_id: None,
            success: false,
            status_message: message,
            sip_status_code: Some(100),
        }
    }
}

/// Transfer progress for NOTIFY messages (RFC 3515)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransferProgress {
    /// 100 Trying - transfer target being contacted
    Trying,
    /// 180 Ringing - transfer target is ringing
    Ringing,
    /// 200 OK - transfer successful
    Success,
    /// 4xx/5xx/6xx - transfer failed
    Failed(u16, String),
}

impl TransferProgress {
    /// Convert to SIP fragment for NOTIFY body
    /// Format: "SIP/2.0 200 OK"
    pub fn to_sipfrag(&self) -> String {
        match self {
            TransferProgress::Trying => "SIP/2.0 100 Trying".to_string(),
            TransferProgress::Ringing => "SIP/2.0 180 Ringing".to_string(),
            TransferProgress::Success => "SIP/2.0 200 OK".to_string(),
            TransferProgress::Failed(code, reason) => {
                format!("SIP/2.0 {} {}", code, reason)
            }
        }
    }

    /// Get status code
    pub fn status_code(&self) -> u16 {
        match self {
            TransferProgress::Trying => 100,
            TransferProgress::Ringing => 180,
            TransferProgress::Success => 200,
            TransferProgress::Failed(code, _) => *code,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blind_transfer_options() {
        let opts = TransferOptions::blind();
        assert_eq!(opts.replaces_header, None);
        assert_eq!(opts.wait_for_establishment, false);
        assert_eq!(opts.terminate_old_call, true);
        assert_eq!(opts.send_notify, true);
    }

    #[test]
    fn test_attended_transfer_options() {
        let opts = TransferOptions::attended("call-id;to-tag=x;from-tag=y".to_string());
        assert!(opts.replaces_header.is_some());
        assert_eq!(opts.wait_for_establishment, true);
        assert_eq!(opts.terminate_old_call, true);
    }

    #[test]
    fn test_managed_consultation_options() {
        let opts = TransferOptions::managed_consultation();
        assert_eq!(opts.replaces_header, None);
        assert_eq!(opts.wait_for_establishment, true);
        assert_eq!(opts.terminate_old_call, false); // Keep for conference
    }

    #[test]
    fn test_transfer_progress_sipfrag() {
        assert_eq!(TransferProgress::Trying.to_sipfrag(), "SIP/2.0 100 Trying");
        assert_eq!(TransferProgress::Ringing.to_sipfrag(), "SIP/2.0 180 Ringing");
        assert_eq!(TransferProgress::Success.to_sipfrag(), "SIP/2.0 200 OK");
        assert_eq!(
            TransferProgress::Failed(404, "Not Found".to_string()).to_sipfrag(),
            "SIP/2.0 404 Not Found"
        );
    }
}
