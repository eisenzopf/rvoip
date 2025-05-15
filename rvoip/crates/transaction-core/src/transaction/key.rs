use std::fmt;
// use std::net::SocketAddr; // This import seems unused in this file. Commenting out.
use std::hash::{Hash, Hasher};

use rvoip_sip_core::prelude::*;
// Removed: use rvoip_sip_core::common::Branch;

use rvoip_sip_core::{
    Method,
    Request, Response, // Added Response
    StatusCode, // Added StatusCode
    types::{
        uri::Uri,
        param::Param,
        via::Via, // Added Via
        cseq::CSeq,
        call_id::CallId,
        from::From,
        to::To,
        address::Address,
        headers::header_name::HeaderName, // Ensure this is the only HeaderName import
    },
    Version, // Added Version
};

/// Uniquely identifies a SIP transaction.
///
/// According to RFC 3261, Section 17, the transaction identifier is a combination of:
/// 1. The `branch` parameter in the top-most `Via` header.
/// 2. The `Method` of the request (e.g., INVITE, REGISTER).
/// 3. Whether the transaction is a client or server transaction.
///
/// For client transactions (Section 17.1.3), the ID is `branch` + `sent-by` + `method`.
/// The `sent-by` component (host and port from Via) helps disambiguate if multiple clients
/// share the same branch generation logic. However, a globally unique `branch` (starting with `z9hG4bK`)
/// is the primary mechanism for uniqueness.
///
/// For server transactions (Section 17.2.3), the ID is also effectively `branch` + `sent-by` + `method`.
/// The `branch` parameter from the top Via header of the request is used.
///
/// This `TransactionKey` struct simplifies this by using the `branch` string, the `method`,
/// and an `is_server` boolean flag to ensure uniqueness within a single transaction manager instance.
/// It assumes the `branch` parameter is generated according to RFC 3261 to be sufficiently unique.
#[derive(Clone)]
pub struct TransactionKey {
    /// The value of the `branch` parameter from the top-most `Via` header.
    /// This is a critical part of the transaction identifier.
    pub branch: String,
    
    /// The SIP method of the request that initiated or is part of the transaction (e.g., INVITE, ACK, BYE).
    /// This is important because a request with the same branch but different method
    /// (e.g., an INVITE and a CANCEL for that INVITE) can belong to different transactions
    /// or be processed in context of the same INVITE transaction depending on rules.
    /// However, for keying, RFC3261 implies INVITE and non-INVITE transactions are distinct even with the same branch.
    pub method: Method,

    /// Distinguishes between client and server transactions.
    /// `true` if this key represents a server transaction, `false` for a client transaction.
    /// This is necessary because a User Agent can be both a client and a server, and might
    /// (though unlikely with proper branch generation) encounter or generate messages
    /// that could lead to key collisions if this flag were not present.
    pub is_server: bool,
}

impl TransactionKey {
    /// Creates a new `TransactionKey`.
    ///
    /// # Arguments
    /// * `branch` - The branch parameter string.
    /// * `method` - The SIP method associated with the transaction.
    /// * `is_server` - `true` if this is a server transaction, `false` otherwise.
    pub fn new(branch: String, method: Method, is_server: bool) -> Self {
        Self {
            branch,
            method,
            is_server,
        }
    }

    /// Attempts to create a `TransactionKey` for a server transaction from an incoming request.
    ///
    /// Extracts the branch parameter from the top-most `Via` header and the request's method.
    /// Sets `is_server` to `true`.
    ///
    /// # Returns
    /// `Some(TransactionKey)` if the top Via header and its branch parameter are present.
    /// `None` otherwise (e.g., malformed request, no Via, or Via without a branch).
    pub fn from_request(request: &Request) -> Option<Self> {
        if let Some(via_header) = request.typed_header::<Via>() {
            if let Some(first_via_value) = via_header.0.first() {
                if let Some(branch_param) = first_via_value.branch() {
                    // Ensure branch is not empty, as per some interpretations of RFC for keying.
                    if branch_param.is_empty() {
                        return None;
                    }
                    let method = request.method();
                    return Some(Self::new(branch_param.to_string(), method.clone(), true));
                }
            }
        }
        None
    }

