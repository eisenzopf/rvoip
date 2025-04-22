use std::net::SocketAddr;
use std::time::Instant;

use rvoip_sip_core::Uri;

/// Registration state
pub struct Registration {
    /// Server address
    pub server: SocketAddr,
    
    /// Registration URI
    pub uri: Uri,
    
    /// Is registered
    pub registered: bool,
    
    /// Registration expiry
    pub expires: u32,
    
    /// Last registration time
    pub registered_at: Option<Instant>,
    
    /// Error message if registration failed
    pub error: Option<String>,
    
    /// Task handle for registration refresh
    pub refresh_task: Option<tokio::task::JoinHandle<()>>,
}

impl Clone for Registration {
    fn clone(&self) -> Self {
        Self {
            server: self.server,
            uri: self.uri.clone(),
            registered: self.registered,
            expires: self.expires,
            registered_at: self.registered_at,
            error: self.error.clone(),
            refresh_task: None, // Don't clone the task handle
        }
    }
} 