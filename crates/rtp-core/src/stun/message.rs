//! RFC 5389 STUN message encoding and decoding.
//!
//! STUN message format (RFC 5389 Section 6):
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
//! ```

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use crate::Error;

/// STUN magic cookie value (RFC 5389 Section 6).
pub const MAGIC_COOKIE: u32 = 0x2112_A442;

/// STUN header size in bytes (20 bytes).
pub const HEADER_SIZE: usize = 20;

/// Minimum attribute size (type u16 + length u16 = 4 bytes).
pub const ATTR_HEADER_SIZE: usize = 4;

// -- Message types --

/// Binding Request (RFC 5389 Section 18.1).
pub const BINDING_REQUEST: u16 = 0x0001;

/// Binding Success Response.
pub const BINDING_RESPONSE: u16 = 0x0101;

/// Binding Error Response.
pub const BINDING_ERROR_RESPONSE: u16 = 0x0111;

// -- Attribute types --

/// MAPPED-ADDRESS attribute (RFC 5389 Section 15.1).
pub const ATTR_MAPPED_ADDRESS: u16 = 0x0001;

/// XOR-MAPPED-ADDRESS attribute (RFC 5389 Section 15.2).
pub const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;

/// ERROR-CODE attribute (RFC 5389 Section 15.6).
pub const ATTR_ERROR_CODE: u16 = 0x0009;

/// SOFTWARE attribute (RFC 5389 Section 15.10).
pub const ATTR_SOFTWARE: u16 = 0x8022;

/// USERNAME attribute (RFC 5389 Section 15.3).
pub const ATTR_USERNAME: u16 = 0x0006;

/// MESSAGE-INTEGRITY attribute (RFC 5389 Section 15.4).
pub const ATTR_MESSAGE_INTEGRITY: u16 = 0x0008;

/// FINGERPRINT attribute (RFC 5389 Section 15.5).
pub const ATTR_FINGERPRINT: u16 = 0x8028;

/// PRIORITY attribute (RFC 8445 Section 7.1.1).
pub const ATTR_PRIORITY: u16 = 0x0024;

/// USE-CANDIDATE attribute (RFC 8445 Section 7.1.1).
pub const ATTR_USE_CANDIDATE: u16 = 0x0025;

/// ICE-CONTROLLED attribute (RFC 8445 Section 7.1.1).
pub const ATTR_ICE_CONTROLLED: u16 = 0x8029;

/// ICE-CONTROLLING attribute (RFC 8445 Section 7.1.1).
pub const ATTR_ICE_CONTROLLING: u16 = 0x802A;

/// REALM attribute (RFC 5389 Section 15.7).
pub const ATTR_REALM: u16 = 0x0014;

/// NONCE attribute (RFC 5389 Section 15.8).
pub const ATTR_NONCE: u16 = 0x0015;

// -- Address family --

/// IPv4 address family for STUN/TURN address attributes.
pub const ADDR_FAMILY_IPV4: u8 = 0x01;
/// IPv6 address family for STUN/TURN address attributes.
pub const ADDR_FAMILY_IPV6: u8 = 0x02;

/// A 96-bit STUN transaction ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransactionId(pub [u8; 12]);

impl TransactionId {
    /// Generate a cryptographically random transaction ID.
    pub fn random() -> Self {
        let mut id = [0u8; 12];
        rand::Rng::fill(&mut rand::thread_rng(), &mut id);
        Self(id)
    }
}

/// A parsed STUN attribute.
#[derive(Debug, Clone)]
pub enum StunAttribute {
    /// MAPPED-ADDRESS: the reflexive transport address.
    MappedAddress(SocketAddr),
    /// XOR-MAPPED-ADDRESS: XOR-obfuscated reflexive transport address.
    XorMappedAddress(SocketAddr),
    /// ERROR-CODE: error class/number + reason phrase.
    ErrorCode {
        code: u16,
        reason: String,
    },
    /// SOFTWARE: server software description.
    Software(String),
    /// USERNAME attribute: authentication username.
    Username(String),
    /// MESSAGE-INTEGRITY attribute: HMAC-SHA1 over the message.
    MessageIntegrity([u8; 20]),
    /// PRIORITY attribute: candidate priority (ICE).
    Priority(u32),
    /// USE-CANDIDATE attribute: nominate this pair (ICE).
    UseCandidate,
    /// ICE-CONTROLLING attribute: tie-breaker value.
    IceControlling(u64),
    /// ICE-CONTROLLED attribute: tie-breaker value.
    IceControlled(u64),
    /// FINGERPRINT attribute: CRC-32 XOR'd with 0x5354554E.
    Fingerprint(u32),
    /// Unknown/unrecognized attribute (type, raw value).
    Unknown(u16, Vec<u8>),
}