    /// Attempts to create a `TransactionKey` for a client transaction from an outgoing response.
    ///
    /// Extracts the branch parameter from the top-most `Via` header (which was added by this client)
    /// and the method from the `CSeq` header of the response (which corresponds to the original request method).
    /// Sets `is_server` to `false`.
    ///
    /// # Returns
    /// `Some(TransactionKey)` if the top Via (with branch) and CSeq (with method) headers are present.
    /// `None` otherwise.
    pub fn from_response(response: &Response) -> Option<Self> {
        if let Some(via_header) = response.typed_header::<Via>() {
            if let Some(first_via_value) = via_header.0.first() {
                if let Some(branch_param) = first_via_value.branch() {
                    // Ensure branch is not empty.
                    if branch_param.is_empty() {
                        return None;
                    }
                    if let Some(cseq_header) = response.typed_header::<CSeq>() {
                        return Some(Self::new(branch_param.to_string(), cseq_header.method.clone(), false));
                    }
                }
            }
        }
        None
    }
    
    /// Returns the branch parameter of the transaction key.
    pub fn branch(&self) -> &str {
        &self.branch
    }

    /// Returns the method associated with the transaction key.
    pub fn method(&self) -> &Method {
        &self.method
    }

    /// Returns `true` if the key is for a server transaction, `false` otherwise.
    pub fn is_server(&self) -> bool {
        self.is_server
    }

    /// Returns a new TransactionKey with a different method but the same branch and is_server values
    pub fn with_method(&self, method: Method) -> Self {
        Self {
            branch: self.branch.clone(),
            method,
            is_server: self.is_server,
        }
    }
}

/// Provides a human-readable debug representation of the `TransactionKey`.
/// Format: "branch_value:METHOD:side" (e.g., "z9hG4bK123:INVITE:server")
impl fmt::Debug for TransactionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let side = if self.is_server { "server" } else { "client" };
        write!(f, "{}:{}:{}:{}", self.branch, self.method, side, if self.method == Method::Invite || self.method == Method::Ack { "INVITE_LIKE"} else {"NON_INVITE_LIKE"} )
    }
}

/// Provides a human-readable display representation of the `TransactionKey`.
/// Format: "branch_value:METHOD:side" (e.g., "z9hG4bK123:INVITE:server")
impl fmt::Display for TransactionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // For display, a slightly more compact form might be desired, or match Debug.
        // Sticking to a format similar to Debug for consistency.
        let side = if self.is_server { "server" } else { "client" };
        write!(f, "Key({}:{}:{})", self.branch, self.method, side)
    }
}

/// Implements equality for `TransactionKey`.
/// Two keys are equal if their `branch`, `method`, and `is_server` fields are all equal.
impl PartialEq for TransactionKey {
    fn eq(&self, other: &Self) -> bool {
        self.branch == other.branch && 
        self.method == other.method && 
        self.is_server == other.is_server
    }
}

/// Marks `TransactionKey` as implementing full equality.
impl Eq for TransactionKey {}

/// Implements hashing for `TransactionKey`.
/// The hash is derived from the `branch`, `method`, and `is_server` fields.
/// This allows `TransactionKey` to be used in hash-based collections like `HashMap` or `HashSet`.
impl Hash for TransactionKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.branch.hash(state);
        self.method.hash(state);
        self.is_server.hash(state);
    }
}

/// A type alias for `TransactionKey`, representing the unique identifier of a transaction.
/// Using `TransactionId` can sometimes be more semantically clear in certain contexts
/// than `TransactionKey`.
pub type TransactionId = TransactionKey;


