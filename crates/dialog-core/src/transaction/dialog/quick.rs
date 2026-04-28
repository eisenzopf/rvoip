//! Quick Dialog Functions
//!
//! This module provides convenient one-liner functions for common dialog operations,
//! building on top of the dialog utility functions to make dialog operations as
//! simple as possible.
//!
//! # Features
//!
//! - One-liner functions for common dialog operations
//! - Automatic builder selection based on SIP method
//! - Simplified parameter handling
//! - Integration with dialog-core templates

use super::{
    request_builder_from_dialog_template, response_builder_for_dialog_transaction,
    DialogRequestTemplate, DialogTransactionContext,
};
use crate::transaction::error::{Error, Result};
use rvoip_sip_core::types::rack::RAck;
use rvoip_sip_core::types::TypedHeader;
use rvoip_sip_core::{Method, Request, Response, StatusCode, Uri};
use std::net::SocketAddr;

fn via_transport_for_uri(uri: &str) -> &'static str {
    use crate::transaction::transport::multiplexed::select_transport_for_uri;
    use rvoip_sip_transport::transport::TransportType;
    use std::str::FromStr;

    if let Ok(parsed) = Uri::from_str(uri) {
        return match select_transport_for_uri(&parsed) {
            TransportType::Tls => "TLS",
            TransportType::Tcp => "TCP",
            TransportType::Wss => "WSS",
            TransportType::Ws => "WS",
            TransportType::Udp => "UDP",
        };
    }

    let lower = uri.to_ascii_lowercase();
    if lower.starts_with("sips:") || lower.contains(";transport=tls") {
        "TLS"
    } else if lower.contains(";transport=tcp") {
        "TCP"
    } else if lower.contains(";transport=wss") {
        "WSS"
    } else if lower.contains(";transport=ws") {
        "WS"
    } else {
        "UDP"
    }
}

fn via_transport_for_uris(request_uri: &str, local_uri: &str) -> &'static str {
    let request_transport = via_transport_for_uri(request_uri);
    if request_transport != "UDP" {
        request_transport
    } else {
        via_transport_for_uri(local_uri)
    }
}

fn via_transport_for_route_or_uris(
    route_set: Option<&[Uri]>,
    request_uri: &str,
    local_uri: &str,
) -> &'static str {
    route_set
        .and_then(|routes| routes.first())
        .map(|route| via_transport_for_uri(&route.to_string()))
        .filter(|transport| *transport != "UDP")
        .unwrap_or_else(|| via_transport_for_uris(request_uri, local_uri))
}

fn default_target_refresh_contact(
    from_uri: &str,
    request_uri: &str,
    route_set: Option<&[Uri]>,
    local_address: SocketAddr,
) -> String {
    let user_part = if let Ok(parsed) = from_uri.parse::<Uri>() {
        parsed
            .user
            .as_ref()
            .map(|u| u.as_str().to_string())
            .unwrap_or_else(|| "user".to_string())
    } else {
        "user".to_string()
    };

    let transport = via_transport_for_route_or_uris(route_set, request_uri, from_uri);
    if transport.eq_ignore_ascii_case("TLS") || from_uri.starts_with("sips:") {
        format!("sips:{}@{};transport=tls", user_part, local_address)
    } else {
        format!("sip:{}@{}", user_part, local_address)
    }
}

/// Quick BYE request creation for dialog termination
///
/// Creates a BYE request from dialog context in a single function call.
///
/// # Arguments
/// * `call_id` - Dialog Call-ID
/// * `from_uri` - Local URI (From header)
/// * `from_tag` - Local tag (From header tag)
/// * `to_uri` - Remote URI (To header)
/// * `to_tag` - Remote tag (To header tag)
/// * `cseq` - Next CSeq number for the dialog
/// * `local_address` - Local address for Via header
/// * `route_set` - Optional route set for proxy routing
///
/// # Returns
/// Ready-to-send BYE request
///
/// # Example
/// ```rust,no_run
/// use rvoip_dialog_core::transaction::dialog::quick::bye_for_dialog;
/// use std::net::SocketAddr;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
/// let bye_request = bye_for_dialog(
///     "call-123",
///     "sip:alice@example.com",
///     "alice-tag",
///     "sip:bob@example.com",
///     "bob-tag",
///     3,
///     local_addr,
///     None,
///     None,
/// )?;
/// # Ok(())
/// # }
/// ```
pub fn bye_for_dialog(
    call_id: impl Into<String>,
    from_uri: impl Into<String>,
    from_tag: impl Into<String>,
    to_uri: impl Into<String>,
    to_tag: impl Into<String>,
    cseq: u32,
    local_address: SocketAddr,
    route_set: Option<Vec<Uri>>,
    extra_headers: Option<Vec<TypedHeader>>,
) -> Result<Request> {
    let to_uri_string = to_uri.into();
    let template = DialogRequestTemplate {
        call_id: call_id.into(),
        from_uri: from_uri.into(),
        from_tag: from_tag.into(),
        to_uri: to_uri_string.clone(),
        to_tag: to_tag.into(),
        request_uri: to_uri_string,
        cseq,
        local_address,
        route_set: route_set.unwrap_or_default(),
        contact: None,
    };

    request_builder_from_dialog_template(&template, Method::Bye, None, None, extra_headers)
}

