//! B2BUA-side transfer orchestration helpers.
//!
//! Per CARVE_PLAN §3 (transfer row): the actual SIP REFER mechanics
//! (the `refer(...)` builder, `accept_refer`, `transfer_attended`, etc.)
//! already live in [`UnifiedCoordinator`] and are NOT re-implemented here.
//! This module provides B2BUA-side scenario glue — pick which leg, build
//! the target, choose blind vs attended — that calls into those methods.

use std::sync::Arc;

use crate::api::unified::UnifiedCoordinator;
use crate::SessionId;
use thiserror::Error;

/// Error returned by the B2BUA transfer helpers.
#[derive(Debug, Error)]
pub enum TransferError {
    /// The transfer (REFER send or accept) failed.
    #[error("transfer failed: {0}")]
    Failed(String),
}

impl From<crate::errors::SessionError> for TransferError {
    fn from(err: crate::errors::SessionError) -> Self {
        TransferError::Failed(err.to_string())
    }
}

/// Blind transfer: send a REFER on `source_session` pointing at `target_uri`.
/// The transferee dials `target_uri` directly; no Replaces header involved.
/// (RFC 3515.)
pub async fn blind_transfer(
    coordinator: &Arc<UnifiedCoordinator>,
    source_session: &SessionId,
    target_uri: &str,
) -> Result<(), TransferError> {
    coordinator
        .refer(source_session, target_uri.to_string())
        .send()
        .await
        .map_err(Into::into)
}

/// Attended transfer with Replaces (RFC 3891 / RFC 5589): send a REFER on
/// `source_session` whose Refer-To header carries a Replaces parameter
/// referencing `replacing_session`. Caller is expected to have already
/// established `replacing_session` and `target_uri` should encode the
/// transferee leg appropriately.
pub async fn attended_transfer(
    coordinator: &Arc<UnifiedCoordinator>,
    source_session: &SessionId,
    target_uri: &str,
    replaces: &str,
) -> Result<(), TransferError> {
    coordinator
        .refer(source_session, target_uri.to_string())
        .with_replaces(replaces.to_string())
        .send()
        .await
        .map_err(Into::into)
}

/// Accept an inbound REFER on `session_id`. Triggers the
/// downstream-call-and-NOTIFY flow per RFC 3515.
pub async fn accept_inbound_refer(
    coordinator: &UnifiedCoordinator,
    session_id: &SessionId,
) -> Result<(), TransferError> {
    coordinator
        .accept_refer(session_id)
        .await
        .map_err(Into::into)
}
