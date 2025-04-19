use crate::uri::Uri;
use crate::types::Param;

/// Represents a URI with associated parameters.
#[derive(Debug, Clone, PartialEq, Eq)] // Add derives as needed
pub struct UriWithParams {
    pub uri: Uri,
    pub params: Vec<Param>,
}

// TODO: Implement helper methods 