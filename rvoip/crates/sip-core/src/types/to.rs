use crate::types::address::Address;

/// Typed To header.
#[derive(Debug, Clone, PartialEq, Eq)] // Add derives as needed
pub struct To(pub Address);
 
// TODO: Implement specific To logic/helpers (e.g., getting tag) 