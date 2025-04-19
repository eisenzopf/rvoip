use crate::types::address::Address;

/// Typed From header.
#[derive(Debug, Clone, PartialEq, Eq)] // Add derives as needed
pub struct From(pub Address);
 
// TODO: Implement specific From logic/helpers (e.g., getting tag) 