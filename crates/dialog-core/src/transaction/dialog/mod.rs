//! Dialog Utility Module
//!
//! This module provides bridge functions between dialog-core templates and 
//! transaction-core builders, enabling seamless integration between the 
//! dialog and transaction layers.
//!
//! # Features
//!
//! - Convert dialog templates to request builders
//! - Create response builders with dialog transaction context
//! - Helper functions for common dialog operations
//! - Maintain clean separation of concerns

pub mod quick; // Phase 3.2: Quick dialog functions

use std::net::SocketAddr;
use rvoip_sip_core::{Request, Response, Method, StatusCode, Uri};
use rvoip_sip_core::types::header::{HeaderName, TypedHeader};
use crate::transaction::client::builders::{InviteBuilder, ByeBuilder, InDialogRequestBuilder};
use crate::transaction::server::builders::{ResponseBuilder, InviteResponseBuilder};
use crate::transaction::error::{Error, Result};

// Re-export quick functions for convenience
pub use quick::{
    bye_for_dialog, refer_for_dialog, update_for_dialog, info_for_dialog,
    notify_for_dialog, message_for_dialog, reinvite_for_dialog,
    response_for_dialog_transaction
};

/// Dialog request template containing extracted dialog context
/// 
/// This structure represents the dialog information needed to create
/// requests within an established dialog. It bridges dialog-core's
/// template system with transaction-core's builders.
#[derive(Debug, Clone)]
pub struct DialogRequestTemplate {
    pub call_id: String,
    pub from_uri: String,
    pub from_tag: String,
    pub to_uri: String,
    pub to_tag: String,
    pub request_uri: String,
    pub cseq: u32,
    pub local_address: SocketAddr,
    pub route_set: Vec<Uri>,
    pub contact: Option<String>,
}

/// Dialog transaction context for response building
/// 
/// Contains information about the transaction and dialog state needed
/// to create appropriate responses with proper dialog handling.
#[derive(Debug, Clone)]
pub struct DialogTransactionContext {
    pub dialog_id: Option<String>,
    pub transaction_id: String,
    pub original_request: Request,
    pub is_dialog_creating: bool,
    pub local_address: SocketAddr,
}

/// Create a request builder from a dialog template
/// 
/// This function bridges dialog-core's template system with transaction-core's
/// fluent builders, providing seamless integration between the layers.
/// 
/// # Arguments
/// * `template` - Dialog request template with extracted context
/// * `method` - SIP method for the request
/// * `body` - Optional request body
/// * `content_type` - Optional content type for the body
/// 
/// # Returns
/// An appropriate request builder configured with dialog context
/// 
/// # Example
/// ```rust,no_run
/// use rvoip_dialog_core::transaction::dialog::{DialogRequestTemplate, request_builder_from_dialog_template};
/// use rvoip_sip_core::Method;
/// use std::net::SocketAddr;
/// 
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
/// let template = DialogRequestTemplate {
///     call_id: "call-123".to_string(),
///     from_uri: "sip:alice@example.com".to_string(),
///     from_tag: "alice-tag".to_string(),
///     to_uri: "sip:bob@example.com".to_string(),
///     to_tag: "bob-tag".to_string(),
///     request_uri: "sip:bob@example.com".to_string(),
///     cseq: 2,
///     local_address: local_addr,
///     route_set: vec![],
///     contact: None,
/// };
/// 
/// let request = request_builder_from_dialog_template(&template, Method::Bye, None, None)?;
/// # Ok(())
/// # }
/// ```
pub fn request_builder_from_dialog_template(
    template: &DialogRequestTemplate,
    method: Method,
    body: Option<String>,
    content_type: Option<String>
) -> Result<Request> {
    match method {
        Method::Invite => {
            let mut builder = InviteBuilder::from_dialog_enhanced(
                &template.call_id,
                &template.from_uri,
                &template.from_tag,
                None, // Display name - use default
                &template.to_uri,
                &template.to_tag,
                None, // Display name - use default
                &template.request_uri,
                template.cseq,
                template.local_address,
                template.route_set.clone(),
                template.contact.clone()
            );
            
            if let Some(sdp_content) = body {
                builder = builder.with_sdp(sdp_content);
            }
            
            // Add Contact header if specified in template
            if let Some(contact_uri) = &template.contact {
                use rvoip_sip_core::types::contact::{Contact, ContactParamInfo};
                use rvoip_sip_core::types::address::Address;
                use rvoip_sip_core::types::uri::Uri;
                
                // Parse the contact URI and create a Contact header
                if let Ok(contact_uri_parsed) = contact_uri.parse::<Uri>() {
                    let contact_addr = Address::new(contact_uri_parsed);
                    let contact_param = ContactParamInfo { address: contact_addr };
                    let contact_header = Contact::new_params(vec![contact_param]);
                    builder = builder.header(TypedHeader::Contact(contact_header));
                }
            }
            
            builder.build()
        },
        
        Method::Bye => {
            ByeBuilder::from_dialog_enhanced(
                &template.call_id,
                &template.from_uri,
                &template.from_tag,
                &template.to_uri,
                &template.to_tag,
                &template.request_uri,
                template.cseq,
                template.local_address,
                template.route_set.clone()
            ).build()
        },
        
        _ => {
            // Use InDialogRequestBuilder for other methods
            let mut builder = InDialogRequestBuilder::new(method)
                .from_dialog_enhanced(
                    &template.call_id,
                    &template.from_uri,
                    &template.from_tag,
                    &template.to_uri,
                    &template.to_tag,
                    &template.request_uri,
                    template.cseq,
                    template.local_address,
                    template.route_set.clone()
                );
            
            // Add Contact header if specified in template
            if let Some(contact_uri) = &template.contact {
                use rvoip_sip_core::types::contact::{Contact, ContactParamInfo};
                use rvoip_sip_core::types::address::Address;
                use rvoip_sip_core::types::uri::Uri;
                
                // Parse the contact URI and create a Contact header
                if let Ok(contact_uri_parsed) = contact_uri.parse::<Uri>() {
                    let contact_addr = Address::new(contact_uri_parsed);
                    let contact_param = ContactParamInfo { address: contact_addr };
                    let contact_header = Contact::new_params(vec![contact_param]);
                    builder = builder.header(TypedHeader::Contact(contact_header));
                }
            }
            
            if let Some(request_body) = body {
                builder = builder.with_body(request_body);
            }
            
            if let Some(ct) = content_type {
                builder = builder.with_content_type(ct);
            }
            
            builder.build()
        }
    }
}

