//! ICE connectivity check list management per RFC 8445 Section 6.1.2.
//!
//! Forms candidate pairs, sorts them by priority, prunes redundant pairs,
//! and builds STUN Binding Requests with ICE attributes for connectivity checks.

use std::net::SocketAddr;

use tracing::{debug, trace};

use crate::stun::message::{StunMessage, StunAttribute, BINDING_REQUEST};
use super::types::{
    CandidateType, CandidatePairState, ComponentId, IceCandidate,
    IceCandidatePair, IceCredentials, IceRole,
};

/// Form candidate pairs from local and remote candidates.
///
/// Per RFC 8445 Section 6.1.2.2, pairs are formed for each local candidate
/// with each remote candidate of the same component. Pairs are initialized
/// in the Frozen state.
pub fn form_candidate_pairs(
    local_candidates: &[IceCandidate],
    remote_candidates: &[IceCandidate],
    role: IceRole,
) -> Vec<IceCandidatePair> {
    let mut pairs = Vec::new();

    for local in local_candidates {
        for remote in remote_candidates {
            // Only pair candidates of the same component
            if local.component != remote.component {
                continue;
            }

            // Only pair candidates of the same transport
            if local.transport != remote.transport {
                continue;
            }

            // Only pair same address family (IPv4 with IPv4, IPv6 with IPv6)
            if local.address.is_ipv4() != remote.address.is_ipv4() {
                continue;
            }

            let priority = match role {
                IceRole::Controlling => {
                    IceCandidatePair::compute_priority(local.priority, remote.priority)
                }
                IceRole::Controlled => {
                    IceCandidatePair::compute_priority(remote.priority, local.priority)
                }
            };

            pairs.push(IceCandidatePair {
                local: local.clone(),
                remote: remote.clone(),
                state: CandidatePairState::Frozen,
                priority,
                nominated: false,
            });
        }
    }

    pairs
}

/// Sort candidate pairs by priority (highest first) per RFC 8445 Section 6.1.2.3.
pub fn sort_pairs(pairs: &mut [IceCandidatePair]) {
    pairs.sort_by(|a, b| b.priority.cmp(&a.priority));
}

/// Prune redundant candidate pairs per RFC 8445 Section 6.1.2.4.
///
/// Two pairs are considered redundant if they have the same local and remote
/// transport addresses. Only the higher-priority pair is kept.
pub fn prune_pairs(pairs: &mut Vec<IceCandidatePair>) {
    // Since pairs should already be sorted by priority (highest first),
    // we keep the first occurrence of each (local_addr, remote_addr) pair.
    let mut seen = std::collections::HashSet::new();
    pairs.retain(|pair| {
        let key = (pair.local.address, pair.remote.address);
        seen.insert(key)
    });
}

/// Initialize the check list by unfreezing the first pair per foundation.
///
/// Per RFC 8445 Section 6.1.2.6, for each foundation, the pair with the
/// lowest component ID is set to Waiting.
pub fn initialize_checklist(pairs: &mut [IceCandidatePair]) {
    let mut activated_foundations = std::collections::HashSet::new();

    // Pairs should be sorted by priority (highest first)
    for pair in pairs.iter_mut() {
        let foundation_key = format!("{}:{}", pair.local.foundation, pair.remote.foundation);
        if !activated_foundations.contains(&foundation_key) {
            pair.state = CandidatePairState::Waiting;
            activated_foundations.insert(foundation_key);
            trace!(
                local = %pair.local,
                remote = %pair.remote,
                "unfreezing pair for foundation"
            );
        }
    }
}