/// A STUN message (request or response).
#[derive(Debug, Clone)]
pub struct StunMessage {
    /// Message type (e.g. BINDING_REQUEST, BINDING_RESPONSE).
    pub msg_type: u16,
    /// Transaction ID.
    pub transaction_id: TransactionId,
    /// Parsed attributes.
    pub attributes: Vec<StunAttribute>,
}

impl StunMessage {
    /// Create a new Binding Request with a random transaction ID.
    pub fn binding_request() -> Self {
        Self {
            msg_type: BINDING_REQUEST,
            transaction_id: TransactionId::random(),
            attributes: Vec::new(),
        }
    }

    /// Encode this message into a byte buffer ready for transmission.
    pub fn encode(&self) -> Vec<u8> {
        let attr_bytes = self.encode_attributes();
        let msg_len = attr_bytes.len() as u16;

        let mut buf = Vec::with_capacity(HEADER_SIZE + attr_bytes.len());

        // Message type (first 2 bits must be 0)
        buf.extend_from_slice(&self.msg_type.to_be_bytes());
        // Message length (excluding 20-byte header)
        buf.extend_from_slice(&msg_len.to_be_bytes());
        // Magic cookie
        buf.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
        // Transaction ID
        buf.extend_from_slice(&self.transaction_id.0);
        // Attributes
        buf.extend_from_slice(&attr_bytes);

        buf
    }

