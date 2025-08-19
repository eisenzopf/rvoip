//! Optimized type definitions with Arc-based sharing for performance
//!
//! This module provides memory-optimized versions of frequently-cloned types.
//! The optimizations reduce allocations by sharing string data via Arc.

use std::sync::Arc;
use std::fmt;
use serde::{Serialize, Deserialize, Serializer, Deserializer};
use uuid::Uuid;

/// High-performance SessionId using Arc<str> for sharing
///
/// This version reduces memory allocations by sharing the underlying string
/// data across multiple clones. Especially beneficial when SessionId is
/// cloned frequently (which it is in event processing).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct OptimizedSessionId(Arc<str>);

impl OptimizedSessionId {
    /// Create a new random session ID
    pub fn new() -> Self {
        let id = format!("sess_{}", Uuid::new_v4());
        Self(id.into())
    }
    
    /// Create a session ID from a string (takes ownership)
    pub fn from_string(id: String) -> Self {
        Self(id.into())
    }
    
    /// Create a session ID from a str slice (clones)
    pub fn from_str(id: &str) -> Self {
        Self(id.into())
    }
    
    /// Create from an existing Arc<str>
    pub fn from_arc_str(id: Arc<str>) -> Self {
        Self(id)
    }
    
    /// Get as str reference (zero-cost)
    pub fn as_str(&self) -> &str {
        &self.0
    }
    
    /// Get the inner Arc<str> for efficient sharing
    pub fn as_arc_str(&self) -> &Arc<str> {
        &self.0
    }
    
    /// Extract the inner Arc<str>
    pub fn into_arc_str(self) -> Arc<str> {
        self.0
    }
    
    /// Check if this is the same instance (not just equal content)
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Default for OptimizedSessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for OptimizedSessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for OptimizedSessionId {
    fn from(s: String) -> Self {
        Self::from_string(s)
    }
}

impl From<&str> for OptimizedSessionId {
    fn from(s: &str) -> Self {
        Self::from_str(s)
    }
}

impl From<Arc<str>> for OptimizedSessionId {
    fn from(s: Arc<str>) -> Self {
        Self::from_arc_str(s)
    }
}

// Compatibility conversion from old SessionId
impl From<crate::api::types::SessionId> for OptimizedSessionId {
    fn from(old: crate::api::types::SessionId) -> Self {
        Self::from_str(old.as_str())
    }
}

// Compatibility conversion to old SessionId
impl From<OptimizedSessionId> for crate::api::types::SessionId {
    fn from(optimized: OptimizedSessionId) -> Self {
        crate::api::types::SessionId::from_string(optimized.as_str().to_string())
    }
}

// Custom serialization to maintain compatibility
impl Serialize for OptimizedSessionId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for OptimizedSessionId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(Self::from_string(s))
    }
}

/// Optimized CallState enum with Arc-based error sharing
///
/// Uses Arc<str> for error messages to reduce allocation overhead.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OptimizedCallState {
    /// Call is being initiated
    Initiating,
    /// Call is ringing
    Ringing,
    /// Call is active and connected
    Active,
    /// Call is on hold
    OnHold,
    /// Call is being transferred
    Transferring,
    /// Call is in the process of terminating
    Terminating,
    /// Call has terminated successfully
    Terminated,
    /// Call has failed
    Failed(Arc<str>), // Use Arc<str> even for error messages
    /// Call was cancelled before connection
    Cancelled,
}

impl fmt::Display for OptimizedCallState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Initiating => write!(f, "Initiating"),
            Self::Ringing => write!(f, "Ringing"),
            Self::Active => write!(f, "Active"),
            Self::OnHold => write!(f, "OnHold"),
            Self::Transferring => write!(f, "Transferring"),
            Self::Terminating => write!(f, "Terminating"),
            Self::Terminated => write!(f, "Terminated"),
            Self::Failed(reason) => write!(f, "Failed: {}", reason),
            Self::Cancelled => write!(f, "Cancelled"),
        }
    }
}

// Custom serialization for OptimizedCallState
impl Serialize for OptimizedCallState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStructVariant;
        
        match self {
            Self::Initiating => serializer.serialize_unit_variant("OptimizedCallState", 0, "Initiating"),
            Self::Ringing => serializer.serialize_unit_variant("OptimizedCallState", 1, "Ringing"),
            Self::Active => serializer.serialize_unit_variant("OptimizedCallState", 2, "Active"),
            Self::OnHold => serializer.serialize_unit_variant("OptimizedCallState", 3, "OnHold"),
            Self::Transferring => serializer.serialize_unit_variant("OptimizedCallState", 4, "Transferring"),
            Self::Terminating => serializer.serialize_unit_variant("OptimizedCallState", 5, "Terminating"),
            Self::Terminated => serializer.serialize_unit_variant("OptimizedCallState", 6, "Terminated"),
            Self::Failed(reason) => {
                serializer.serialize_newtype_variant("OptimizedCallState", 7, "Failed", reason.as_ref())
            }
            Self::Cancelled => serializer.serialize_unit_variant("OptimizedCallState", 8, "Cancelled"),
        }
    }
}

