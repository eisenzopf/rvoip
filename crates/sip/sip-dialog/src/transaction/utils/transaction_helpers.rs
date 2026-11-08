//! Transaction helper utilities for transaction-core
//!
//! This module provides utilities for working with SIP transactions,
//! including transaction key extraction and classification.

use crate::transaction::error::{Error, Result};
use crate::transaction::{TransactionKey, TransactionKind};
use rvoip_sip_core::prelude::*;
use tracing::{debug, error, info};

use super::message_extractors::{extract_branch, extract_cseq};

/// Extract the transaction classification (prefix) and branch from a message
/// Used by manager to determine transaction type and potentially match.
pub fn extract_transaction_parts(message: &Message) -> Result<(TransactionKind, String)> {
    let branch = extract_branch(message)
        .ok_or_else(|| Error::Other("Missing branch parameter in Via header".to_string()))?;

    let kind = match message {
        Message::Request(req) => {
            match req.method() {
                Method::Invite => TransactionKind::InviteServer,
                Method::Ack => TransactionKind::InviteServer, // Matches existing IST
                Method::Cancel => TransactionKind::InviteServer, // Matches existing IST
                _ => TransactionKind::NonInviteServer,
            }
        }
        Message::Response(_) => {
            let (_, cseq_method) = extract_cseq(message).ok_or_else(|| {
                Error::Other("Missing or invalid CSeq header in Response".to_string())
            })?;

            if cseq_method == Method::Invite {
                TransactionKind::InviteClient
            } else {
                TransactionKind::NonInviteClient
            }
        }
    };

    Ok((kind, branch))
}

/// Extract a transaction key from a SIP message if possible.
pub fn transaction_key_from_message(message: &Message) -> Option<TransactionKey> {
    match message {
        Message::Request(request) => transaction_key_from_request(request),
        Message::Response(response) => transaction_key_from_response(response),
    }
}

/// Extract the server transaction key from a borrowed request: the RFC 3261
/// branch, or the RFC 2543 transaction identifier (RFC 3261 §17.2.3).
///
/// Same result as [`transaction_key_from_message`] for `Message::Request`,
/// without wrapping (and therefore deep-copying) the request into a
/// [`Message`] first.
pub fn transaction_key_from_request(request: &Request) -> Option<TransactionKey> {
    let key = TransactionKey::from_request(request);
    match &key {
        Some(key) => debug!(
            method=%crate::transaction::safe_diagnostics::SafeMethod::new(&request.method()),
            transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(key),
            "Generated server transaction key from request"
        ),
        None => debug!("🔍 TX_KEY: Could not generate key from request - missing Via"),
    }
    key
}

/// Extract the client transaction key from a borrowed response.
///
/// Same result as [`transaction_key_from_message`] for `Message::Response`,
/// without wrapping (and therefore deep-copying) the response into a
/// [`Message`] first.
pub fn transaction_key_from_response(response: &Response) -> Option<TransactionKey> {
    // Get Via header using TypedHeader
    if let Some(via) = response.typed_header::<Via>() {
        if let Some(first_via) = via.0.first() {
            if let Some(branch) = first_via.branch() {
                // Get method from CSeq header
                if let Some(cseq) = response.typed_header::<CSeq>() {
                    let key = TransactionKey::new(branch.to_string(), cseq.method.clone(), false);
                    info!(
                        status=%response.status(),
                        method=%crate::transaction::safe_diagnostics::SafeMethod::new(&cseq.method),
                        transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&key),
                        "Generated client transaction key from response"
                    );
                    debug!(
                        status=%response.status(),
                        method=%crate::transaction::safe_diagnostics::SafeMethod::new(&cseq.method),
                        transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&key),
                        "Generated client transaction key from response"
                    );
                    return Some(key);
                } else {
                    error!("🔍 TX_KEY: Response missing CSeq header!");
                    debug!("🔍 TX_KEY: Response missing CSeq header");
                }
            } else {
                error!("🔍 TX_KEY: Response Via header missing branch parameter!");
                debug!("🔍 TX_KEY: Response Via header missing branch parameter");
            }
        } else {
            error!("🔍 TX_KEY: Response Via header is empty!");
            debug!("🔍 TX_KEY: Response Via header is empty");
        }
    } else {
        error!("🔍 TX_KEY: Response missing Via header!");
        debug!("🔍 TX_KEY: Response missing Via header");
    }
    None
}

/// Determine which kind of transaction to create based on the request method.
pub fn determine_transaction_kind(request: &Request, is_server: bool) -> TransactionKind {
    match (request.method(), is_server) {
        (Method::Invite, true) => TransactionKind::InviteServer,
        (Method::Invite, false) => TransactionKind::InviteClient,
        (_, true) => TransactionKind::NonInviteServer,
        (_, false) => TransactionKind::NonInviteClient,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};

    fn bye(via_branch: Option<Option<&str>>) -> Request {
        let builder = SimpleRequestBuilder::new(Method::Bye, "sip:bob@example.com")
            .expect("request builder")
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", Some("bob-tag"))
            .call_id("transaction-key-helpers")
            .cseq(2);
        match via_branch {
            Some(branch) => builder.via("192.0.2.10:5060", "UDP", branch).build(),
            None => builder.build(),
        }
    }

    fn ok(via_branch: Option<Option<&str>>, cseq: bool) -> Response {
        let mut builder = SimpleResponseBuilder::new(StatusCode::Ok, None);
        if let Some(branch) = via_branch {
            builder = builder.via("192.0.2.10:5060", "UDP", branch);
        }
        if cseq {
            builder = builder.cseq(2, Method::Bye);
        }
        builder.build()
    }

    #[test]
    fn request_key_matches_the_message_key() {
        for request in [
            bye(Some(Some("z9hG4bK-key-helpers"))),
            bye(Some(None)),
            bye(None),
        ] {
            assert_eq!(
                transaction_key_from_request(&request),
                transaction_key_from_message(&Message::Request(request.clone())),
            );
        }
        assert_eq!(
            transaction_key_from_request(&bye(Some(Some("z9hG4bK-key-helpers")))),
            Some(TransactionKey::new(
                "z9hG4bK-key-helpers".to_string(),
                Method::Bye,
                true,
            )),
        );
        assert_eq!(transaction_key_from_request(&bye(None)), None);
    }

    #[test]
    fn response_key_matches_the_message_key() {
        for response in [
            ok(Some(Some("z9hG4bK-key-helpers")), true),
            ok(Some(Some("z9hG4bK-key-helpers")), false),
            ok(Some(None), true),
            ok(None, true),
        ] {
            assert_eq!(
                transaction_key_from_response(&response),
                transaction_key_from_message(&Message::Response(response.clone())),
            );
        }
        assert_eq!(
            transaction_key_from_response(&ok(Some(Some("z9hG4bK-key-helpers")), true)),
            Some(TransactionKey::new(
                "z9hG4bK-key-helpers".to_string(),
                Method::Bye,
                false,
            )),
        );
        assert_eq!(
            transaction_key_from_response(&ok(Some(Some("z9hG4bK-key-helpers")), false)),
            None,
        );
    }
}
