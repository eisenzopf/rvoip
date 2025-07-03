//! Dialog Types (parallel to MediaEngine/types)
//!
//! Type definitions for dialog integration, providing session-level abstractions
//! over dialog-core SIP operations.

use std::collections::HashMap;
use crate::api::types::SessionId;
use rvoip_dialog_core::{DialogId, api::{CallHandle, DialogHandle}};

/// Dialog mapping for session-to-dialog coordination
pub type SessionDialogMap = HashMap<SessionId, DialogId>;
pub type DialogSessionMap = HashMap<DialogId, SessionId>;

/// Dialog operation handle for session management
#[derive(Debug, Clone)]
pub struct SessionDialogHandle {
    pub session_id: SessionId,
    pub dialog_id: DialogId,
    pub call_handle: Option<CallHandle>,
    pub dialog_handle: Option<DialogHandle>,
}

impl SessionDialogHandle {
    pub fn new(session_id: SessionId, dialog_id: DialogId) -> Self {
        Self {
            session_id,
            dialog_id,
            call_handle: None,
            dialog_handle: None,
        }
    }
    
    pub fn with_call_handle(mut self, handle: CallHandle) -> Self {
        self.call_handle = Some(handle);
        self
    }
    
    pub fn with_dialog_handle(mut self, handle: DialogHandle) -> Self {
        self.dialog_handle = Some(handle);
        self
    }
}

/// Dialog operation result for async operations
#[derive(Debug, Clone)]
pub struct DialogOperationResult {
    pub session_id: SessionId,
    pub dialog_id: DialogId,
    pub success: bool,
    pub message: Option<String>,
}

/// Dialog state for session tracking
#[derive(Debug, Clone, PartialEq)]
pub enum DialogState {
    Creating,
    Early,
    Confirmed,
    Terminated,
    Failed(String),
}

/// Dialog session information
#[derive(Debug, Clone)]
pub struct DialogSession {
    pub session_id: SessionId,
    pub dialog_id: DialogId,
    pub state: DialogState,
    pub local_uri: String,
    pub remote_uri: String,
    pub created_at: std::time::Instant,
} 