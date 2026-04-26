//! RFC 8489 STUN message codec — Binding Request encode + Binding
//! Response decode for the `MAPPED-ADDRESS` / `XOR-MAPPED-ADDRESS`
//! attributes only.
//!
//! Wire format (§5):
//!
//! ```text
//!  0                   1                   2                   3
//!  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |0 0|     STUN Message Type     |         Message Length        |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |                         Magic Cookie                          |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |                                                               |
//! |                     Transaction ID (96 bits)                  |
//! |                                                               |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |                       Attributes (TLV)                        |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! ```

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use rand::RngCore;

use super::StunError;

/// RFC 8489 §6 fixed magic cookie.
pub const MAGIC_COOKIE: u32 = 0x2112_A442;

/// Binding Request type code (RFC 8489 §6 + IANA registry).
const BINDING_REQUEST: u16 = 0x0001;
/// Binding Response (success) type code.
const BINDING_RESPONSE: u16 = 0x0101;

/// `MAPPED-ADDRESS` attribute type (RFC 8489 §14.1, comprehension-required).
const ATTR_MAPPED_ADDRESS: u16 = 0x0001;
/// `XOR-MAPPED-ADDRESS` attribute type (RFC 8489 §14.2).
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;

const FAMILY_IPV4: u8 = 0x01;
const FAMILY_IPV6: u8 = 0x02;

const HEADER_LEN: usize = 20;
const TXN_ID_LEN: usize = 12;

/// Encode a Binding Request with a fresh 96-bit transaction id.
/// Returns `(wire_bytes, transaction_id)`.
pub fn encode_binding_request() -> (Vec<u8>, [u8; TXN_ID_LEN]) {
    let mut txn_id = [0u8; TXN_ID_LEN];
    rand::thread_rng().fill_bytes(&mut txn_id);

    let mut buf = Vec::with_capacity(HEADER_LEN);
    buf.extend_from_slice(&BINDING_REQUEST.to_be_bytes());
    buf.extend_from_slice(&0u16.to_be_bytes()); // length: no attributes
    buf.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
    buf.extend_from_slice(&txn_id);

    (buf, txn_id)
}

/// Decode a Binding Response and return the discovered public mapping.
///
/// Validates:
/// - Header length, magic cookie, transaction id match.
/// - Message type is exactly `Binding Response (success)` (0x0101).
/// - Body is well-formed TLVs and contains a `MAPPED-ADDRESS` or
///   `XOR-MAPPED-ADDRESS` attribute.
///
/// Unknown comprehension-optional attributes (high bit set on type)
/// are skipped silently per RFC 8489 §14. Unknown
/// comprehension-required attributes are also skipped here — the UAC
/// is best-effort and the discovered address is what matters; any
/// truly broken response will simply lack the mapped-address attr and
/// surface as `NoMappedAddress`.
pub fn decode_binding_response(
    bytes: &[u8],
    expected_txn_id: &[u8; TXN_ID_LEN],
) -> Result<SocketAddr, StunError> {
    if bytes.len() < HEADER_LEN {
        return Err(StunError::TooShort { got: bytes.len() });
    }

    let msg_type = u16::from_be_bytes([bytes[0], bytes[1]]);
    let msg_len = u16::from_be_bytes([bytes[2], bytes[3]]) as usize;
    let cookie = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let txn_id = &bytes[8..20];

    if cookie != MAGIC_COOKIE {
        return Err(StunError::MagicCookieMismatch {
            got: cookie,
            expected: MAGIC_COOKIE,
        });
    }
    if txn_id != expected_txn_id {
        return Err(StunError::TransactionIdMismatch);
    }
    if msg_type != BINDING_RESPONSE {
        return Err(StunError::NotBindingResponse(msg_type));
    }
    if HEADER_LEN + msg_len > bytes.len() {
        return Err(StunError::AttributeTruncated {
            got: bytes.len(),
            need: HEADER_LEN + msg_len,
        });
    }

    // Walk attributes. Each is a 4-byte header (type, length) followed
    // by `length` body bytes, padded to 4-byte alignment.
    let mut cursor = HEADER_LEN;
    let body_end = HEADER_LEN + msg_len;
    let mut found_xor: Option<SocketAddr> = None;
    let mut found_plain: Option<SocketAddr> = None;

    while cursor + 4 <= body_end {
        let attr_type = u16::from_be_bytes([bytes[cursor], bytes[cursor + 1]]);
        let attr_len =
            u16::from_be_bytes([bytes[cursor + 2], bytes[cursor + 3]]) as usize;
        let body_start = cursor + 4;
        let body_end_attr = body_start + attr_len;
        if body_end_attr > body_end {
            return Err(StunError::AttributeTruncated {
                got: body_end - body_start,
                need: attr_len,
            });
        }
        let body = &bytes[body_start..body_end_attr];

        match attr_type {
            ATTR_XOR_MAPPED_ADDRESS => {
                found_xor = Some(decode_xor_mapped_address(body, expected_txn_id)?);
            }
            ATTR_MAPPED_ADDRESS => {
                found_plain = Some(decode_mapped_address(body)?);
            }
            _ => {
                // Skip comprehension-optional and any other type.
                tracing::trace!("STUN: skipping attribute type 0x{:04x}", attr_type);
            }
        }

        // Advance past body + 4-byte alignment padding.
        let padded_len = (attr_len + 3) & !3;
        cursor = body_start + padded_len;
    }

    // Prefer XOR-MAPPED-ADDRESS (RFC 8489 §14.2: any STUN-aware NAT
    // would mangle a plain MAPPED-ADDRESS in the body). Fall back to
    // plain MAPPED-ADDRESS for legacy RFC 5389 servers that emit only
    // it.
    found_xor
        .or(found_plain)
        .ok_or(StunError::NoMappedAddress)
}

