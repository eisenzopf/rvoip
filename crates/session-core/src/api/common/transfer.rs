//! Call transfer utilities

use crate::api::types::SessionId;
use crate::api::control::SessionControl;
use crate::coordinator::SessionCoordinator;
use crate::errors::Result;
use std::sync::Arc;

/// Transfer types
#[derive(Debug, Clone)]
pub enum TransferType {
    /// Blind transfer (immediate)
    Blind(String),
    /// Attended transfer (with consultation)
    Attended {
        target: String,
        consultation_session: SessionId,
    },
}

/// Perform a call transfer
pub async fn transfer_call(
    coordinator: &Arc<SessionCoordinator>,
    session_id: &SessionId,
    transfer_type: TransferType,
) -> Result<()> {
    match transfer_type {
        TransferType::Blind(target) => {
            SessionControl::transfer_session(coordinator, session_id, &target).await
        }
        TransferType::Attended { target, consultation_session } => {
            // For attended transfer, we would bridge the original call with the consultation
            // This requires more complex signaling that will be implemented later
            // For now, fall back to blind transfer
            SessionControl::transfer_session(coordinator, session_id, &target).await
        }
    }
}