/// Quick REFER request creation for call transfer
///
/// Creates a REFER request to transfer a call to a new target.
///
/// # Arguments
/// * `call_id` - Dialog Call-ID
/// * `from_uri` - Local URI (From header)
/// * `from_tag` - Local tag (From header tag)
/// * `to_uri` - Remote URI (To header)
/// * `to_tag` - Remote tag (To header tag)
/// * `target_uri` - URI to transfer the call to
/// * `cseq` - Next CSeq number for the dialog
/// * `local_address` - Local address for Via header
/// * `route_set` - Optional route set for proxy routing
///
/// # Returns
/// Ready-to-send REFER request
///
/// # Example
/// ```rust,no_run
/// use rvoip_dialog_core::transaction::dialog::quick::refer_for_dialog;
/// use std::net::SocketAddr;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
/// let refer_request = refer_for_dialog(
///     "call-123",
///     "sip:alice@example.com",
///     "alice-tag",
///     "sip:bob@example.com",
///     "bob-tag",
///     "sip:charlie@example.com",
///     2,
///     local_addr,
///     None
/// )?;
/// # Ok(())
/// # }
/// ```
pub fn refer_for_dialog(
    call_id: impl Into<String>,
    from_uri: impl Into<String>,
    from_tag: impl Into<String>,
    to_uri: impl Into<String>,
    to_tag: impl Into<String>,
    target_uri: impl Into<String>,
    cseq: u32,
    local_address: SocketAddr,
    route_set: Option<Vec<Uri>>,
) -> Result<Request> {
    let to_uri_string = to_uri.into();
    let target_uri_str = target_uri.into();

    let template = DialogRequestTemplate {
        call_id: call_id.into(),
        from_uri: from_uri.into(),
        from_tag: from_tag.into(),
        to_uri: to_uri_string.clone(),
        to_tag: to_tag.into(),
        request_uri: to_uri_string,
        cseq,
        local_address,
        route_set: route_set.unwrap_or_default(),
        contact: None,
    };

    // Build the REFER request without a body - we'll add the Refer-To header separately
    let mut request = request_builder_from_dialog_template(
        &template,
        Method::Refer,
        None, // No body - Refer-To is a header, not body content
        None, // No content type needed
        None,
    )?;

    // Add the Refer-To header using the proper SIP type
    use rvoip_sip_core::types::address::Address;
    use rvoip_sip_core::types::refer_to::ReferTo;
    use rvoip_sip_core::types::uri::Uri;
    use rvoip_sip_core::types::TypedHeader;
    use std::str::FromStr;

    // Parse the target URI and create a ReferTo header
    if let Ok(parsed_uri) = Uri::from_str(&target_uri_str) {
        let address = Address::new(parsed_uri);
        let refer_to = ReferTo::new(address);

        // Add the ReferTo header to the request
        request.headers.push(TypedHeader::ReferTo(refer_to));
    } else {
        return Err(Error::Other(format!(
            "Invalid Refer-To URI: {}",
            target_uri_str
        )));
    }

    Ok(request)
}

