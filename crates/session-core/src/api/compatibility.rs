//! Compatibility utilities for migrating to optimized types
//!
//! This module provides utilities to help migrate existing code from the original
//! types to the optimized Arc-based types gradually, without breaking changes.

use crate::api::types::{SessionId, CallState, MediaInfo};
use crate::api::optimized_types::{OptimizedSessionId, OptimizedCallState, OptimizedMediaInfo, OptimizedQualityMetrics};

/// Trait for converting from original types to optimized types
pub trait ToOptimized<T> {
    fn to_optimized(self) -> T;
}

/// Trait for converting from optimized types to original types
pub trait FromOptimized<T> {
    fn from_optimized(optimized: T) -> Self;
}

// SessionId conversions
impl ToOptimized<OptimizedSessionId> for SessionId {
    fn to_optimized(self) -> OptimizedSessionId {
        OptimizedSessionId::from(self)
    }
}

impl FromOptimized<OptimizedSessionId> for SessionId {
    fn from_optimized(optimized: OptimizedSessionId) -> Self {
        optimized.into()
    }
}

// CallState conversions
impl ToOptimized<OptimizedCallState> for CallState {
    fn to_optimized(self) -> OptimizedCallState {
        OptimizedCallState::from(self)
    }
}

impl FromOptimized<OptimizedCallState> for CallState {
    fn from_optimized(optimized: OptimizedCallState) -> Self {
        optimized.into()
    }
}

/// Session adapter that can work with either SessionId type
#[derive(Debug, Clone)]
pub enum SessionIdAdapter {
    Original(SessionId),
    Optimized(OptimizedSessionId),
}

impl SessionIdAdapter {
    /// Create from original SessionId
    pub fn from_original(id: SessionId) -> Self {
        Self::Original(id)
    }
    
    /// Create from optimized SessionId
    pub fn from_optimized(id: OptimizedSessionId) -> Self {
        Self::Optimized(id)
    }
    
    /// Get as str reference (works for both types)
    pub fn as_str(&self) -> &str {
        match self {
            Self::Original(id) => id.as_str(),
            Self::Optimized(id) => id.as_str(),
        }
    }
    
    /// Convert to optimized type (zero-cost if already optimized)
    pub fn into_optimized(self) -> OptimizedSessionId {
        match self {
            Self::Original(id) => id.to_optimized(),
            Self::Optimized(id) => id,
        }
    }
    
    /// Convert to original type (clones if optimized)
    pub fn into_original(self) -> SessionId {
        match self {
            Self::Original(id) => id,
            Self::Optimized(id) => SessionId::from_optimized(id),
        }
    }
}

impl From<SessionId> for SessionIdAdapter {
    fn from(id: SessionId) -> Self {
        Self::Original(id)
    }
}

impl From<OptimizedSessionId> for SessionIdAdapter {
    fn from(id: OptimizedSessionId) -> Self {
        Self::Optimized(id)
    }
}

impl std::fmt::Display for SessionIdAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Helper functions for common migration patterns
pub mod helpers {
    use super::*;
    
    /// Create an optimized SessionId from any string-like input
    pub fn create_optimized_session_id(input: impl AsRef<str>) -> OptimizedSessionId {
        OptimizedSessionId::from_str(input.as_ref())
    }
    
    /// Convert a collection of original SessionIds to optimized ones
    pub fn convert_session_ids<I>(ids: I) -> Vec<OptimizedSessionId>
    where
        I: IntoIterator<Item = SessionId>,
    {
        ids.into_iter().map(|id| id.to_optimized()).collect()
    }
    
    /// Batch convert optimized SessionIds back to original format
    pub fn convert_back_session_ids<I>(ids: I) -> Vec<SessionId>
    where
        I: IntoIterator<Item = OptimizedSessionId>,
    {
        ids.into_iter().map(|id| SessionId::from_optimized(id)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_session_id_conversions() {
        let original = SessionId::new();
        let optimized = original.clone().to_optimized();
        let back_to_original = SessionId::from_optimized(optimized.clone());
        
        assert_eq!(original.as_str(), back_to_original.as_str());
    }
    
    #[test]
    fn test_session_id_adapter() {
        let original = SessionId::new();
        let adapter = SessionIdAdapter::from_original(original.clone());
        
        assert_eq!(adapter.as_str(), original.as_str());
        
        let optimized = adapter.into_optimized();
        assert_eq!(optimized.as_str(), original.as_str());
    }
    
    #[test]
    fn test_helpers() {
        let original_ids = vec![SessionId::new(), SessionId::new()];
        let optimized_ids = helpers::convert_session_ids(original_ids.clone());
        let back_to_original = helpers::convert_back_session_ids(optimized_ids);
        
        assert_eq!(original_ids.len(), back_to_original.len());
        for (orig, converted) in original_ids.iter().zip(back_to_original.iter()) {
            assert_eq!(orig.as_str(), converted.as_str());
        }
    }
}