fn decode_mapped_address(body: &[u8]) -> Result<SocketAddr, StunError> {
    if body.len() < 4 {
        return Err(StunError::AttributeTruncated {
            got: body.len(),
            need: 4,
        });
    }
    // body[0] is reserved (0x00), body[1] is family, body[2..4] is port.
    let family = body[1];
    let port = u16::from_be_bytes([body[2], body[3]]);
    match family {
        FAMILY_IPV4 => {
            if body.len() < 8 {
                return Err(StunError::AttributeTruncated {
                    got: body.len(),
                    need: 8,
                });
            }
            let ip = Ipv4Addr::new(body[4], body[5], body[6], body[7]);
            Ok(SocketAddr::new(IpAddr::V4(ip), port))
        }
        FAMILY_IPV6 => {
            if body.len() < 20 {
                return Err(StunError::AttributeTruncated {
                    got: body.len(),
                    need: 20,
                });
            }
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&body[4..20]);
            Ok(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(octets)), port))
        }
        other => Err(StunError::UnknownAddressFamily(other)),
    }
}

fn decode_xor_mapped_address(
    body: &[u8],
    txn_id: &[u8; TXN_ID_LEN],
) -> Result<SocketAddr, StunError> {
    if body.len() < 4 {
        return Err(StunError::AttributeTruncated {
            got: body.len(),
            need: 4,
        });
    }
    let family = body[1];
    // Port is XOR'd with the high 16 bits of the magic cookie (RFC
    // 8489 §14.2).
    let xport = u16::from_be_bytes([body[2], body[3]]);
    let port = xport ^ ((MAGIC_COOKIE >> 16) as u16);

    match family {
        FAMILY_IPV4 => {
            if body.len() < 8 {
                return Err(StunError::AttributeTruncated {
                    got: body.len(),
                    need: 8,
                });
            }
            // IPv4 address is XOR'd with the magic cookie.
            let xa = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
            let a = xa ^ MAGIC_COOKIE;
            let octets = a.to_be_bytes();
            Ok(SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3])),
                port,
            ))
        }
        FAMILY_IPV6 => {
            if body.len() < 20 {
                return Err(StunError::AttributeTruncated {
                    got: body.len(),
                    need: 20,
                });
            }
            // IPv6 address is XOR'd with magic cookie || transaction id.
            let mut mask = [0u8; 16];
            mask[0..4].copy_from_slice(&MAGIC_COOKIE.to_be_bytes());
            mask[4..16].copy_from_slice(txn_id);
            let mut octets = [0u8; 16];
            for i in 0..16 {
                octets[i] = body[4 + i] ^ mask[i];
            }
            Ok(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(octets)), port))
        }
        other => Err(StunError::UnknownAddressFamily(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_request_has_correct_header_shape() {
        let (bytes, txn) = encode_binding_request();
        assert_eq!(bytes.len(), 20, "request with no attributes is exactly the header");
        assert_eq!(u16::from_be_bytes([bytes[0], bytes[1]]), BINDING_REQUEST);
        assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), 0, "no attributes");
        assert_eq!(
            u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            MAGIC_COOKIE
        );
        assert_eq!(&bytes[8..20], &txn[..]);
    }

    #[test]
    fn binding_request_uses_distinct_transaction_ids() {
        let (_, t1) = encode_binding_request();
        let (_, t2) = encode_binding_request();
        assert_ne!(t1, t2, "transaction IDs must be random per request");
    }

    /// Build a synthetic Binding Response with a single
    /// `XOR-MAPPED-ADDRESS` attribute pointing at the supplied
    /// address, for round-trip testing.
    fn craft_binding_response(addr: SocketAddr, txn: &[u8; TXN_ID_LEN]) -> Vec<u8> {
        let (family, addr_bytes): (u8, Vec<u8>) = match addr.ip() {
            IpAddr::V4(v4) => {
                let xa = u32::from_be_bytes(v4.octets()) ^ MAGIC_COOKIE;
                (FAMILY_IPV4, xa.to_be_bytes().to_vec())
            }
            IpAddr::V6(v6) => {
                let mut mask = [0u8; 16];
                mask[0..4].copy_from_slice(&MAGIC_COOKIE.to_be_bytes());
                mask[4..16].copy_from_slice(txn);
                let mut xb = v6.octets();
                for i in 0..16 {
                    xb[i] ^= mask[i];
                }
                (FAMILY_IPV6, xb.to_vec())
            }
        };

        let xport = addr.port() ^ ((MAGIC_COOKIE >> 16) as u16);

        // Attribute body: reserved(0) | family | xport(2) | xaddr
        let mut attr_body = Vec::new();
        attr_body.push(0);
        attr_body.push(family);
        attr_body.extend_from_slice(&xport.to_be_bytes());
        attr_body.extend_from_slice(&addr_bytes);

        let attr_len = attr_body.len() as u16;
        let mut body = Vec::new();
        body.extend_from_slice(&ATTR_XOR_MAPPED_ADDRESS.to_be_bytes());
        body.extend_from_slice(&attr_len.to_be_bytes());
        body.extend_from_slice(&attr_body);
        // Pad to 4-byte boundary.
        while body.len() % 4 != 0 {
            body.push(0);
        }

        let msg_len = body.len() as u16;
        let mut msg = Vec::new();
        msg.extend_from_slice(&BINDING_RESPONSE.to_be_bytes());
        msg.extend_from_slice(&msg_len.to_be_bytes());
        msg.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
        msg.extend_from_slice(txn);
        msg.extend_from_slice(&body);
        msg
    }

    #[test]
    fn xor_mapped_address_ipv4_round_trip() {
        let txn = [0x55u8; TXN_ID_LEN];
        let addr: SocketAddr = "203.0.113.42:30000".parse().unwrap();
        let bytes = craft_binding_response(addr, &txn);
        let decoded = decode_binding_response(&bytes, &txn).unwrap();
        assert_eq!(decoded, addr);
    }

    #[test]
    fn xor_mapped_address_ipv6_round_trip() {
        let txn = [0xAAu8; TXN_ID_LEN];
        let addr: SocketAddr = "[2001:db8::1]:31337".parse().unwrap();
        let bytes = craft_binding_response(addr, &txn);
        let decoded = decode_binding_response(&bytes, &txn).unwrap();
        assert_eq!(decoded, addr);
    }

    #[test]
    fn transaction_id_mismatch_rejected() {
        let req_txn = [0x11u8; TXN_ID_LEN];
        let other_txn = [0x22u8; TXN_ID_LEN];
        let addr: SocketAddr = "1.2.3.4:5678".parse().unwrap();
        let bytes = craft_binding_response(addr, &other_txn);
        assert!(matches!(
            decode_binding_response(&bytes, &req_txn),
            Err(StunError::TransactionIdMismatch)
        ));
    }

    #[test]
    fn magic_cookie_mismatch_rejected() {
        let txn = [0x33u8; TXN_ID_LEN];
        let addr: SocketAddr = "1.2.3.4:5678".parse().unwrap();
        let mut bytes = craft_binding_response(addr, &txn);
        // Corrupt the cookie.
        bytes[4] ^= 0xff;
        let err = decode_binding_response(&bytes, &txn).unwrap_err();
        assert!(matches!(err, StunError::MagicCookieMismatch { .. }));
    }

    #[test]
    fn unknown_attribute_skipped_when_xor_mapped_present() {
        let txn = [0x44u8; TXN_ID_LEN];
        let addr: SocketAddr = "198.51.100.7:9999".parse().unwrap();
        let mut bytes = craft_binding_response(addr, &txn);

        // Append an unknown comprehension-optional attribute (type
        // 0xC000 — high bit set means optional). Length 4 with 4
        // bytes of body for a clean 4-byte alignment.
        let unknown_attr_type: u16 = 0xC000;
        let body_len: u16 = 4;
        let extra_len = 4 + body_len as usize;
        let new_msg_len = u16::from_be_bytes([bytes[2], bytes[3]]) + extra_len as u16;
        bytes[2..4].copy_from_slice(&new_msg_len.to_be_bytes());
        bytes.extend_from_slice(&unknown_attr_type.to_be_bytes());
        bytes.extend_from_slice(&body_len.to_be_bytes());
        bytes.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);

        let decoded = decode_binding_response(&bytes, &txn).unwrap();
        assert_eq!(decoded, addr, "decoder must ignore unknown attrs");
    }

    #[test]
    fn truncated_response_returns_too_short() {
        let txn = [0u8; TXN_ID_LEN];
        let bytes = vec![0x01, 0x01]; // 2 bytes — way below 20-byte header.
        let err = decode_binding_response(&bytes, &txn).unwrap_err();
        assert!(matches!(err, StunError::TooShort { got: 2 }));
    }

    #[test]
    fn response_with_no_address_attribute_errors() {
        let txn = [0x66u8; TXN_ID_LEN];
        let mut msg = Vec::new();
        msg.extend_from_slice(&BINDING_RESPONSE.to_be_bytes());
        msg.extend_from_slice(&0u16.to_be_bytes());
        msg.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
        msg.extend_from_slice(&txn);
        let err = decode_binding_response(&msg, &txn).unwrap_err();
        assert!(matches!(err, StunError::NoMappedAddress));
    }

    #[test]
    fn legacy_mapped_address_fallback_decodes_when_no_xor() {
        let txn = [0x77u8; TXN_ID_LEN];
        let addr: SocketAddr = "192.0.2.1:443".parse().unwrap();

        // Build a Binding Response with only the plain MAPPED-ADDRESS
        // attribute (no XOR variant). RFC 5389 servers may emit this.
        let mut attr_body = Vec::new();
        attr_body.push(0);
        attr_body.push(FAMILY_IPV4);
        attr_body.extend_from_slice(&addr.port().to_be_bytes());
        match addr.ip() {
            IpAddr::V4(v4) => attr_body.extend_from_slice(&v4.octets()),
            _ => unreachable!(),
        }
        let attr_len = attr_body.len() as u16;
        let mut body = Vec::new();
        body.extend_from_slice(&ATTR_MAPPED_ADDRESS.to_be_bytes());
        body.extend_from_slice(&attr_len.to_be_bytes());
        body.extend_from_slice(&attr_body);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        let msg_len = body.len() as u16;
        let mut msg = Vec::new();
        msg.extend_from_slice(&BINDING_RESPONSE.to_be_bytes());
        msg.extend_from_slice(&msg_len.to_be_bytes());
        msg.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
        msg.extend_from_slice(&txn);
        msg.extend_from_slice(&body);

        let decoded = decode_binding_response(&msg, &txn).unwrap();
        assert_eq!(decoded, addr);
    }
}