/// Quick UPDATE request creation for session modification
///
/// Creates an UPDATE request to modify session parameters.
///
/// # Arguments
/// * `call_id` - Dialog Call-ID
/// * `from_uri` - Local URI (From header)
/// * `from_tag` - Local tag (From header tag)
/// * `to_uri` - Remote URI (To header)
/// * `to_tag` - Remote tag (To header tag)
/// * `sdp_content` - Optional SDP for media updates
/// * `cseq` - Next CSeq number for the dialog
/// * `local_address` - Local address for Via header
/// * `route_set` - Optional route set for proxy routing
///
/// # Returns
/// Ready-to-send UPDATE request
///
/// # Example
/// ```rust,no_run
/// use rvoip_dialog_core::transaction::dialog::quick::update_for_dialog;
/// use std::net::SocketAddr;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
/// let update_request = update_for_dialog(
///     "call-123",
///     "sip:alice@example.com",
///     "alice-tag",
///     "sip:bob@example.com",
///     "bob-tag",
///     Some("v=0\r\no=alice 456 789 IN IP4 127.0.0.1\r\n...".to_string()),
///     2,
///     local_addr,
///     None
/// )?;
/// # Ok(())
/// # }
/// ```
pub fn update_for_dialog(
    call_id: impl Into<String>,
    from_uri: impl Into<String>,
    from_tag: impl Into<String>,
    to_uri: impl Into<String>,
    to_tag: impl Into<String>,
    sdp_content: Option<String>,
    cseq: u32,
    local_address: SocketAddr,
    route_set: Option<Vec<Uri>>,
) -> Result<Request> {
    update_for_dialog_with_contact(
        call_id,
        from_uri,
        from_tag,
        to_uri,
        to_tag,
        sdp_content,
        cseq,
        local_address,
        route_set,
        None,
    )
}

/// Quick UPDATE request creation with an explicit Contact override.
pub fn update_for_dialog_with_contact(
    call_id: impl Into<String>,
    from_uri: impl Into<String>,
    from_tag: impl Into<String>,
    to_uri: impl Into<String>,
    to_tag: impl Into<String>,
    sdp_content: Option<String>,
    cseq: u32,
    local_address: SocketAddr,
    route_set: Option<Vec<Uri>>,
    contact_uri: Option<String>,
) -> Result<Request> {
    let to_uri_string = to_uri.into();
    let from_uri_string = from_uri.into();
    let route_set = route_set.unwrap_or_default();
    let contact_uri = contact_uri.unwrap_or_else(|| {
        default_target_refresh_contact(
            &from_uri_string,
            &to_uri_string,
            Some(&route_set),
            local_address,
        )
    });

    let template = DialogRequestTemplate {
        call_id: call_id.into(),
        from_uri: from_uri_string,
        from_tag: from_tag.into(),
        to_uri: to_uri_string.clone(),
        to_tag: to_tag.into(),
        request_uri: to_uri_string,
        cseq,
        local_address,
        route_set,
        contact: Some(contact_uri), // Include Contact header for target refresh capability
    };

    let content_type = if sdp_content.is_some() {
        Some("application/sdp".to_string())
    } else {
        None
    };

    request_builder_from_dialog_template(&template, Method::Update, sdp_content, content_type, None)
}

/// Quick INFO request creation for mid-dialog information
///
/// Creates an INFO request to send application-specific information.
///
/// # Arguments
/// * `call_id` - Dialog Call-ID
/// * `from_uri` - Local URI (From header)
/// * `from_tag` - Local tag (From header tag)
/// * `to_uri` - Remote URI (To header)
/// * `to_tag` - Remote tag (To header tag)
/// * `content` - Information content to send
/// * `content_type` - Optional content type (defaults to "application/info")
/// * `cseq` - Next CSeq number for the dialog
/// * `local_address` - Local address for Via header
/// * `route_set` - Optional route set for proxy routing
///
/// # Returns
/// Ready-to-send INFO request
///
/// # Example
/// ```rust,no_run
/// use rvoip_dialog_core::transaction::dialog::quick::info_for_dialog;
/// use std::net::SocketAddr;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
/// let info_request = info_for_dialog(
///     "call-123",
///     "sip:alice@example.com",
///     "alice-tag",
///     "sip:bob@example.com",
///     "bob-tag",
///     "Custom application data",
///     Some("application/custom".to_string()),
///     2,
///     local_addr,
///     None
/// )?;
/// # Ok(())
/// # }
/// ```
pub fn info_for_dialog(
    call_id: impl Into<String>,
    from_uri: impl Into<String>,
    from_tag: impl Into<String>,
    to_uri: impl Into<String>,
    to_tag: impl Into<String>,
    content: impl Into<String>,
    content_type: Option<String>,
    cseq: u32,
    local_address: SocketAddr,
    route_set: Option<Vec<Uri>>,
) -> Result<Request> {
    let to_uri_string = to_uri.into();
    let template = DialogRequestTemplate {
        call_id: call_id.into(),
        from_uri: from_uri.into(),
        from_tag: from_tag.into(),
        to_uri: to_uri_string.clone(),
        to_tag: to_tag.into(),
        request_uri: to_uri_string,
        cseq,
        local_address,
        route_set: route_set.unwrap_or_default(),
        contact: None,
    };

    let ct = content_type.unwrap_or_else(|| "application/info".to_string());
    request_builder_from_dialog_template(
        &template,
        Method::Info,
        Some(content.into()),
        Some(ct),
        None,
    )
}