/// Build a STUN Binding Request for an ICE connectivity check.
///
/// Per RFC 8445 Section 7.2.4, the request includes:
/// - USERNAME: `{remote_ufrag}:{local_ufrag}`
/// - MESSAGE-INTEGRITY: HMAC-SHA1 with the remote password
/// - ICE-CONTROLLING or ICE-CONTROLLED with tie-breaker
/// - PRIORITY: the priority the controlling agent would assign
/// - USE-CANDIDATE: if the controlling agent is nominating
pub fn build_check_request(
    pair: &IceCandidatePair,
    local_credentials: &IceCredentials,
    remote_credentials: &IceCredentials,
    role: IceRole,
    tie_breaker: u64,
    nominate: bool,
) -> (Vec<u8>, crate::stun::message::TransactionId) {
    let mut msg = StunMessage {
        msg_type: BINDING_REQUEST,
        transaction_id: crate::stun::message::TransactionId::random(),
        attributes: Vec::new(),
    };

    // USERNAME = remote_ufrag:local_ufrag
    let username = format!("{}:{}", remote_credentials.ufrag, local_credentials.ufrag);
    msg.attributes.push(StunAttribute::Username(username));

    // ICE-CONTROLLING or ICE-CONTROLLED
    match role {
        IceRole::Controlling => {
            msg.attributes.push(StunAttribute::IceControlling(tie_breaker));
        }
        IceRole::Controlled => {
            msg.attributes.push(StunAttribute::IceControlled(tie_breaker));
        }
    }

    // PRIORITY: the priority the agent would assign to a peer-reflexive candidate
    // learned from this check
    let prflx_priority = super::gather::compute_priority(
        CandidateType::PeerReflexive,
        65535,
        pair.local.component,
    );
    msg.attributes.push(StunAttribute::Priority(prflx_priority));

    // USE-CANDIDATE (only for controlling agent when nominating)
    if role == IceRole::Controlling && nominate {
        msg.attributes.push(StunAttribute::UseCandidate);
    }

    let txn_id = msg.transaction_id;

    // Encode with MESSAGE-INTEGRITY using remote password
    let encoded = msg.encode_with_integrity(remote_credentials.pwd.as_bytes());

    debug!(
        local = %pair.local.address,
        remote = %pair.remote.address,
        role = %role,
        nominate = nominate,
        "built ICE connectivity check request"
    );

    (encoded, txn_id)
}

/// Build a STUN Binding Response for a received connectivity check.
///
/// Per RFC 8445 Section 7.3.1.4, the response includes:
/// - XOR-MAPPED-ADDRESS: the source address of the request
/// - MESSAGE-INTEGRITY: HMAC-SHA1 with the local password
pub fn build_check_response(
    request_txn_id: crate::stun::message::TransactionId,
    source_addr: &SocketAddr,
    local_credentials: &IceCredentials,
) -> Vec<u8> {
    use crate::stun::message::{
        BINDING_RESPONSE, ATTR_XOR_MAPPED_ADDRESS,
        encode_xor_address,
    };

    let mut msg = StunMessage {
        msg_type: BINDING_RESPONSE,
        transaction_id: request_txn_id,
        attributes: Vec::new(),
    };

    // We need to add XOR-MAPPED-ADDRESS manually since StunAttribute doesn't
    // encode response attributes. Build the raw attribute bytes.
    let xor_addr_value = encode_xor_address(source_addr, &request_txn_id.0);

    // Encode all attributes first, then add XOR-MAPPED-ADDRESS and integrity
    let mut attr_bytes = Vec::new();
    StunMessage::encode_attr(&mut attr_bytes, ATTR_XOR_MAPPED_ADDRESS, &xor_addr_value);

    // Build the message with integrity
    // We use a custom approach: build header + XOR-MAPPED-ADDRESS + MESSAGE-INTEGRITY
    use hmac::{Hmac, Mac};
    use sha1::Sha1;

    // MESSAGE-INTEGRITY is 24 bytes (4 header + 20 value)
    let mi_total = 24u16;
    let msg_len_for_mi = (attr_bytes.len() as u16) + mi_total;

    let mut hmac_input = Vec::with_capacity(20 + attr_bytes.len());
    hmac_input.extend_from_slice(&msg.msg_type.to_be_bytes());
    hmac_input.extend_from_slice(&msg_len_for_mi.to_be_bytes());
    hmac_input.extend_from_slice(&crate::stun::message::MAGIC_COOKIE.to_be_bytes());
    hmac_input.extend_from_slice(&request_txn_id.0);
    hmac_input.extend_from_slice(&attr_bytes);

    let key = local_credentials.pwd.as_bytes();
    let mac = <Hmac<Sha1>>::new_from_slice(key);
    let mut hmac_bytes = [0u8; 20];
    if let Ok(mut m) = mac {
        m.update(&hmac_input);
        let result = m.finalize().into_bytes();
        hmac_bytes.copy_from_slice(&result);
    }

    StunMessage::encode_attr(
        &mut attr_bytes,
        crate::stun::message::ATTR_MESSAGE_INTEGRITY,
        &hmac_bytes,
    );

    // Build final message
    let final_len = attr_bytes.len() as u16;
    let mut buf = Vec::with_capacity(20 + attr_bytes.len());
    buf.extend_from_slice(&msg.msg_type.to_be_bytes());
    buf.extend_from_slice(&final_len.to_be_bytes());
    buf.extend_from_slice(&crate::stun::message::MAGIC_COOKIE.to_be_bytes());
    buf.extend_from_slice(&request_txn_id.0);
    buf.extend_from_slice(&attr_bytes);

    buf
}

