use std::fmt;
use uuid::Uuid;
use serde::{Serialize, Deserialize};

/// Unique identifier for a SIP session
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub Uuid);

impl SessionId {
    /// Create a new session ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_session_id_creation() {
        let id1 = SessionId::new();
        let id2 = SessionId::new();
        
        // Verify each instance is unique
        assert_ne!(id1, id2);
        
        // Test display implementation
        let id_str = id1.to_string();
        assert!(!id_str.is_empty());
        
        // Test default implementation
        let default_id = SessionId::default();
        assert_ne!(default_id, id1);
    }
} 