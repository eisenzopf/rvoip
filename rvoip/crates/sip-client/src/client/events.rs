use crate::call::CallEvent;

/// Event types emitted by the SIP client
#[derive(Debug, Clone)]
pub enum SipClientEvent {
    /// Call-related event
    Call(CallEvent),
    
    /// Registration state changed
    RegistrationState {
        /// Is the client registered
        registered: bool,
        
        /// Registration server
        server: String,
        
        /// Registration expiry in seconds
        expires: Option<u32>,
        
        /// Error message if registration failed
        error: Option<String>,
    },
    
    /// Client error
    Error(String),
} 