    /// Decode a STUN message from raw bytes.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        if data.len() < HEADER_SIZE {
            return Err(Error::StunError(format!(
                "message too short: {} bytes, need at least {}",
                data.len(),
                HEADER_SIZE
            )));
        }

        // First two bits must be 0 (RFC 5389 Section 6)
        if data[0] & 0xC0 != 0 {
            return Err(Error::StunError(
                "first two bits of message type must be 0".into(),
            ));
        }

        let msg_type = u16::from_be_bytes([data[0], data[1]]);
        let msg_len = u16::from_be_bytes([data[2], data[3]]) as usize;
        let cookie = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);

        if cookie != MAGIC_COOKIE {
            return Err(Error::StunError(format!(
                "invalid magic cookie: 0x{:08X}, expected 0x{:08X}",
                cookie, MAGIC_COOKIE
            )));
        }

        if data.len() < HEADER_SIZE + msg_len {
            return Err(Error::StunError(format!(
                "message truncated: have {} bytes but header says {} + {}",
                data.len(),
                HEADER_SIZE,
                msg_len
            )));
        }

        let mut txn_id = [0u8; 12];
        txn_id.copy_from_slice(&data[8..20]);

        let attributes = Self::decode_attributes(&data[HEADER_SIZE..HEADER_SIZE + msg_len], &txn_id)?;

        Ok(Self {
            msg_type,
            transaction_id: TransactionId(txn_id),
            attributes,
        })
    }

    /// Find the first XOR-MAPPED-ADDRESS, falling back to MAPPED-ADDRESS.
    pub fn mapped_address(&self) -> Option<SocketAddr> {
        // Prefer XOR-MAPPED-ADDRESS per RFC 5389
        for attr in &self.attributes {
            if let StunAttribute::XorMappedAddress(addr) = attr {
                return Some(*addr);
            }
        }
        for attr in &self.attributes {
            if let StunAttribute::MappedAddress(addr) = attr {
                return Some(*addr);
            }
        }
        None
    }

    /// Extract the error code if this is an error response.
    pub fn error_code(&self) -> Option<(u16, &str)> {
        for attr in &self.attributes {
            if let StunAttribute::ErrorCode { code, reason } = attr {
                return Some((*code, reason.as_str()));
            }
        }
        None
    }

    /// Encode this message with MESSAGE-INTEGRITY computed using the given key.
    ///
    /// Per RFC 5389 Section 15.4, MESSAGE-INTEGRITY is an HMAC-SHA1 over
    /// the STUN message up to (and including) the attribute header of the
    /// MESSAGE-INTEGRITY attribute itself, with the message length field
    /// adjusted to point to the end of the MESSAGE-INTEGRITY attribute.
    pub fn encode_with_integrity(&self, key: &[u8]) -> Vec<u8> {
        use hmac::{Hmac, Mac};
        use sha1::Sha1;

        // First encode all attributes *except* MessageIntegrity and Fingerprint
        let mut attr_bytes = Vec::new();
        for attr in &self.attributes {
            match attr {
                StunAttribute::MessageIntegrity(_) | StunAttribute::Fingerprint(_) => {}
                _ => {
                    self.encode_single_attr(&mut attr_bytes, attr);
                }
            }
        }

        // MESSAGE-INTEGRITY: 4 byte header + 20 byte value = 24 bytes total
        let mi_total = 24u16;
        let msg_len_for_mi = (attr_bytes.len() as u16) + mi_total;

        // Build pseudo-message for HMAC computation
        let mut hmac_input = Vec::with_capacity(HEADER_SIZE + attr_bytes.len());
        hmac_input.extend_from_slice(&self.msg_type.to_be_bytes());
        hmac_input.extend_from_slice(&msg_len_for_mi.to_be_bytes());
        hmac_input.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
        hmac_input.extend_from_slice(&self.transaction_id.0);
        hmac_input.extend_from_slice(&attr_bytes);

        let mac = <Hmac<Sha1>>::new_from_slice(key);
        let mut mac = match mac {
            Ok(m) => m,
            Err(_) => return self.encode(),
        };
        mac.update(&hmac_input);
        let hmac_result = mac.finalize().into_bytes();
        let mut hmac_bytes = [0u8; 20];
        hmac_bytes.copy_from_slice(&hmac_result);

        // Append the MESSAGE-INTEGRITY attribute
        Self::encode_attr(&mut attr_bytes, ATTR_MESSAGE_INTEGRITY, &hmac_bytes);

        // Build the final message
        let final_msg_len = attr_bytes.len() as u16;
        let mut buf = Vec::with_capacity(HEADER_SIZE + attr_bytes.len());
        buf.extend_from_slice(&self.msg_type.to_be_bytes());
        buf.extend_from_slice(&final_msg_len.to_be_bytes());
        buf.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
        buf.extend_from_slice(&self.transaction_id.0);
        buf.extend_from_slice(&attr_bytes);

        buf
    }

    /// Verify the MESSAGE-INTEGRITY attribute against the given key.
    ///
    /// Returns `true` if the HMAC matches, `false` if it does not match
    /// or if no MESSAGE-INTEGRITY attribute is present.
    pub fn verify_integrity(&self, raw_message: &[u8], key: &[u8]) -> bool {
        use hmac::{Hmac, Mac};
        use sha1::Sha1;

        let expected_hmac = match self.attributes.iter().find_map(|a| {
            if let StunAttribute::MessageIntegrity(h) = a { Some(*h) } else { None }
        }) {
            Some(h) => h,
            None => return false,
        };

        // Find the offset of MESSAGE-INTEGRITY in the raw message
        let mut mi_offset = None;
        let mut offset = HEADER_SIZE;
        while offset + ATTR_HEADER_SIZE <= raw_message.len() {
            let at = u16::from_be_bytes([raw_message[offset], raw_message[offset + 1]]);
            let al = u16::from_be_bytes([raw_message[offset + 2], raw_message[offset + 3]]) as usize;
            if at == ATTR_MESSAGE_INTEGRITY {
                mi_offset = Some(offset);
                break;
            }
            let padded = al + ((4 - (al % 4)) % 4);
            offset += ATTR_HEADER_SIZE + padded;
        }

        let mi_offset = match mi_offset {
            Some(o) => o,
            None => return false,
        };

        // Adjusted length: everything up to end of MESSAGE-INTEGRITY value
        let adjusted_len = (mi_offset - HEADER_SIZE + ATTR_HEADER_SIZE + 20) as u16;

        let mut hmac_input = Vec::with_capacity(mi_offset);
        hmac_input.extend_from_slice(&raw_message[0..2]);
        hmac_input.extend_from_slice(&adjusted_len.to_be_bytes());
        hmac_input.extend_from_slice(&raw_message[4..HEADER_SIZE]);
        hmac_input.extend_from_slice(&raw_message[HEADER_SIZE..mi_offset]);

        let mac = match <Hmac<Sha1>>::new_from_slice(key) {
            Ok(m) => m,
            Err(_) => return false,
        };
        let mut mac = mac;
        mac.update(&hmac_input);

        mac.verify_slice(&expected_hmac).is_ok()
    }

    /// Helper to encode a single attribute into a buffer.
    fn encode_single_attr(&self, buf: &mut Vec<u8>, attr: &StunAttribute) {
        match attr {
            StunAttribute::Software(s) => Self::encode_attr(buf, ATTR_SOFTWARE, s.as_bytes()),
            StunAttribute::Username(s) => Self::encode_attr(buf, ATTR_USERNAME, s.as_bytes()),
            StunAttribute::Priority(p) => Self::encode_attr(buf, ATTR_PRIORITY, &p.to_be_bytes()),
            StunAttribute::UseCandidate => Self::encode_attr(buf, ATTR_USE_CANDIDATE, &[]),
            StunAttribute::IceControlling(v) => Self::encode_attr(buf, ATTR_ICE_CONTROLLING, &v.to_be_bytes()),
            StunAttribute::IceControlled(v) => Self::encode_attr(buf, ATTR_ICE_CONTROLLED, &v.to_be_bytes()),
            StunAttribute::MessageIntegrity(h) => Self::encode_attr(buf, ATTR_MESSAGE_INTEGRITY, h),
            StunAttribute::Fingerprint(c) => Self::encode_attr(buf, ATTR_FINGERPRINT, &c.to_be_bytes()),
            _ => {} // Response-only attributes not encoded by the client
        }
    }

    // -- Private helpers --

    fn encode_attributes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        for attr in &self.attributes {
            match attr {
                StunAttribute::Software(s) => {
                    Self::encode_attr(&mut buf, ATTR_SOFTWARE, s.as_bytes());
                }
                StunAttribute::Username(s) => {
                    Self::encode_attr(&mut buf, ATTR_USERNAME, s.as_bytes());
                }
                StunAttribute::Priority(p) => {
                    Self::encode_attr(&mut buf, ATTR_PRIORITY, &p.to_be_bytes());
                }
                StunAttribute::UseCandidate => {
                    Self::encode_attr(&mut buf, ATTR_USE_CANDIDATE, &[]);
                }
                StunAttribute::IceControlling(val) => {
                    Self::encode_attr(&mut buf, ATTR_ICE_CONTROLLING, &val.to_be_bytes());
                }
                StunAttribute::IceControlled(val) => {
                    Self::encode_attr(&mut buf, ATTR_ICE_CONTROLLED, &val.to_be_bytes());
                }
                StunAttribute::MessageIntegrity(hmac) => {
                    Self::encode_attr(&mut buf, ATTR_MESSAGE_INTEGRITY, hmac);
                }
                StunAttribute::Fingerprint(crc) => {
                    Self::encode_attr(&mut buf, ATTR_FINGERPRINT, &crc.to_be_bytes());
                }
                // Response-only attributes: MappedAddress, XorMappedAddress, ErrorCode
                _ => {}
            }
        }
        buf
    }

    /// Encode a single STUN/TURN attribute (type + length + value + padding).
    pub fn encode_attr(buf: &mut Vec<u8>, attr_type: u16, value: &[u8]) {
        buf.extend_from_slice(&attr_type.to_be_bytes());
        buf.extend_from_slice(&(value.len() as u16).to_be_bytes());
        buf.extend_from_slice(value);
        // Pad to 4-byte boundary (RFC 5389 Section 15)
        let padding = (4 - (value.len() % 4)) % 4;
        buf.extend(std::iter::repeat(0u8).take(padding));
    }

    fn decode_attributes(data: &[u8], txn_id: &[u8; 12]) -> Result<Vec<StunAttribute>, Error> {
        let mut attrs = Vec::new();
        let mut offset = 0;

        while offset + ATTR_HEADER_SIZE <= data.len() {
            let attr_type = u16::from_be_bytes([data[offset], data[offset + 1]]);
            let attr_len = u16::from_be_bytes([data[offset + 2], data[offset + 3]]) as usize;
            offset += ATTR_HEADER_SIZE;

            if offset + attr_len > data.len() {
                return Err(Error::StunError(format!(
                    "attribute 0x{:04X} truncated: need {} bytes at offset {} but only {} remain",
                    attr_type,
                    attr_len,
                    offset,
                    data.len() - offset
                )));
            }

            let value = &data[offset..offset + attr_len];

            let attr = match attr_type {
                ATTR_MAPPED_ADDRESS => Self::decode_mapped_address(value)?,
                ATTR_XOR_MAPPED_ADDRESS => Self::decode_xor_mapped_address(value, txn_id)?,
                ATTR_ERROR_CODE => Self::decode_error_code(value)?,
                ATTR_SOFTWARE => Self::decode_software(value),
                ATTR_USERNAME => StunAttribute::Username(String::from_utf8_lossy(value).into_owned()),
                ATTR_MESSAGE_INTEGRITY => {
                    if value.len() >= 20 {
                        let mut hmac = [0u8; 20];
                        hmac.copy_from_slice(&value[..20]);
                        StunAttribute::MessageIntegrity(hmac)
                    } else {
                        StunAttribute::Unknown(attr_type, value.to_vec())
                    }
                }
                ATTR_PRIORITY => {
                    if value.len() >= 4 {
                        let p = u32::from_be_bytes([value[0], value[1], value[2], value[3]]);
                        StunAttribute::Priority(p)
                    } else {
                        StunAttribute::Unknown(attr_type, value.to_vec())
                    }
                }
                ATTR_USE_CANDIDATE => StunAttribute::UseCandidate,
                ATTR_ICE_CONTROLLING => {
                    if value.len() >= 8 {
                        let v = u64::from_be_bytes([
                            value[0], value[1], value[2], value[3],
                            value[4], value[5], value[6], value[7],
                        ]);
                        StunAttribute::IceControlling(v)
                    } else {
                        StunAttribute::Unknown(attr_type, value.to_vec())
                    }
                }
                ATTR_ICE_CONTROLLED => {
                    if value.len() >= 8 {
                        let v = u64::from_be_bytes([
                            value[0], value[1], value[2], value[3],
                            value[4], value[5], value[6], value[7],
                        ]);
                        StunAttribute::IceControlled(v)
                    } else {
                        StunAttribute::Unknown(attr_type, value.to_vec())
                    }
                }
                ATTR_FINGERPRINT => {
                    if value.len() >= 4 {
                        let crc = u32::from_be_bytes([value[0], value[1], value[2], value[3]]);
                        StunAttribute::Fingerprint(crc)
                    } else {
                        StunAttribute::Unknown(attr_type, value.to_vec())
                    }
                }
                other => StunAttribute::Unknown(other, value.to_vec()),
            };

            attrs.push(attr);

            // Advance past value + padding to 4-byte boundary
            let padded_len = attr_len + ((4 - (attr_len % 4)) % 4);
            offset += padded_len;
        }

        Ok(attrs)
    }

    fn decode_mapped_address(value: &[u8]) -> Result<StunAttribute, Error> {
        let addr = decode_address(value, false, &[0u8; 12])?;
        Ok(StunAttribute::MappedAddress(addr))
    }

    fn decode_xor_mapped_address(value: &[u8], txn_id: &[u8; 12]) -> Result<StunAttribute, Error> {
        let addr = decode_address(value, true, txn_id)?;
        Ok(StunAttribute::XorMappedAddress(addr))
    }

    fn decode_error_code(value: &[u8]) -> Result<StunAttribute, Error> {
        if value.len() < 4 {
            return Err(Error::StunError(
                "ERROR-CODE attribute too short".into(),
            ));
        }
        let class = (value[2] & 0x07) as u16;
        let number = value[3] as u16;
        let code = class * 100 + number;
        let reason = String::from_utf8_lossy(&value[4..]).into_owned();
        Ok(StunAttribute::ErrorCode { code, reason })
    }

    fn decode_software(value: &[u8]) -> StunAttribute {
        StunAttribute::Software(String::from_utf8_lossy(value).into_owned())
    }
}