#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::types::uri::Uri;
    use rvoip_sip_core::types::headers::header_name::HeaderName;
    use rvoip_sip_core::types::param::Param;
    use rvoip_sip_core::types::cseq::CSeq;
    use rvoip_sip_core::types::address::Address;
    use rvoip_sip_core::types::via::Via;
    use rvoip_sip_core::types::call_id::CallId;
    use rvoip_sip_core::types::from::From;
    use rvoip_sip_core::types::to::To;
    use rvoip_sip_core::StatusCode;
    use rvoip_sip_core::Method;
    use rvoip_sip_core::Request;
    use rvoip_sip_core::Response;
    use rvoip_sip_core::TypedHeader;
    use std::collections::HashSet;
    use std::str::FromStr;

    fn create_test_request_custom(
        method: Method,
        branch_opt: Option<&str>,
        add_via: bool,
    ) -> Request {
        let mut req = Request::new(method.clone(), Uri::sip("test@example.com"));
        if add_via {
            let via_host = "client.example.com:5060";
            let via_params = match branch_opt {
                Some(b_val) if !b_val.is_empty() => vec![Param::branch(b_val.to_string())],
                _ => Vec::new(), // For Some("") or None, use Via::new which won't auto-add branch if params are empty
            };
            // Use Via::new when we need specific control over parameters (like no branch or empty branch)
            let via_header_val = Via::new("SIP", "2.0", "UDP", via_host, None, via_params).unwrap();
            req.headers.push(TypedHeader::Via(via_header_val));
        }
        req.headers.push(TypedHeader::From(From::new(Address::new(Uri::sip("alice@localhost")))));
        req.headers.push(TypedHeader::To(To::new(Address::new(Uri::sip("bob@localhost")))));
        req.headers.push(TypedHeader::CallId(CallId::new("callid-test-key")));
        req.headers.push(TypedHeader::CSeq(CSeq::new(1, method)));
        req
    }

    fn create_test_response_custom(
        method_for_cseq: Method,
        branch_opt: Option<&str>,
        add_via: bool,
        add_cseq: bool,
    ) -> Response {
        let mut res = Response::new(StatusCode::Ok).with_reason("OK");
        if add_via {
            let via_host = "client.example.com:5060";
            let via_params = match branch_opt {
                Some(b_val) if !b_val.is_empty() => vec![Param::branch(b_val.to_string())],
                _ => Vec::new(),
            };
            let via_header_val = Via::new("SIP", "2.0", "UDP", via_host, None, via_params).unwrap();
            res.headers.push(TypedHeader::Via(via_header_val));
        }
        if add_cseq {
            res.headers.push(TypedHeader::CSeq(CSeq::new(1, method_for_cseq)));
        }
        res.headers.push(TypedHeader::From(From::new(Address::new(Uri::sip("alice@localhost")))));
        res.headers.push(TypedHeader::To(To::new(Address::new(Uri::sip("bob@localhost")))));
        res.headers.push(TypedHeader::CallId(CallId::new("callid-test-key")));
        res
    }

    #[test]
    fn test_transaction_key_new() {
        let key = TransactionKey::new("branch1".to_string(), Method::Invite, true);
        assert_eq!(key.branch(), "branch1");
        assert_eq!(*key.method(), Method::Invite);
        assert!(key.is_server());
    }

    #[test]
    fn test_from_request_success() {
        let req = create_test_request_custom(Method::Invite, Some("branch2"), true);
        let key = TransactionKey::from_request(&req).unwrap();
        assert_eq!(key.branch(), "branch2");
        assert_eq!(*key.method(), Method::Invite);
        assert!(key.is_server());
    }

    #[test]
    fn test_from_request_no_via() {
        let req = create_test_request_custom(Method::Invite, None, false);
        assert!(TransactionKey::from_request(&req).is_none());
    }

    #[test]
    fn test_from_request_via_no_branch() {
        let req = create_test_request_custom(Method::Invite, Some(""), true);
        assert!(TransactionKey::from_request(&req).is_none());
    }

    #[test]
    fn test_from_request_via_empty_branch() {
        let req = create_test_request_custom(Method::Invite, Some(""), true);
        assert!(TransactionKey::from_request(&req).is_none());
    }

    #[test]
    fn test_from_response_success() {
        let res = create_test_response_custom(Method::Invite, Some("branch3"), true, true);
        let key = TransactionKey::from_response(&res).unwrap();
        assert_eq!(key.branch(), "branch3");
        assert_eq!(*key.method(), Method::Invite);
        assert!(!key.is_server());
    }

    #[test]
    fn test_from_response_no_via() {
        let res = create_test_response_custom(Method::Invite, None, false, true);
        assert!(TransactionKey::from_response(&res).is_none());
    }

    #[test]
    fn test_from_response_via_no_branch() {
        let res = create_test_response_custom(Method::Invite, Some(""), true, true);
        assert!(TransactionKey::from_response(&res).is_none());
    }
    
    #[test]
    fn test_from_response_via_empty_branch() {
        let res = create_test_response_custom(Method::Invite, Some(""), true, true);
        assert!(TransactionKey::from_response(&res).is_none());
    }

    #[test]
    fn test_from_response_no_cseq() {
        let res = create_test_response_custom(Method::Invite, Some("branch4"), true, false);
        assert!(TransactionKey::from_response(&res).is_none());
    }

    #[test]
    fn test_transaction_key_equality() {
        let key1 = TransactionKey::new("b1".to_string(), Method::Invite, true);
        let key2 = TransactionKey::new("b1".to_string(), Method::Invite, true);
        let key3 = TransactionKey::new("b2".to_string(), Method::Invite, true); // Diff branch
        let key4 = TransactionKey::new("b1".to_string(), Method::Register, true); // Diff method
        let key5 = TransactionKey::new("b1".to_string(), Method::Invite, false); // Diff is_server

        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
        assert_ne!(key1, key4);
        assert_ne!(key1, key5);
    }

    #[test]
    fn test_transaction_key_hashing() {
        let key1 = TransactionKey::new("hash_branch".to_string(), Method::Ack, false);
        let key2 = TransactionKey::new("hash_branch".to_string(), Method::Ack, false);
        let key3 = TransactionKey::new("other_branch".to_string(), Method::Ack, false);

        let mut set = HashSet::new();
        assert!(set.insert(key1.clone()));
        assert!(!set.insert(key2.clone())); // Should not insert, as it's equal to key1
        assert!(set.insert(key3.clone()));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_transaction_key_display_debug_format() {
        let key_server = TransactionKey::new("z9hG4bKalpha".to_string(), Method::Invite, true);
        let key_client = TransactionKey::new("z9hG4bKbeta".to_string(), Method::Message, false);

        assert_eq!(format!("{}", key_server), "Key(z9hG4bKalpha:INVITE:server)");
        assert_eq!(format!("{:?}", key_server), "z9hG4bKalpha:INVITE:server:INVITE_LIKE");

        assert_eq!(format!("{}", key_client), "Key(z9hG4bKbeta:MESSAGE:client)");
        assert_eq!(format!("{:?}", key_client), "z9hG4bKbeta:MESSAGE:client:NON_INVITE_LIKE");
        
        // Test ACK for INVITE_LIKE in Debug
        let key_ack = TransactionKey::new("z9hG4bKgamma".to_string(), Method::Ack, true); // ACK for server
        assert_eq!(format!("{:?}", key_ack), "z9hG4bKgamma:ACK:server:INVITE_LIKE");
    }

    #[test]
    fn transaction_id_type_alias() {
        let key = TransactionKey::new("id_branch".to_string(), Method::Info, true);
        let id: TransactionId = key.clone();
        assert_eq!(key, id);
    }
} 