/// Create a response builder with dialog transaction context
/// 
/// This function creates an appropriate response builder that understands
/// dialog context and can handle dialog-creating responses properly.
/// 
/// # Arguments
/// * `context` - Dialog transaction context
/// * `status_code` - SIP status code for the response
/// * `contact_address` - Optional contact address for the response
/// * `sdp_content` - Optional SDP content for the response
/// 
/// # Returns
/// A configured response builder ready for customization and building
/// 
/// # Example
/// ```rust,no_run
/// use rvoip_dialog_core::transaction::dialog::{DialogTransactionContext, response_builder_for_dialog_transaction};
/// use rvoip_dialog_core::transaction::client::builders::InviteBuilder;
/// use rvoip_sip_core::StatusCode;
/// use std::net::SocketAddr;
/// 
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
/// let original_request = InviteBuilder::new()
///     .from_to("sip:alice@example.com", "sip:bob@example.com")
///     .local_address(local_addr)
///     .build()?;
/// 
/// let context = DialogTransactionContext {
///     dialog_id: Some("dialog-123".to_string()),
///     transaction_id: "txn-456".to_string(),
///     original_request,
///     is_dialog_creating: true,
///     local_address: local_addr,
/// };
/// 
/// let response = response_builder_for_dialog_transaction(
///     &context,
///     StatusCode::Ok,
///     Some(local_addr),
///     Some("v=0\r\no=server 456 789 IN IP4 127.0.0.1\r\n".to_string())
/// )?;
/// # Ok(())
/// # }
/// ```
pub fn response_builder_for_dialog_transaction(
    context: &DialogTransactionContext,
    status_code: StatusCode,
    contact_address: Option<SocketAddr>,
    sdp_content: Option<String>
) -> Result<Response> {
    // Determine if this is an INVITE transaction
    let is_invite = context.original_request.method() == Method::Invite;
    
    if is_invite {
        // Use InviteResponseBuilder for INVITE responses
        let mut builder = InviteResponseBuilder::from_dialog_context(
            status_code,
            &context.original_request,
            context.dialog_id.as_deref()
        );
        
        // Add contact if provided
        if let Some(contact_addr) = contact_address {
            builder = builder.with_contact_address(contact_addr, Some("server"));
        }
        
        // Add SDP content if provided
        if let Some(sdp) = sdp_content {
            builder = builder.with_sdp_answer(sdp);
        }
        
        builder.build()
    } else {
        // Use general ResponseBuilder for other methods
        let mut builder = ResponseBuilder::from_dialog_transaction(
            status_code,
            &context.original_request,
            context.dialog_id.as_deref()
        );
        
        // Add contact if provided
        if let Some(contact_addr) = contact_address {
            builder = builder.with_contact_address(contact_addr, Some("server"));
        }
        
        // Add SDP content if provided (though rare for non-INVITE)
        if let Some(sdp) = sdp_content {
            builder = builder.with_sdp(sdp);
        }
        
        builder.build()
    }
}