/// Decode a MAPPED-ADDRESS or XOR-MAPPED-ADDRESS value.
///
/// Format (RFC 5389 Section 15.1/15.2):
/// ```text
///  0                   1                   2                   3
///  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |0 0 0 0 0 0 0 0|    Family     |           Port                |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |                                                               |
/// |                 Address (32 bits or 128 bits)                 |
/// |                                                               |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// ```
/// Decode a MAPPED-ADDRESS or XOR-MAPPED-ADDRESS attribute value.
pub fn decode_address(value: &[u8], xor: bool, txn_id: &[u8; 12]) -> Result<SocketAddr, Error> {
    if value.len() < 4 {
        return Err(Error::StunError("address attribute too short".into()));
    }

    let family = value[1];
    let raw_port = u16::from_be_bytes([value[2], value[3]]);

    let port = if xor {
        raw_port ^ (MAGIC_COOKIE >> 16) as u16
    } else {
        raw_port
    };

    match family {
        ADDR_FAMILY_IPV4 => {
            if value.len() < 8 {
                return Err(Error::StunError(
                    "IPv4 address attribute too short".into(),
                ));
            }
            let raw_ip = u32::from_be_bytes([value[4], value[5], value[6], value[7]]);
            let ip = if xor {
                Ipv4Addr::from(raw_ip ^ MAGIC_COOKIE)
            } else {
                Ipv4Addr::from(raw_ip)
            };
            Ok(SocketAddr::new(IpAddr::V4(ip), port))
        }
        ADDR_FAMILY_IPV6 => {
            if value.len() < 20 {
                return Err(Error::StunError(
                    "IPv6 address attribute too short".into(),
                ));
            }
            let mut ip_bytes = [0u8; 16];
            ip_bytes.copy_from_slice(&value[4..20]);

            if xor {
                // XOR with magic cookie (first 4 bytes) + transaction ID (12 bytes)
                let cookie_bytes = MAGIC_COOKIE.to_be_bytes();
                for i in 0..4 {
                    ip_bytes[i] ^= cookie_bytes[i];
                }
                for i in 0..12 {
                    ip_bytes[4 + i] ^= txn_id[i];
                }
            }

            let ip = Ipv6Addr::from(ip_bytes);
            Ok(SocketAddr::new(IpAddr::V6(ip), port))
        }
        other => Err(Error::StunError(format!(
            "unknown address family: 0x{:02X}",
            other
        ))),
    }
}