/// Quick NOTIFY request creation for event notifications
///
/// Creates a NOTIFY request to send event notifications within a dialog.
///
/// # Arguments
/// * `call_id` - Dialog Call-ID
/// * `from_uri` - Local URI (From header)
/// * `from_tag` - Local tag (From header tag)
/// * `to_uri` - Remote URI (To header)
/// * `to_tag` - Remote tag (To header tag)
/// * `event_type` - Event type for the notification
/// * `notification_body` - Optional notification body
/// * `cseq` - Next CSeq number for the dialog
/// * `local_address` - Local address for Via header
/// * `route_set` - Optional route set for proxy routing
///
/// # Returns
/// Ready-to-send NOTIFY request
///
/// # Example
/// ```rust,no_run
/// use rvoip_dialog_core::transaction::dialog::quick::notify_for_dialog;
/// use std::net::SocketAddr;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
/// let notify_request = notify_for_dialog(
///     "call-123",
///     "sip:alice@example.com",
///     "alice-tag",
///     "sip:bob@example.com",
///     "bob-tag",
///     "dialog",
///     Some("Dialog state information".to_string()),
///     Some("active".to_string()), // subscription_state (RFC 6665)
///     2,
///     local_addr,
///     None
/// )?;
/// # Ok(())
/// # }
/// ```
pub fn notify_for_dialog(
    call_id: impl Into<String>,
    from_uri: impl Into<String>,
    from_tag: impl Into<String>,
    to_uri: impl Into<String>,
    to_tag: impl Into<String>,
    event_type: impl Into<String>,
    notification_body: Option<String>,
    subscription_state: Option<String>,
    cseq: u32,
    local_address: SocketAddr,
    route_set: Option<Vec<Uri>>,
) -> Result<Request> {
    use crate::transaction::client::builders::InDialogRequestBuilder;

    let to_uri_string = to_uri.into();
    let template = DialogRequestTemplate {
        call_id: call_id.into(),
        from_uri: from_uri.into(),
        from_tag: from_tag.into(),
        to_uri: to_uri_string.clone(),
        to_tag: to_tag.into(),
        request_uri: to_uri_string.clone(),
        cseq,
        local_address,
        route_set: route_set.unwrap_or_default(),
        contact: None,
    };

    // Use InDialogRequestBuilder directly for NOTIFY since it handles Event headers properly
    let mut builder = InDialogRequestBuilder::for_notify(event_type, notification_body);

    // Add subscription state if provided (required for RFC 6665 compliance)
    if let Some(state) = subscription_state {
        builder = builder.with_subscription_state(state);
    }

    builder = builder.from_dialog_enhanced(
        &template.call_id,
        &template.from_uri,
        &template.from_tag,
        &template.to_uri,
        &template.to_tag,
        &to_uri_string,
        template.cseq,
        template.local_address,
        template.route_set,
    );

    builder.build()
}

/// Quick MESSAGE request creation for instant messaging
///
/// Creates a MESSAGE request to send an instant message within a dialog.
///
/// # Arguments
/// * `call_id` - Dialog Call-ID
/// * `from_uri` - Local URI (From header)
/// * `from_tag` - Local tag (From header tag)
/// * `to_uri` - Remote URI (To header)
/// * `to_tag` - Remote tag (To header tag)
/// * `message_content` - The message content to send
/// * `content_type` - Optional content type (defaults to "text/plain")
/// * `cseq` - Next CSeq number for the dialog
/// * `local_address` - Local address for Via header
/// * `route_set` - Optional route set for proxy routing
///
/// # Returns
/// Ready-to-send MESSAGE request
///
/// # Example
/// ```rust,no_run
/// use rvoip_dialog_core::transaction::dialog::quick::message_for_dialog;
/// use std::net::SocketAddr;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
/// let message_request = message_for_dialog(
///     "call-123",
///     "sip:alice@example.com",
///     "alice-tag",
///     "sip:bob@example.com",
///     "bob-tag",
///     "Hello from Alice!",
///     Some("text/plain".to_string()),
///     2,
///     local_addr,
///     None
/// )?;
/// # Ok(())
/// # }
/// ```
pub fn message_for_dialog(
    call_id: impl Into<String>,
    from_uri: impl Into<String>,
    from_tag: impl Into<String>,
    to_uri: impl Into<String>,
    to_tag: impl Into<String>,
    message_content: impl Into<String>,
    content_type: Option<String>,
    cseq: u32,
    local_address: SocketAddr,
    route_set: Option<Vec<Uri>>,
) -> Result<Request> {
    let to_uri_string = to_uri.into();
    let template = DialogRequestTemplate {
        call_id: call_id.into(),
        from_uri: from_uri.into(),
        from_tag: from_tag.into(),
        to_uri: to_uri_string.clone(),
        to_tag: to_tag.into(),
        request_uri: to_uri_string,
        cseq,
        local_address,
        route_set: route_set.unwrap_or_default(),
        contact: None,
    };

    let ct = content_type.unwrap_or_else(|| "text/plain".to_string());
    request_builder_from_dialog_template(
        &template,
        Method::Message,
        Some(message_content.into()),
        Some(ct),
        None,
    )
}