/// Extract dialog template from a SIP request
/// 
/// This function analyzes a SIP request and extracts dialog context
/// that can be used to create subsequent requests in the same dialog.
/// 
/// # Arguments
/// * `request` - The SIP request to extract dialog context from
/// * `local_address` - Local address for the dialog
/// * `next_cseq` - Next CSeq number for the dialog
/// 
/// # Returns
/// Dialog request template with extracted context
/// 
/// # Example
/// ```rust,no_run
/// use rvoip_dialog_core::transaction::dialog::extract_dialog_template_from_request;
/// use rvoip_dialog_core::transaction::client::builders::InviteBuilder;
/// use std::net::SocketAddr;
/// 
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
/// let invite_request = InviteBuilder::new()
///     .from_to("sip:alice@example.com", "sip:bob@example.com")
///     .local_address(local_addr)
///     .build()?;
/// 
/// let template = extract_dialog_template_from_request(&invite_request, local_addr, 2)?;
/// # Ok(())
/// # }
/// ```
pub fn extract_dialog_template_from_request(
    request: &Request,
    local_address: SocketAddr,
    next_cseq: u32
) -> Result<DialogRequestTemplate> {
    // Extract required headers
    let call_id = request.call_id()
        .ok_or_else(|| Error::Other("Missing Call-ID header".to_string()))?
        .value().to_string();
    
    let from = request.from()
        .ok_or_else(|| Error::Other("Missing From header".to_string()))?;
    let from_uri = from.address().uri.to_string();
    let from_tag = from.tag()
        .ok_or_else(|| Error::Other("Missing From tag".to_string()))?
        .to_string();
    
    let to = request.to()
        .ok_or_else(|| Error::Other("Missing To header".to_string()))?;
    let to_uri = to.address().uri.to_string();
    let to_tag = to.tag()
        .ok_or_else(|| Error::Other("Missing To tag for in-dialog request".to_string()))?
        .to_string();
    
    // Extract route set from Route headers
    let route_set: Vec<Uri> = request.headers.iter()
        .filter_map(|header| {
            if let TypedHeader::Route(route) = header {
                // Route contains Vec<ParserRouteValue>, so iterate over all route entries
                Some(route.iter().map(|route_entry| route_entry.0.uri.clone()).collect::<Vec<_>>())
            } else {
                None
            }
        })
        .flatten()
        .collect();
    
    // Extract contact from Contact header if present
    let contact = request.header(&HeaderName::Contact)
        .and_then(|header| {
            if let TypedHeader::Contact(contact_header) = header {
                let addresses: Vec<_> = contact_header.addresses().collect();
                addresses.first()
                    .map(|addr| addr.uri.to_string())
            } else {
                None
            }
        });
    
    Ok(DialogRequestTemplate {
        call_id,
        from_uri,
        from_tag,
        to_uri: to_uri.clone(),
        to_tag,
        request_uri: to_uri, // Default to To URI
        cseq: next_cseq,
        local_address,
        route_set,
        contact,
    })
}