impl<'de> Deserialize<'de> for OptimizedCallState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::{self, Visitor, VariantAccess, EnumAccess};
        
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "PascalCase")]
        enum Field {
            Initiating,
            Ringing,
            Active,
            OnHold,
            Transferring,
            Terminating,
            Terminated,
            Failed,
            Cancelled,
        }
        
        struct OptimizedCallStateVisitor;
        
        impl<'de> Visitor<'de> for OptimizedCallStateVisitor {
            type Value = OptimizedCallState;
            
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("enum OptimizedCallState")
            }
            
            fn visit_enum<A>(self, data: A) -> Result<Self::Value, A::Error>
            where
                A: EnumAccess<'de>,
            {
                let (variant, variant_access) = data.variant()?;
                match variant {
                    Field::Initiating => {
                        variant_access.unit_variant()?;
                        Ok(OptimizedCallState::Initiating)
                    }
                    Field::Ringing => {
                        variant_access.unit_variant()?;
                        Ok(OptimizedCallState::Ringing)
                    }
                    Field::Active => {
                        variant_access.unit_variant()?;
                        Ok(OptimizedCallState::Active)
                    }
                    Field::OnHold => {
                        variant_access.unit_variant()?;
                        Ok(OptimizedCallState::OnHold)
                    }
                    Field::Transferring => {
                        variant_access.unit_variant()?;
                        Ok(OptimizedCallState::Transferring)
                    }
                    Field::Terminating => {
                        variant_access.unit_variant()?;
                        Ok(OptimizedCallState::Terminating)
                    }
                    Field::Terminated => {
                        variant_access.unit_variant()?;
                        Ok(OptimizedCallState::Terminated)
                    }
                    Field::Failed => {
                        let reason: String = variant_access.newtype_variant()?;
                        Ok(OptimizedCallState::Failed(reason.into()))
                    }
                    Field::Cancelled => {
                        variant_access.unit_variant()?;
                        Ok(OptimizedCallState::Cancelled)
                    }
                }
            }
        }
        
        deserializer.deserialize_enum("OptimizedCallState", &["Initiating", "Ringing", "Active", "OnHold", "Transferring", "Terminating", "Terminated", "Failed", "Cancelled"], OptimizedCallStateVisitor)
    }
}

// Compatibility conversions for CallState
impl From<crate::api::types::CallState> for OptimizedCallState {
    fn from(old: crate::api::types::CallState) -> Self {
        match old {
            crate::api::types::CallState::Initiating => Self::Initiating,
            crate::api::types::CallState::Ringing => Self::Ringing,
            crate::api::types::CallState::Active => Self::Active,
            crate::api::types::CallState::OnHold => Self::OnHold,
            crate::api::types::CallState::Transferring => Self::Transferring,
            crate::api::types::CallState::Terminating => Self::Terminating,
            crate::api::types::CallState::Terminated => Self::Terminated,
            crate::api::types::CallState::Failed(reason) => Self::Failed(reason.into()),
            crate::api::types::CallState::Cancelled => Self::Cancelled,
        }
    }
}

impl From<OptimizedCallState> for crate::api::types::CallState {
    fn from(optimized: OptimizedCallState) -> Self {
        match optimized {
            OptimizedCallState::Initiating => Self::Initiating,
            OptimizedCallState::Ringing => Self::Ringing,
            OptimizedCallState::Active => Self::Active,
            OptimizedCallState::OnHold => Self::OnHold,
            OptimizedCallState::Transferring => Self::Transferring,
            OptimizedCallState::Terminating => Self::Terminating,
            OptimizedCallState::Terminated => Self::Terminated,
            OptimizedCallState::Failed(reason) => Self::Failed(reason.as_ref().to_string()),
            OptimizedCallState::Cancelled => Self::Cancelled,
        }
    }
}

/// Optimized MediaInfo with Arc-based string sharing
#[derive(Debug, Clone, PartialEq)]
pub struct OptimizedMediaInfo {
    /// Local SDP (shared)
    pub local_sdp: Option<Arc<str>>,
    /// Remote SDP (shared)
    pub remote_sdp: Option<Arc<str>>,
    /// Negotiated codec (shared)
    pub codec: Arc<str>,
    /// Local media address
    pub local_addr: std::net::SocketAddr,
    /// Remote media address
    pub remote_addr: std::net::SocketAddr,
    /// Media quality metrics
    pub quality: OptimizedQualityMetrics,
}

/// Copy-optimized quality metrics
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OptimizedQualityMetrics {
    /// Mean Opinion Score (1.0 - 5.0)
    pub mos: f32,
    /// Jitter in milliseconds
    pub jitter_ms: f32,
    /// Packet loss percentage (0.0 - 100.0)
    pub packet_loss: f32,
    /// Round-trip time in milliseconds
    pub rtt_ms: f32,
}

impl Default for OptimizedQualityMetrics {
    fn default() -> Self {
        Self {
            mos: 4.0, // Good quality
            jitter_ms: 0.0,
            packet_loss: 0.0,
            rtt_ms: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_session_id_arc_sharing() {
        let id1 = OptimizedSessionId::new();
        let id2 = id1.clone();
        
        // Should share the same underlying Arc
        assert!(id1.ptr_eq(&id2));
        assert_eq!(id1, id2);
    }
    
    #[test]
    fn test_session_id_compatibility() {
        // Test conversion from old to new
        let old_id = crate::api::types::SessionId::new();
        let optimized: OptimizedSessionId = old_id.clone().into();
        let back_to_old: crate::api::types::SessionId = optimized.into();
        
        assert_eq!(old_id.as_str(), back_to_old.as_str());
    }
    
    #[test]
    fn test_call_state_clone() {
        let state = OptimizedCallState::Active;
        let state2 = state.clone(); // Clone the state
        
        assert_eq!(state, state2);
        assert_eq!(state, OptimizedCallState::Active);
    }
    
    #[test]
    fn test_serialization() {
        let id = OptimizedSessionId::new();
        let serialized = serde_json::to_string(&id).unwrap();
        let deserialized: OptimizedSessionId = serde_json::from_str(&serialized).unwrap();
        
        assert_eq!(id.as_str(), deserialized.as_str());
    }
}