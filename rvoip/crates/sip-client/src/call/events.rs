use std::sync::Arc;

use rvoip_sip_core::Response;

use super::call_struct::Call;
use crate::media::MediaType;
use super::types::CallState;

/// Call events
#[derive(Debug, Clone)]
pub enum CallEvent {
    /// Event system is ready
    Ready,
    
    /// Incoming call received
    IncomingCall(Arc<Call>),
    
    /// Call state changed
    StateChanged {
        /// Call instance
        call: Arc<Call>,
        /// Previous state
        previous: CallState,
        /// New state
        current: CallState,
    },
    
    /// Media added to call
    MediaAdded {
        /// Call instance
        call: Arc<Call>,
        /// Media type
        media_type: MediaType,
    },
    
    /// Media removed from call
    MediaRemoved {
        /// Call instance
        call: Arc<Call>,
        /// Media type
        media_type: MediaType,
    },
    
    /// DTMF digit received
    DtmfReceived {
        /// Call instance
        call: Arc<Call>,
        /// DTMF digit
        digit: char,
    },
    
    /// Call terminated
    Terminated {
        /// Call instance
        call: Arc<Call>,
        /// Reason for termination
        reason: String,
    },
    
    /// Response received for a call
    ResponseReceived {
        /// Call instance
        call: Arc<Call>,
        /// Response received
        response: Response,
        /// Transaction ID
        transaction_id: String,
    },
    
    /// Error occurred
    Error {
        /// Call instance
        call: Arc<Call>,
        /// Error description
        error: String,
    },
} 