//! ⚠️ **DEPRECATED: CLIENT MANAGER MOVED TO CLIENT-CORE**
//!
//! This module contained client-specific business logic that has been moved to client-core.

#[deprecated(
    since = "1.0.0",
    note = "Client manager moved to client-core. Use rvoip_client_core::ClientManager instead."
)]
pub struct ClientManager {
    _placeholder: (),
}

impl ClientManager {
    #[deprecated(since = "1.0.0", note = "Use client-core instead")]
    pub fn new() -> Self {
        Self { _placeholder: () }
    }
} 