/// Create dialog transaction context from request and dialog information
/// 
/// Helper function to create dialog transaction context for response building.
/// 
/// # Arguments
/// * `transaction_id` - Transaction identifier
/// * `original_request` - The original request to respond to
/// * `dialog_id` - Optional dialog identifier
/// * `local_address` - Local address for the dialog
/// 
/// # Returns
/// Dialog transaction context ready for response building
pub fn create_dialog_transaction_context(
    transaction_id: impl Into<String>,
    original_request: Request,
    dialog_id: Option<String>,
    local_address: SocketAddr
) -> DialogTransactionContext {
    let is_dialog_creating = match original_request.method() {
        Method::Invite => {
            // INVITE is dialog-creating if To header has no tag
            original_request.to()
                .map(|to| to.tag().is_none())
                .unwrap_or(false)
        },
        Method::Subscribe => {
            // SUBSCRIBE is dialog-creating if To header has no tag (RFC 6665)
            original_request.to()
                .map(|to| to.tag().is_none())
                .unwrap_or(false)
        },
        _ => false, // Other methods are typically not dialog-creating
    };
    
    DialogTransactionContext {
        dialog_id,
        transaction_id: transaction_id.into(),
        original_request,
        is_dialog_creating,
        local_address,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    
    #[tokio::test]
    async fn test_dialog_template_creation() {
        let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        
        let template = DialogRequestTemplate {
            call_id: "test-call-123".to_string(),
            from_uri: "sip:alice@example.com".to_string(),
            from_tag: "alice-tag".to_string(),
            to_uri: "sip:bob@example.com".to_string(),
            to_tag: "bob-tag".to_string(),
            request_uri: "sip:bob@example.com".to_string(),
            cseq: 2,
            local_address: local_addr,
            route_set: vec![],
            contact: None,
        };
        
        // Test BYE creation from template
        let bye_request = request_builder_from_dialog_template(
            &template,
            Method::Bye,
            None,
            None
        ).expect("Failed to create BYE from template");
        
        assert_eq!(bye_request.method(), Method::Bye);
        assert_eq!(bye_request.call_id().unwrap().value(), template.call_id);
        assert_eq!(bye_request.from().unwrap().tag().unwrap(), template.from_tag);
        assert_eq!(bye_request.to().unwrap().tag().unwrap(), template.to_tag);
        assert_eq!(bye_request.cseq().unwrap().seq, template.cseq);
    }
    
    #[tokio::test]
    async fn test_dialog_transaction_context() {
        use crate::transaction::client::builders::InviteBuilder;
        
        let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        
        let original_request = InviteBuilder::new()
            .from_to("sip:alice@example.com", "sip:bob@example.com")
            .local_address(local_addr)
            .build()
            .expect("Failed to create INVITE");
        
        let context = create_dialog_transaction_context(
            "txn-123",
            original_request.clone(),
            Some("dialog-456".to_string()),
            local_addr
        );
        
        assert_eq!(context.transaction_id, "txn-123");
        assert_eq!(context.dialog_id, Some("dialog-456".to_string()));
        assert!(context.is_dialog_creating); // INVITE with no To tag
        assert_eq!(context.local_address, local_addr);
        
        // Test response building from context
        let response = response_builder_for_dialog_transaction(
            &context,
            StatusCode::Ok,
            Some(local_addr),
            Some("v=0\r\no=server 456 789 IN IP4 127.0.0.1\r\n".to_string())
        ).expect("Failed to build response from context");
        
        assert_eq!(response.status_code(), 200);
        assert!(response.to().unwrap().tag().is_some()); // Auto-generated To tag
        assert!(response.body().len() > 0); // Has SDP content
    }
}

/// Helper functions for common dialog operations
pub mod helpers {
    use super::*;
    
    /// Quick BYE request creation from dialog template
    pub fn quick_bye_from_template(template: &DialogRequestTemplate) -> Result<Request> {
        request_builder_from_dialog_template(template, Method::Bye, None, None)
    }
    
    /// Quick REFER request creation from dialog template
    pub fn quick_refer_from_template(
        template: &DialogRequestTemplate,
        target_uri: &str
    ) -> Result<Request> {
        request_builder_from_dialog_template(
            template,
            Method::Refer,
            Some(format!("Refer-To: {}\r\n", target_uri)),
            Some("message/sipfrag".to_string())
        )
    }
    
    /// Quick UPDATE request creation from dialog template
    pub fn quick_update_from_template(
        template: &DialogRequestTemplate,
        sdp: Option<String>
    ) -> Result<Request> {
        let content_type = if sdp.is_some() {
            Some("application/sdp".to_string())
        } else {
            None
        };
        
        request_builder_from_dialog_template(template, Method::Update, sdp, content_type)
    }
    
    /// Quick INFO request creation from dialog template
    pub fn quick_info_from_template(
        template: &DialogRequestTemplate,
        content: &str
    ) -> Result<Request> {
        request_builder_from_dialog_template(
            template,
            Method::Info,
            Some(content.to_string()),
            Some("application/info".to_string())
        )
    }
    
    /// Quick NOTIFY request creation from dialog template
    pub fn quick_notify_from_template(
        template: &DialogRequestTemplate,
        event: &str,
        body: Option<String>
    ) -> Result<Request> {
        let mut builder = InDialogRequestBuilder::for_notify(event, body)
            .from_dialog_enhanced(
                &template.call_id,
                &template.from_uri,
                &template.from_tag,
                &template.to_uri,
                &template.to_tag,
                &template.request_uri,
                template.cseq,
                template.local_address,
                template.route_set.clone()
            );
        
        builder.build()
    }
} 