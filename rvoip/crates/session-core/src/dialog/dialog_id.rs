use std::fmt;
use uuid::Uuid;
use serde::{Serialize, Deserialize};

/// Unique identifier for a SIP dialog
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DialogId(pub Uuid);

impl DialogId {
    /// Create a new dialog ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl fmt::Display for DialogId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for DialogId {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_dialog_id_creation() {
        let id1 = DialogId::new();
        let id2 = DialogId::new();
        
        // Two different IDs should not be equal
        assert_ne!(id1, id2);
    }
    
    #[test]
    fn test_dialog_id_display() {
        let id = DialogId::new();
        let id_string = id.to_string();
        
        // String representation should not be empty
        assert!(!id_string.is_empty());
        
        // String should contain the UUID value
        assert_eq!(id_string, id.0.to_string());
    }
    
    #[test]
    fn test_dialog_id_default() {
        let id = DialogId::default();
        
        // Default should create a new UUID
        assert!(!id.to_string().is_empty());
    }
} 