/// Find the next pair that is in the Waiting state.
pub fn next_waiting_pair(pairs: &[IceCandidatePair]) -> Option<usize> {
    pairs.iter().position(|p| p.state == CandidatePairState::Waiting)
}

/// Find a pair by its local and remote addresses.
pub fn find_pair_by_addresses(
    pairs: &[IceCandidatePair],
    local_addr: SocketAddr,
    remote_addr: SocketAddr,
) -> Option<usize> {
    pairs.iter().position(|p| {
        p.local.address == local_addr && p.remote.address == remote_addr
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_candidate(addr: &str, ctype: CandidateType, component: ComponentId) -> IceCandidate {
        let address: SocketAddr = addr.parse().unwrap_or_else(|e| panic!("parse: {e}"));
        IceCandidate {
            foundation: super::super::gather::generate_foundation(ctype, &address, None),
            component,
            transport: "udp".to_string(),
            priority: super::super::gather::compute_priority(ctype, 65535, component),
            address,
            candidate_type: ctype,
            related_address: None,
            ufrag: "test".to_string(),
        }
    }

    #[test]
    fn test_form_candidate_pairs() {
        let local = vec![
            make_candidate("192.168.1.1:5000", CandidateType::Host, ComponentId::Rtp),
        ];
        let remote = vec![
            make_candidate("10.0.0.1:6000", CandidateType::Host, ComponentId::Rtp),
            make_candidate("10.0.0.2:6000", CandidateType::Host, ComponentId::Rtp),
        ];

        let pairs = form_candidate_pairs(&local, &remote, IceRole::Controlling);
        assert_eq!(pairs.len(), 2, "should form 2 pairs (1 local x 2 remote)");

        for pair in &pairs {
            assert_eq!(pair.state, CandidatePairState::Frozen);
            assert!(!pair.nominated);
        }
    }

    #[test]
    fn test_form_pairs_different_components_not_paired() {
        let local = vec![
            make_candidate("192.168.1.1:5000", CandidateType::Host, ComponentId::Rtp),
        ];
        let remote = vec![
            make_candidate("10.0.0.1:6000", CandidateType::Host, ComponentId::Rtcp),
        ];

        let pairs = form_candidate_pairs(&local, &remote, IceRole::Controlling);
        assert_eq!(pairs.len(), 0, "different components should not be paired");
    }

    #[test]
    fn test_sort_pairs_by_priority() {
        let local1 = make_candidate("192.168.1.1:5000", CandidateType::Host, ComponentId::Rtp);
        let local2 = make_candidate("192.168.1.2:5000", CandidateType::ServerReflexive, ComponentId::Rtp);
        let remote = make_candidate("10.0.0.1:6000", CandidateType::Host, ComponentId::Rtp);

        let mut pairs = vec![
            IceCandidatePair {
                local: local2.clone(),
                remote: remote.clone(),
                state: CandidatePairState::Frozen,
                priority: IceCandidatePair::compute_priority(local2.priority, remote.priority),
                nominated: false,
            },
            IceCandidatePair {
                local: local1.clone(),
                remote: remote.clone(),
                state: CandidatePairState::Frozen,
                priority: IceCandidatePair::compute_priority(local1.priority, remote.priority),
                nominated: false,
            },
        ];

        sort_pairs(&mut pairs);

        // Host-host pair should be higher priority than srflx-host
        assert!(pairs[0].priority > pairs[1].priority);
        assert_eq!(pairs[0].local.candidate_type, CandidateType::Host);
    }

    #[test]
    fn test_prune_redundant_pairs() {
        let local = make_candidate("192.168.1.1:5000", CandidateType::Host, ComponentId::Rtp);
        let remote = make_candidate("10.0.0.1:6000", CandidateType::Host, ComponentId::Rtp);

        let mut pairs = vec![
            IceCandidatePair {
                local: local.clone(),
                remote: remote.clone(),
                state: CandidatePairState::Frozen,
                priority: 1000,
                nominated: false,
            },
            IceCandidatePair {
                local: local.clone(),
                remote: remote.clone(),
                state: CandidatePairState::Frozen,
                priority: 500,
                nominated: false,
            },
        ];

        // Sort first (higher priority first)
        sort_pairs(&mut pairs);
        prune_pairs(&mut pairs);

        assert_eq!(pairs.len(), 1, "duplicate pair should be pruned");
        assert_eq!(pairs[0].priority, 1000, "higher priority pair should be kept");
    }

    #[test]
    fn test_initialize_checklist() {
        let local = vec![
            make_candidate("192.168.1.1:5000", CandidateType::Host, ComponentId::Rtp),
            make_candidate("192.168.1.2:5000", CandidateType::Host, ComponentId::Rtp),
        ];
        let remote = vec![
            make_candidate("10.0.0.1:6000", CandidateType::Host, ComponentId::Rtp),
        ];

        let mut pairs = form_candidate_pairs(&local, &remote, IceRole::Controlling);
        sort_pairs(&mut pairs);
        initialize_checklist(&mut pairs);

        // At least one pair should be in Waiting state
        let waiting_count = pairs.iter().filter(|p| p.state == CandidatePairState::Waiting).count();
        assert!(waiting_count >= 1, "at least one pair should be unfrozen");
    }

    #[test]
    fn test_build_check_request() {
        let local_creds = IceCredentials {
            ufrag: "ABCD".to_string(),
            pwd: "password1234567890abcd".to_string(),
        };
        let remote_creds = IceCredentials {
            ufrag: "EFGH".to_string(),
            pwd: "remote_password_22chars".to_string(),
        };

        let local = make_candidate("192.168.1.1:5000", CandidateType::Host, ComponentId::Rtp);
        let remote = make_candidate("10.0.0.1:6000", CandidateType::Host, ComponentId::Rtp);

        let pair = IceCandidatePair {
            local,
            remote,
            state: CandidatePairState::Waiting,
            priority: 1000,
            nominated: false,
        };

        let (encoded, txn_id) = build_check_request(
            &pair,
            &local_creds,
            &remote_creds,
            IceRole::Controlling,
            12345,
            false,
        );

        // Should be a valid STUN message
        assert!(encoded.len() >= 20, "encoded message should be at least 20 bytes");

        // Decode and verify
        let decoded = StunMessage::decode(&encoded)
            .unwrap_or_else(|e| panic!("decode: {e}"));
        assert_eq!(decoded.msg_type, BINDING_REQUEST);
        assert_eq!(decoded.transaction_id, txn_id);

        // Should contain USERNAME
        let has_username = decoded.attributes.iter().any(|a| {
            matches!(a, StunAttribute::Username(u) if u == "EFGH:ABCD")
        });
        assert!(has_username, "should contain USERNAME attribute");

        // Should contain ICE-CONTROLLING
        let has_controlling = decoded.attributes.iter().any(|a| {
            matches!(a, StunAttribute::IceControlling(12345))
        });
        assert!(has_controlling, "should contain ICE-CONTROLLING attribute");

        // Should contain PRIORITY
        let has_priority = decoded.attributes.iter().any(|a| {
            matches!(a, StunAttribute::Priority(_))
        });
        assert!(has_priority, "should contain PRIORITY attribute");

        // Should NOT contain USE-CANDIDATE (nominate=false)
        let has_use_candidate = decoded.attributes.iter().any(|a| {
            matches!(a, StunAttribute::UseCandidate)
        });
        assert!(!has_use_candidate, "should NOT contain USE-CANDIDATE");
    }

    #[test]
    fn test_build_check_request_with_nomination() {
        let local_creds = IceCredentials {
            ufrag: "ABCD".to_string(),
            pwd: "password1234567890abcd".to_string(),
        };
        let remote_creds = IceCredentials {
            ufrag: "EFGH".to_string(),
            pwd: "remote_password_22chars".to_string(),
        };

        let local = make_candidate("192.168.1.1:5000", CandidateType::Host, ComponentId::Rtp);
        let remote = make_candidate("10.0.0.1:6000", CandidateType::Host, ComponentId::Rtp);

        let pair = IceCandidatePair {
            local,
            remote,
            state: CandidatePairState::Succeeded,
            priority: 1000,
            nominated: false,
        };

        let (encoded, _) = build_check_request(
            &pair,
            &local_creds,
            &remote_creds,
            IceRole::Controlling,
            99999,
            true, // nominate
        );

        let decoded = StunMessage::decode(&encoded)
            .unwrap_or_else(|e| panic!("decode: {e}"));

        let has_use_candidate = decoded.attributes.iter().any(|a| {
            matches!(a, StunAttribute::UseCandidate)
        });
        assert!(has_use_candidate, "should contain USE-CANDIDATE when nominating");
    }

    #[test]
    fn test_next_waiting_pair() {
        let local = make_candidate("192.168.1.1:5000", CandidateType::Host, ComponentId::Rtp);
        let remote = make_candidate("10.0.0.1:6000", CandidateType::Host, ComponentId::Rtp);

        let pairs = vec![
            IceCandidatePair {
                local: local.clone(),
                remote: remote.clone(),
                state: CandidatePairState::Frozen,
                priority: 1000,
                nominated: false,
            },
            IceCandidatePair {
                local: local.clone(),
                remote: remote.clone(),
                state: CandidatePairState::Waiting,
                priority: 500,
                nominated: false,
            },
        ];

        let idx = next_waiting_pair(&pairs);
        assert_eq!(idx, Some(1));
    }

    #[test]
    fn test_find_pair_by_addresses() {
        let local = make_candidate("192.168.1.1:5000", CandidateType::Host, ComponentId::Rtp);
        let remote = make_candidate("10.0.0.1:6000", CandidateType::Host, ComponentId::Rtp);

        let pairs = vec![IceCandidatePair {
            local: local.clone(),
            remote: remote.clone(),
            state: CandidatePairState::Frozen,
            priority: 1000,
            nominated: false,
        }];

        let found = find_pair_by_addresses(
            &pairs,
            "192.168.1.1:5000".parse().unwrap_or_else(|e| panic!("parse: {e}")),
            "10.0.0.1:6000".parse().unwrap_or_else(|e| panic!("parse: {e}")),
        );
        assert_eq!(found, Some(0));

        let not_found = find_pair_by_addresses(
            &pairs,
            "192.168.1.1:5000".parse().unwrap_or_else(|e| panic!("parse: {e}")),
            "10.0.0.2:6000".parse().unwrap_or_else(|e| panic!("parse: {e}")),
        );
        assert_eq!(not_found, None);
    }
}