/// Quick re-INVITE request creation for session modification
///
/// Creates a re-INVITE request to modify an existing session (change media, etc.).
///
/// # Arguments
/// * `call_id` - Dialog Call-ID
/// * `from_uri` - Local URI (From header)
/// * `from_tag` - Local tag (From header tag)
/// * `to_uri` - Remote URI (To header)
/// * `to_tag` - Remote tag (To header tag)
/// * `sdp_offer` - SDP offer for the re-INVITE
/// * `cseq` - Next CSeq number for the dialog
/// * `local_address` - Local address for Via header
/// * `route_set` - Optional route set for proxy routing
/// * `contact` - Optional contact URI
///
/// # Returns
/// Ready-to-send re-INVITE request
///
/// # Example
/// ```rust,no_run
/// use rvoip_dialog_core::transaction::dialog::quick::reinvite_for_dialog;
/// use std::net::SocketAddr;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
/// let reinvite_request = reinvite_for_dialog(
///     "call-123",
///     "sip:alice@example.com",
///     "alice-tag",
///     "sip:bob@example.com",
///     "bob-tag",
///     "v=0\r\no=alice 456 789 IN IP4 127.0.0.1\r\n...",
///     2,
///     local_addr,
///     None,
///     Some("sip:alice@192.168.1.100".to_string())
/// )?;
/// # Ok(())
/// # }
/// ```
pub fn reinvite_for_dialog(
    call_id: impl Into<String>,
    from_uri: impl Into<String>,
    from_tag: impl Into<String>,
    to_uri: impl Into<String>,
    to_tag: impl Into<String>,
    sdp_offer: impl Into<String>,
    cseq: u32,
    local_address: SocketAddr,
    route_set: Option<Vec<Uri>>,
    contact: Option<String>,
) -> Result<Request> {
    let to_uri_string = to_uri.into();
    let template = DialogRequestTemplate {
        call_id: call_id.into(),
        from_uri: from_uri.into(),
        from_tag: from_tag.into(),
        to_uri: to_uri_string.clone(),
        to_tag: to_tag.into(),
        request_uri: to_uri_string,
        cseq,
        local_address,
        route_set: route_set.unwrap_or_default(),
        contact,
    };

    request_builder_from_dialog_template(
        &template,
        Method::Invite,
        Some(sdp_offer.into()),
        Some("application/sdp".to_string()),
        None,
    )
}

/// Quick PRACK request creation (RFC 3262 §7.2)
///
/// Creates a PRACK request acknowledging a reliable provisional (18x)
/// response. The `RAck` header is populated from the `RSeq` of the 18x
/// being acknowledged, plus the CSeq and method of the original INVITE.
///
/// # Arguments
/// * `call_id` - Dialog Call-ID
/// * `from_uri` - Local URI (From header)
/// * `from_tag` - Local tag (From header tag)
/// * `to_uri` - Remote URI (To header)
/// * `to_tag` - Remote tag (To header tag) — required; PRACK is in-dialog
/// * `rseq` - `RSeq` value from the reliable 18x being acknowledged
/// * `invite_cseq` - CSeq of the original INVITE that produced the 18x
/// * `prack_cseq` - Next local CSeq to use for this PRACK
/// * `local_address` - Local address for Via header
/// * `route_set` - Optional route set for proxy routing
pub fn prack_for_dialog(
    call_id: impl Into<String>,
    from_uri: impl Into<String>,
    from_tag: impl Into<String>,
    to_uri: impl Into<String>,
    to_tag: impl Into<String>,
    rseq: u32,
    invite_cseq: u32,
    prack_cseq: u32,
    local_address: SocketAddr,
    route_set: Option<Vec<Uri>>,
) -> Result<Request> {
    let to_uri_string = to_uri.into();
    let template = DialogRequestTemplate {
        call_id: call_id.into(),
        from_uri: from_uri.into(),
        from_tag: from_tag.into(),
        to_uri: to_uri_string.clone(),
        to_tag: to_tag.into(),
        request_uri: to_uri_string,
        cseq: prack_cseq,
        local_address,
        route_set: route_set.unwrap_or_default(),
        contact: None,
    };

    let mut request =
        request_builder_from_dialog_template(&template, Method::Prack, None, None, None)?;

    // The generic builder doesn't know about RAck; append it here per RFC 3262 §7.2.
    let rack = RAck::new(rseq, invite_cseq, Method::Invite);
    request.headers.push(TypedHeader::RAck(rack));

    Ok(request)
}