/// Encode an XOR address value (for XOR-MAPPED-ADDRESS, XOR-PEER-ADDRESS, XOR-RELAYED-ADDRESS).
///
/// Returns the encoded attribute value bytes (without type/length header).
pub fn encode_xor_address(addr: &SocketAddr, txn_id: &[u8; 12]) -> Vec<u8> {
    let cookie_bytes = MAGIC_COOKIE.to_be_bytes();

    match addr {
        SocketAddr::V4(v4) => {
            let port = v4.port() ^ (MAGIC_COOKIE >> 16) as u16;
            let ip = u32::from(v4.ip().to_owned()) ^ MAGIC_COOKIE;
            let mut buf = Vec::with_capacity(8);
            buf.push(0x00); // reserved
            buf.push(ADDR_FAMILY_IPV4);
            buf.extend_from_slice(&port.to_be_bytes());
            buf.extend_from_slice(&ip.to_be_bytes());
            buf
        }
        SocketAddr::V6(v6) => {
            let port = v6.port() ^ (MAGIC_COOKIE >> 16) as u16;
            let mut ip_bytes = v6.ip().octets();
            for i in 0..4 {
                ip_bytes[i] ^= cookie_bytes[i];
            }
            for i in 0..12 {
                ip_bytes[4 + i] ^= txn_id[i];
            }
            let mut buf = Vec::with_capacity(20);
            buf.push(0x00); // reserved
            buf.push(ADDR_FAMILY_IPV6);
            buf.extend_from_slice(&port.to_be_bytes());
            buf.extend_from_slice(&ip_bytes);
            buf
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transaction_id_random_uniqueness() {
        let id1 = TransactionId::random();
        let id2 = TransactionId::random();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_binding_request_encode_decode_roundtrip() {
        let msg = StunMessage::binding_request();
        let encoded = msg.encode();

        assert!(encoded.len() >= HEADER_SIZE);
        // First two bits must be 0
        assert_eq!(encoded[0] & 0xC0, 0);
        // Magic cookie
        let cookie = u32::from_be_bytes([encoded[4], encoded[5], encoded[6], encoded[7]]);
        assert_eq!(cookie, MAGIC_COOKIE);

        let decoded = StunMessage::decode(&encoded).unwrap_or_else(|e| panic!("decode failed: {e}"));
        assert_eq!(decoded.msg_type, BINDING_REQUEST);
        assert_eq!(decoded.transaction_id, msg.transaction_id);
    }

    #[test]
    fn test_xor_mapped_address_ipv4() {
        // Construct a fake Binding Response with XOR-MAPPED-ADDRESS
        let txn_id = TransactionId([0x01, 0x02, 0x03, 0x04, 0x05, 0x06,
                                     0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C]);

        let expected_addr: SocketAddr = "203.0.113.5:12345".parse()
            .unwrap_or_else(|e| panic!("parse addr: {e}"));

        // XOR the port: 12345 ^ (0x2112A442 >> 16) = 12345 ^ 0x2112
        let xor_port = (expected_addr.port() ^ (MAGIC_COOKIE >> 16) as u16).to_be_bytes();

        // XOR the IPv4 address with magic cookie
        let ip_bytes: [u8; 4] = match expected_addr.ip() {
            IpAddr::V4(v4) => v4.octets(),
            _ => unreachable!(),
        };
        let cookie_bytes = MAGIC_COOKIE.to_be_bytes();
        let xor_ip: [u8; 4] = [
            ip_bytes[0] ^ cookie_bytes[0],
            ip_bytes[1] ^ cookie_bytes[1],
            ip_bytes[2] ^ cookie_bytes[2],
            ip_bytes[3] ^ cookie_bytes[3],
        ];

        // Build the attribute value: 1 byte reserved + 1 byte family + 2 bytes port + 4 bytes addr
        let attr_value: Vec<u8> = vec![
            0x00, ADDR_FAMILY_IPV4,
            xor_port[0], xor_port[1],
            xor_ip[0], xor_ip[1], xor_ip[2], xor_ip[3],
        ];

        // Build full message
        let attr_type_bytes = ATTR_XOR_MAPPED_ADDRESS.to_be_bytes();
        let attr_len_bytes = (attr_value.len() as u16).to_be_bytes();

        let mut msg_bytes = Vec::new();
        // Header
        msg_bytes.extend_from_slice(&BINDING_RESPONSE.to_be_bytes());
        let total_attr_len = (ATTR_HEADER_SIZE + attr_value.len()) as u16;
        msg_bytes.extend_from_slice(&total_attr_len.to_be_bytes());
        msg_bytes.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
        msg_bytes.extend_from_slice(&txn_id.0);
        // Attribute
        msg_bytes.extend_from_slice(&attr_type_bytes);
        msg_bytes.extend_from_slice(&attr_len_bytes);
        msg_bytes.extend_from_slice(&attr_value);

        let decoded = StunMessage::decode(&msg_bytes).unwrap_or_else(|e| panic!("decode failed: {e}"));
        assert_eq!(decoded.msg_type, BINDING_RESPONSE);

        let addr = decoded.mapped_address();
        assert!(addr.is_some(), "no mapped address found");
        assert_eq!(addr.unwrap_or_else(|| panic!("unreachable")), expected_addr);
    }

    #[test]
    fn test_xor_mapped_address_ipv6() {
        let txn_id = TransactionId([0xA1, 0xB2, 0xC3, 0xD4, 0xE5, 0xF6,
                                     0x17, 0x28, 0x39, 0x4A, 0x5B, 0x6C]);

        let expected_addr: SocketAddr = "[2001:db8::1]:8080".parse()
            .unwrap_or_else(|e| panic!("parse addr: {e}"));

        let port = expected_addr.port();
        let xor_port = (port ^ (MAGIC_COOKIE >> 16) as u16).to_be_bytes();

        let ip_bytes: [u8; 16] = match expected_addr.ip() {
            IpAddr::V6(v6) => v6.octets(),
            _ => unreachable!(),
        };

        let cookie_bytes = MAGIC_COOKIE.to_be_bytes();
        let mut xor_ip = ip_bytes;
        for i in 0..4 {
            xor_ip[i] ^= cookie_bytes[i];
        }
        for i in 0..12 {
            xor_ip[4 + i] ^= txn_id.0[i];
        }

        let mut attr_value = vec![0x00, ADDR_FAMILY_IPV6, xor_port[0], xor_port[1]];
        attr_value.extend_from_slice(&xor_ip);

        let mut msg_bytes = Vec::new();
        msg_bytes.extend_from_slice(&BINDING_RESPONSE.to_be_bytes());
        let total_attr_len = (ATTR_HEADER_SIZE + attr_value.len()) as u16;
        msg_bytes.extend_from_slice(&total_attr_len.to_be_bytes());
        msg_bytes.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
        msg_bytes.extend_from_slice(&txn_id.0);
        msg_bytes.extend_from_slice(&ATTR_XOR_MAPPED_ADDRESS.to_be_bytes());
        msg_bytes.extend_from_slice(&(attr_value.len() as u16).to_be_bytes());
        msg_bytes.extend_from_slice(&attr_value);

        let decoded = StunMessage::decode(&msg_bytes).unwrap_or_else(|e| panic!("decode: {e}"));
        let addr = decoded.mapped_address();
        assert_eq!(addr.unwrap_or_else(|| panic!("no addr")), expected_addr);
    }

    #[test]
    fn test_error_code_attribute() {
        let txn_id = TransactionId::random();

        // Build an error response with code 420 (Unknown Attribute)
        let reason = b"Unknown Attribute";
        let mut error_value = vec![
            0x00, 0x00,
            0x04,          // class = 4
            0x14,          // number = 20 -> code = 420
        ];
        error_value.extend_from_slice(reason);

        let padding = (4 - (error_value.len() % 4)) % 4;
        let padded_error_len = error_value.len() + padding;

        let mut msg_bytes = Vec::new();
        msg_bytes.extend_from_slice(&BINDING_ERROR_RESPONSE.to_be_bytes());
        let total_attr_len = (ATTR_HEADER_SIZE + padded_error_len) as u16;
        msg_bytes.extend_from_slice(&total_attr_len.to_be_bytes());
        msg_bytes.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
        msg_bytes.extend_from_slice(&txn_id.0);
        msg_bytes.extend_from_slice(&ATTR_ERROR_CODE.to_be_bytes());
        msg_bytes.extend_from_slice(&(error_value.len() as u16).to_be_bytes());
        msg_bytes.extend_from_slice(&error_value);
        // padding
        msg_bytes.extend(std::iter::repeat(0u8).take(padding));

        let decoded = StunMessage::decode(&msg_bytes).unwrap_or_else(|e| panic!("decode: {e}"));
        assert_eq!(decoded.msg_type, BINDING_ERROR_RESPONSE);

        let (code, phrase) = decoded.error_code().unwrap_or_else(|| panic!("no error code"));
        assert_eq!(code, 420);
        assert_eq!(phrase, "Unknown Attribute");
    }

    #[test]
    fn test_decode_too_short() {
        let result = StunMessage::decode(&[0u8; 10]);
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_bad_magic_cookie() {
        let mut data = [0u8; 20];
        // Wrong cookie
        data[4] = 0xFF;
        let result = StunMessage::decode(&data);
        assert!(result.is_err());
    }
}
