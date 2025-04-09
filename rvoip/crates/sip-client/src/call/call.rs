// Re-export the Call struct implementation from the new module structure

pub(crate) use super::call_struct::Call;

// The implementation is now split across multiple files:
// - call_struct.rs - The struct definition and constructor
// - api.rs - Public API (answer, reject, hangup, send_dtmf)
// - sip_handlers.rs - SIP protocol handling (handle_request, handle_response)
// - state.rs - Call state management
// - media.rs - Media setup and management
// - dialog.rs - SIP dialog management 