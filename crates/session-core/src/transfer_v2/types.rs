//! Transfer types and options for all transfer implementations (v2)

use crate::state_table::types::SessionId;
use crate::state_table::types::DialogId;
use serde::{Deserialize, Serialize};

/// Options for configuring transfer behavior
#[derive(Debug, Clone, Default)]
pub struct TransferOptions {
    pub replaces_header: Option<String>,
    pub wait_for_establishment: bool,
    pub terminate_old_call: bool,
    pub send_notify: bool,
    pub transferor_session_id: Option<SessionId>,
    pub establishment_timeout_ms: u64,
}

impl TransferOptions {
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

    pub fn managed_consultation() -> Self {
        Self {
            replaces_header: None,
            wait_for_establishment: true,
            terminate_old_call: false,
            send_notify: false,
            transferor_session_id: None,
            establishment_timeout_ms: 30000,
        }
    }

    pub fn with_transferor_session(mut self, session_id: SessionId) -> Self {
        self.transferor_session_id = Some(session_id);
        self
    }

    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.establishment_timeout_ms = timeout_ms;
        self
    }
}

/// Result of a transfer operation
#[derive(Debug, Clone)]
pub struct TransferResult {
    pub new_session_id: SessionId,
    pub new_dialog_id: Option<DialogId>,
    pub success: bool,
    pub status_message: String,
    pub sip_status_code: Option<u16>,
}

impl TransferResult {
    pub fn success(new_session_id: SessionId, new_dialog_id: Option<DialogId>) -> Self {
        Self {
            new_session_id,
            new_dialog_id,
            success: true,
            status_message: "Transfer completed successfully".to_string(),
            sip_status_code: Some(200),
        }
    }

    pub fn failure(new_session_id: SessionId, error: String, status_code: Option<u16>) -> Self {
        Self {
            new_session_id,
            new_dialog_id: None,
            success: false,
            status_message: error,
            sip_status_code: status_code,
        }
    }

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
    Trying,
    Ringing,
    Success,
    Failed(u16, String),
}

impl TransferProgress {
    pub fn to_sipfrag(&self) -> String {
        match self {
            TransferProgress::Trying => "SIP/2.0 100 Trying".to_string(),
            TransferProgress::Ringing => "SIP/2.0 180 Ringing".to_string(),
            TransferProgress::Success => "SIP/2.0 200 OK".to_string(),
            TransferProgress::Failed(code, reason) => format!("SIP/2.0 {} {}", code, reason),
        }
    }

    pub fn status_code(&self) -> u16 {
        match self {
            TransferProgress::Trying => 100,
            TransferProgress::Ringing => 180,
            TransferProgress::Success => 200,
            TransferProgress::Failed(code, _) => *code,
        }
    }
}
