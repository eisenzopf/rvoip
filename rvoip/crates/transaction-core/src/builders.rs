//! Builder utilities and helper functions
//!
//! This module provides convenience functions for creating SIP requests and responses,
//! as well as helper functions for extracting dialog information from SIP messages.

use rvoip_sip_core::{Request, Response};
use crate::error::Result;

/// Client-side request builders for common SIP operations
pub use crate::client::builders::{
    InviteBuilder, ByeBuilder, RegisterBuilder,
    quick as client_quick
};

/// Server-side response builders for common SIP operations
pub use crate::server::builders::{
    ResponseBuilder, InviteResponseBuilder, RegisterResponseBuilder,
    quick as server_quick
};

/// Dialog information extracted from SIP messages
#[derive(Debug, Clone)]
pub struct DialogInfo {
    pub call_id: String,
    pub from_uri: String,
    pub from_tag: String,
    pub to_uri: String,
    pub to_tag: Option<String>,
    pub cseq: u32,
}

/// Helper functions for extracting dialog information
pub mod dialog_utils {
    use super::*;
    
    /// Extract dialog information from a SIP request
    pub fn extract_dialog_info(request: &Request) -> Option<DialogInfo> {
        let call_id = request.call_id()?.value().to_string();
        
        let from = request.from()?;
        let from_uri = from.address().uri.to_string();
        let from_tag = from.tag()?.to_string();
        
        let to = request.to()?;
        let to_uri = to.address().uri.to_string();
        let to_tag = to.tag().map(|t| t.to_string());
        
        let cseq = request.cseq()?.seq;
        
        Some(DialogInfo {
            call_id,
            from_uri,
            from_tag,
            to_uri,
            to_tag,
            cseq,
        })
    }
    
    /// Extract dialog information from a SIP response
    pub fn extract_dialog_info_from_response(response: &Response) -> Option<DialogInfo> {
        let call_id = response.call_id()?.value().to_string();
        
        let from = response.from()?;
        let from_uri = from.address().uri.to_string();
        let from_tag = from.tag()?.to_string();
        
        let to = response.to()?;
        let to_uri = to.address().uri.to_string();
        let to_tag = to.tag().map(|t| t.to_string());
        
        let cseq = response.cseq()?.seq;
        
        Some(DialogInfo {
            call_id,
            from_uri,
            from_tag,
            to_uri,
            to_tag,
            cseq,
        })
    }
} 