/// Create an out-of-dialog SUBSCRIBE request.
pub fn subscribe_out_of_dialog(
    target_uri: impl Into<String>,
    from_uri: impl Into<String>,
    contact_uri: impl Into<String>,
    event_package: impl Into<String>,
    expires: u32,
    cseq: u32,
    local_address: SocketAddr,
) -> Result<Request> {
    let target_uri = target_uri.into();
    let from_uri = from_uri.into();
    let contact_uri = contact_uri.into();
    let event_package = event_package.into();
    let branch = crate::transaction::utils::dialog_utils::generate_branch();
    let via_transport = via_transport_for_uris(&target_uri, &from_uri);

    let request = rvoip_sip_core::builder::SimpleRequestBuilder::subscribe(
        &target_uri,
        &event_package,
        expires,
    )
    .map_err(|e| Error::Other(format!("Failed to build SUBSCRIBE request: {}", e)))?
    .from(
        "",
        &from_uri,
        Some(&format!("tag-{}", uuid::Uuid::new_v4().simple())),
    )
    .to("", &target_uri, None)
    .call_id(&format!("sub-{}", uuid::Uuid::new_v4()))
    .cseq(cseq)
    .via(&local_address.to_string(), via_transport, Some(&branch))
    .max_forwards(70)
    .contact(&contact_uri, None)
    .build();

    Ok(request)
}

/// Create an out-of-dialog MESSAGE request.
pub fn message_out_of_dialog(
    target_uri: impl Into<String>,
    from_uri: impl Into<String>,
    body: impl Into<String>,
    cseq: u32,
    local_address: SocketAddr,
) -> Result<Request> {
    let target_uri = target_uri.into();
    let from_uri = from_uri.into();
    let body = body.into();
    let branch = crate::transaction::utils::dialog_utils::generate_branch();
    let via_transport = via_transport_for_uris(&target_uri, &from_uri);

    let request = rvoip_sip_core::builder::SimpleRequestBuilder::new(Method::Message, &target_uri)
        .map_err(|e| Error::Other(format!("Failed to build MESSAGE request: {}", e)))?
        .from(
            "",
            &from_uri,
            Some(&format!("tag-{}", uuid::Uuid::new_v4().simple())),
        )
        .to("", &target_uri, None)
        .call_id(&format!("msg-{}", uuid::Uuid::new_v4()))
        .cseq(cseq)
        .via(&local_address.to_string(), via_transport, Some(&branch))
        .max_forwards(70)
        .content_type("text/plain")
        .body(bytes::Bytes::from(body))
        .build();

    Ok(request)
}

