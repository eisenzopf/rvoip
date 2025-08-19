//! Transaction helper utilities for transaction-core
//! 
//! This module provides utilities for working with SIP transactions,
//! including transaction key extraction and classification.

use rvoip_sip_core::prelude::*;
use crate::transaction::error::{Error, Result};
use crate::transaction::{TransactionKey, TransactionKind};
use tracing::debug;

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
            let (_, cseq_method) = extract_cseq(message)
                .ok_or_else(|| Error::Other("Missing or invalid CSeq header in Response".to_string()))?;

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
        Message::Request(request) => {
            // Get Via header using TypedHeader
            if let Some(via) = request.typed_header::<Via>() {
                if let Some(first_via) = via.0.first() {
                    if let Some(branch) = first_via.branch() {
                        let method = request.method();
                        let key = TransactionKey::new(branch.to_string(), method.clone(), true);
                        debug!("🔍 TX_KEY: Generated server key from request {}: {}", method, key);
                        return Some(key);
                    }
                }
            }
            debug!("🔍 TX_KEY: Could not generate key from request - missing Via/branch");
            None
        }
        Message::Response(response) => {
            // Get Via header using TypedHeader
            if let Some(via) = response.typed_header::<Via>() {
                if let Some(first_via) = via.0.first() {
                    if let Some(branch) = first_via.branch() {
                        // Get method from CSeq header
                        if let Some(cseq) = response.typed_header::<CSeq>() {
                            let key = TransactionKey::new(branch.to_string(), cseq.method.clone(), false);
                            debug!("🔍 TX_KEY: Generated client key from response {} {}: {}", response.status(), cseq.method, key);
                            return Some(key);
                        } else {
                            debug!("🔍 TX_KEY: Response missing CSeq header");
                        }
                    } else {
                        debug!("🔍 TX_KEY: Response Via header missing branch parameter");
                    }
                } else {
                    debug!("🔍 TX_KEY: Response Via header is empty");
                }
            } else {
                debug!("🔍 TX_KEY: Response missing Via header");
            }
            None
        }
    }
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