/// Quick response creation for dialog transactions
///
/// Creates an appropriate response for a dialog transaction with automatic
/// dialog-aware processing.
///
/// # Arguments
/// * `transaction_id` - Transaction identifier
/// * `original_request` - The original request to respond to
/// * `dialog_id` - Optional dialog identifier
/// * `status_code` - SIP status code for the response
/// * `local_address` - Local address for Contact header
/// * `sdp_content` - Optional SDP content for the response
/// * `custom_reason` - Optional custom reason phrase
///
/// # Returns
/// Ready-to-send response
///
/// # Example
/// ```rust,no_run
/// use rvoip_dialog_core::transaction::dialog::quick::response_for_dialog_transaction;
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
/// let response = response_for_dialog_transaction(
///     "txn-123",
///     original_request,
///     Some("dialog-456".to_string()),
///     StatusCode::Ok,
///     local_addr,
///     Some("v=0\r\no=server 456 789 IN IP4 127.0.0.1\r\n...".to_string()),
///     None
/// )?;
/// # Ok(())
/// # }
/// ```
pub fn response_for_dialog_transaction(
    transaction_id: impl Into<String>,
    original_request: Request,
    dialog_id: Option<String>,
    status_code: StatusCode,
    local_address: SocketAddr,
    sdp_content: Option<String>,
    custom_reason: Option<String>,
) -> Result<Response> {
    let context = DialogTransactionContext {
        dialog_id,
        transaction_id: transaction_id.into(),
        original_request,
        is_dialog_creating: false, // Will be determined automatically
        local_address,
    };

    let mut response = response_builder_for_dialog_transaction(
        &context,
        status_code,
        Some(local_address),
        sdp_content,
    )?;

    // Apply custom reason phrase if provided
    if let Some(reason) = custom_reason {
        response = response.with_reason(reason);
    }

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    #[tokio::test]
    async fn test_quick_bye_for_dialog() {
        let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();

        let bye_request = bye_for_dialog(
            "call-123",
            "sip:alice@example.com",
            "alice-tag",
            "sip:bob@example.com",
            "bob-tag",
            3,
            local_addr,
            None,
            None,
        )
        .expect("Failed to create BYE");

        assert_eq!(bye_request.method(), Method::Bye);
        assert_eq!(bye_request.call_id().unwrap().value(), "call-123");
        assert_eq!(bye_request.from().unwrap().tag().unwrap(), "alice-tag");
        assert_eq!(bye_request.to().unwrap().tag().unwrap(), "bob-tag");
        assert_eq!(bye_request.cseq().unwrap().seq, 3);
    }

    #[tokio::test]
    async fn test_quick_refer_for_dialog() {
        let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();

        let refer_request = refer_for_dialog(
            "call-456",
            "sip:alice@example.com",
            "alice-tag",
            "sip:bob@example.com",
            "bob-tag",
            "sip:charlie@example.com",
            2,
            local_addr,
            None,
        )
        .expect("Failed to create REFER");

        assert_eq!(refer_request.method(), Method::Refer);
        assert_eq!(refer_request.call_id().unwrap().value(), "call-456");
        assert_eq!(refer_request.cseq().unwrap().seq, 2);

        // Check that Refer-To is in the headers, not the body
        use rvoip_sip_core::types::refer_to::ReferTo;
        let refer_to_header = refer_request.typed_header::<ReferTo>();
        assert!(
            refer_to_header.is_some(),
            "Refer-To header should be present"
        );
        assert_eq!(
            refer_to_header.unwrap().uri().to_string(),
            "sip:charlie@example.com"
        );

        // Body should be empty since Refer-To is now a header
        assert_eq!(refer_request.body().len(), 0, "Body should be empty");
    }

    #[tokio::test]
    async fn test_quick_update_for_dialog() {
        let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        let sdp_content = "v=0\r\no=alice 456 789 IN IP4 127.0.0.1\r\n";

        let update_request = update_for_dialog(
            "call-789",
            "sip:alice@example.com",
            "alice-tag",
            "sip:bob@example.com",
            "bob-tag",
            Some(sdp_content.to_string()),
            4,
            local_addr,
            None,
        )
        .expect("Failed to create UPDATE");

        assert_eq!(update_request.method(), Method::Update);
        assert_eq!(update_request.call_id().unwrap().value(), "call-789");
        assert_eq!(update_request.cseq().unwrap().seq, 4);
        assert_eq!(update_request.body(), sdp_content.as_bytes());
    }

    #[tokio::test]
    async fn update_for_dialog_uses_sips_contact_for_tls_route() {
        use rvoip_sip_core::types::headers::HeaderAccess;
        use rvoip_sip_core::HeaderName;

        let local_addr: SocketAddr = "192.0.2.10:5071".parse().unwrap();
        let route: Uri = "sips:proxy.example.com:5061;lr;transport=tls"
            .parse()
            .unwrap();

        let update_request = update_for_dialog(
            "call-789",
            "sip:alice@example.com",
            "alice-tag",
            "sip:bob@example.com",
            "bob-tag",
            Some("v=0\r\n".to_string()),
            4,
            local_addr,
            Some(vec![route]),
        )
        .expect("Failed to create UPDATE");

        assert_eq!(update_request.first_via_transport(), Some("TLS"));
        assert_eq!(
            update_request
                .raw_header_value(&HeaderName::Contact)
                .unwrap(),
            "<sips:alice@192.0.2.10:5071;transport=tls>"
        );
        rvoip_sip_core::validation::validate_wire_request(&update_request).unwrap();
    }

    #[tokio::test]
    async fn out_of_dialog_subscribe_uses_tls_via_for_sips_target() {
        let local_addr: SocketAddr = "192.0.2.10:5071".parse().unwrap();

        let request = subscribe_out_of_dialog(
            "sips:events@pbx.example.com;transport=tls",
            "sips:1001@pbx.example.com",
            "sips:1001@192.0.2.10:5071;transport=tls",
            "dialog",
            3600,
            1,
            local_addr,
        )
        .expect("SUBSCRIBE");

        assert_eq!(request.first_via_transport(), Some("TLS"));
        rvoip_sip_core::validation::validate_wire_request(&request).unwrap();
    }

    #[tokio::test]
    async fn out_of_dialog_message_uses_tls_via_for_sips_target() {
        let local_addr: SocketAddr = "192.0.2.10:5071".parse().unwrap();

        let request = message_out_of_dialog(
            "sips:1002@pbx.example.com;transport=tls",
            "sips:1001@pbx.example.com",
            "hello",
            1,
            local_addr,
        )
        .expect("MESSAGE");

        assert_eq!(request.first_via_transport(), Some("TLS"));
        rvoip_sip_core::validation::validate_wire_request(&request).unwrap();
    }

    #[tokio::test]
    async fn test_quick_info_for_dialog() {
        let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        let info_content = "Custom application data";

        let info_request = info_for_dialog(
            "call-012",
            "sip:alice@example.com",
            "alice-tag",
            "sip:bob@example.com",
            "bob-tag",
            info_content,
            Some("application/custom".to_string()),
            5,
            local_addr,
            None,
        )
        .expect("Failed to create INFO");

        assert_eq!(info_request.method(), Method::Info);
        assert_eq!(info_request.call_id().unwrap().value(), "call-012");
        assert_eq!(info_request.cseq().unwrap().seq, 5);
        assert_eq!(info_request.body(), info_content.as_bytes());
    }

    #[tokio::test]
    async fn test_quick_notify_for_dialog() {
        let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        let notification_body = "Dialog state information";

        let notify_request = notify_for_dialog(
            "call-345",
            "sip:alice@example.com",
            "alice-tag",
            "sip:bob@example.com",
            "bob-tag",
            "dialog",
            Some(notification_body.to_string()),
            None, // subscription_state
            6,
            local_addr,
            None,
        )
        .expect("Failed to create NOTIFY");

        assert_eq!(notify_request.method(), Method::Notify);
        assert_eq!(notify_request.call_id().unwrap().value(), "call-345");
        assert_eq!(notify_request.cseq().unwrap().seq, 6);
        assert_eq!(notify_request.body(), notification_body.as_bytes());
        assert!(notify_request
            .header(&rvoip_sip_core::HeaderName::ContentType)
            .is_some());
        rvoip_sip_core::validation::validate_wire_request(&notify_request).unwrap();
    }

    #[tokio::test]
    async fn test_quick_message_for_dialog() {
        let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        let message_content = "Hello from Alice!";

        let message_request = message_for_dialog(
            "call-678",
            "sip:alice@example.com",
            "alice-tag",
            "sip:bob@example.com",
            "bob-tag",
            message_content,
            Some("text/plain".to_string()),
            7,
            local_addr,
            None,
        )
        .expect("Failed to create MESSAGE");

        assert_eq!(message_request.method(), Method::Message);
        assert_eq!(message_request.call_id().unwrap().value(), "call-678");
        assert_eq!(message_request.cseq().unwrap().seq, 7);
        assert_eq!(message_request.body(), message_content.as_bytes());
    }

    #[tokio::test]
    async fn test_quick_reinvite_for_dialog() {
        let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        let sdp_offer = "v=0\r\no=alice 456 789 IN IP4 127.0.0.1\r\n";

        let reinvite_request = reinvite_for_dialog(
            "call-901",
            "sip:alice@example.com",
            "alice-tag",
            "sip:bob@example.com",
            "bob-tag",
            sdp_offer,
            8,
            local_addr,
            None,
            Some("sip:alice@192.168.1.100".to_string()),
        )
        .expect("Failed to create re-INVITE");

        assert_eq!(reinvite_request.method(), Method::Invite);
        assert_eq!(reinvite_request.call_id().unwrap().value(), "call-901");
        assert_eq!(reinvite_request.cseq().unwrap().seq, 8);
        assert_eq!(reinvite_request.body(), sdp_offer.as_bytes());
    }
}
