//! RFC 8489 STUN codec, complete enough for ICE connectivity checks.
//!
//! Bytes in, bytes out — no sockets. The parser borrows the datagram and
//! verifies FINGERPRINT during parse (it is syntactic); MESSAGE-INTEGRITY
//! verification is a separate call because it needs a password the caller
//! resolves from the USERNAME, per the RFC 8489 §9.2.4 ordering.

use hmac::{Hmac, Mac};
use sha1::Sha1;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// RFC 8489 §5 magic cookie.
pub const MAGIC_COOKIE: u32 = 0x2112_A442;
/// Fixed STUN header length.
pub const HEADER_LEN: usize = 20;
/// Transaction id length.
pub const TXN_ID_LEN: usize = 12;

const ATTR_MAPPED_ADDRESS: u16 = 0x0001;
const ATTR_USERNAME: u16 = 0x0006;
const ATTR_MESSAGE_INTEGRITY: u16 = 0x0008;
const ATTR_ERROR_CODE: u16 = 0x0009;
const ATTR_UNKNOWN_ATTRIBUTES: u16 = 0x000A;
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;
const ATTR_PRIORITY: u16 = 0x0024;
const ATTR_USE_CANDIDATE: u16 = 0x0025;
const ATTR_SOFTWARE: u16 = 0x8022;
const ATTR_FINGERPRINT: u16 = 0x8028;
const ATTR_ICE_CONTROLLED: u16 = 0x8029;
const ATTR_ICE_CONTROLLING: u16 = 0x802A;

/// RFC 8489 §14.7: the CRC-32 is XORed with this before transmission.
const FINGERPRINT_XOR: u32 = 0x5354_554E;

/// STUN message class.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageClass {
    /// Binding request — a connectivity check or keepalive-with-consent.
    Request,
    /// Binding indication — a keepalive that expects no response.
    Indication,
    /// Success response to a request.
    SuccessResponse,
    /// Error response to a request.
    ErrorResponse,
}

/// A STUN transaction id: random per request, echoed in the response.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct TransactionId(pub [u8; TXN_ID_LEN]);

impl TransactionId {
    /// A fresh random transaction id.
    #[must_use]
    pub fn random() -> Self {
        let mut id = [0_u8; TXN_ID_LEN];
        rand::Rng::fill(&mut rand::thread_rng(), &mut id[..]);
        Self(id)
    }
}

impl std::fmt::Debug for TransactionId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Correlation material, not payload: show it, it is not a secret.
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Codec-level failures. `NotStun` specifically means "hand this datagram to
/// whatever else shares the socket" rather than "protocol error".
#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum StunError {
    /// The datagram is not a STUN message at all.
    #[error("not a STUN message")]
    NotStun,
    /// A STUN message, but malformed.
    #[error("malformed STUN message: {0}")]
    Malformed(&'static str),
    /// FINGERPRINT was present and did not match.
    #[error("STUN fingerprint mismatch")]
    FingerprintMismatch,
}

/// One attribute a message under construction carries, in order.
#[derive(Clone, Debug)]
pub enum Attribute {
    /// USERNAME — `receiver_ufrag:sender_ufrag` for ICE checks.
    Username(String),
    /// PRIORITY — the prflx priority the check's source would have.
    Priority(u32),
    /// USE-CANDIDATE — the controlling agent nominates this pair.
    UseCandidate,
    /// ICE-CONTROLLING with the sender's tie-breaker.
    IceControlling(u64),
    /// ICE-CONTROLLED with the sender's tie-breaker.
    IceControlled(u64),
    /// XOR-MAPPED-ADDRESS — the source address the server saw.
    XorMappedAddress(SocketAddr),
    /// ERROR-CODE with a reason phrase.
    ErrorCode {
        /// Numeric code, e.g. 487.
        code: u16,
        /// Human-readable reason.
        reason: String,
    },
    /// SOFTWARE — diagnostic only.
    Software(String),
    /// UNKNOWN-ATTRIBUTES for 420 responses.
    UnknownAttributes(Vec<u16>),
}

/// A STUN message under construction.
#[derive(Debug)]
pub struct StunMessage {
    class: MessageClass,
    transaction_id: TransactionId,
    attributes: Vec<Attribute>,
}

impl StunMessage {
    /// Start a Binding message of the given class.
    #[must_use]
    pub fn binding(class: MessageClass, transaction_id: TransactionId) -> Self {
        Self {
            class,
            transaction_id,
            attributes: Vec::new(),
        }
    }

    /// Append an attribute (order is preserved on the wire).
    #[must_use]
    pub fn with(mut self, attribute: Attribute) -> Self {
        self.attributes.push(attribute);
        self
    }

    /// Encode, optionally appending MESSAGE-INTEGRITY (HMAC-SHA1 with
    /// `integrity_key`, the short-term-credential password) and FINGERPRINT.
    ///
    /// ICE connectivity checks require both; plain server-discovery Binding
    /// requests carry neither. The two length-field rewrites this performs
    /// are the RFC 8489 §14.5/§14.7 rules: the integrity HMAC is computed as
    /// if MESSAGE-INTEGRITY were the final attribute, and the fingerprint
    /// CRC as if FINGERPRINT were.
    #[must_use]
    pub fn encode(&self, integrity_key: Option<&[u8]>, fingerprint: bool) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(128);
        let type_code: u16 = match self.class {
            MessageClass::Request => 0x0001,
            MessageClass::Indication => 0x0011,
            MessageClass::SuccessResponse => 0x0101,
            MessageClass::ErrorResponse => 0x0111,
        };
        buffer.extend_from_slice(&type_code.to_be_bytes());
        buffer.extend_from_slice(&0_u16.to_be_bytes());
        buffer.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
        buffer.extend_from_slice(&self.transaction_id.0);

        for attribute in &self.attributes {
            encode_attribute(&mut buffer, &self.transaction_id, attribute);
        }

        if let Some(key) = integrity_key {
            let length_through_integrity = (buffer.len() - HEADER_LEN + 24) as u16;
            buffer[2..4].copy_from_slice(&length_through_integrity.to_be_bytes());
            let mut mac =
                Hmac::<Sha1>::new_from_slice(key).expect("HMAC accepts any key length");
            mac.update(&buffer);
            let tag = mac.finalize().into_bytes();
            buffer.extend_from_slice(&ATTR_MESSAGE_INTEGRITY.to_be_bytes());
            buffer.extend_from_slice(&20_u16.to_be_bytes());
            buffer.extend_from_slice(&tag);
        }

        if fingerprint {
            // The CRC is computed over a header whose length already counts
            // the FINGERPRINT attribute itself (RFC 8489 §14.7).
            let length_through_fingerprint = (buffer.len() - HEADER_LEN + 8) as u16;
            buffer[2..4].copy_from_slice(&length_through_fingerprint.to_be_bytes());
            let crc = crc32(&buffer) ^ FINGERPRINT_XOR;
            buffer.extend_from_slice(&ATTR_FINGERPRINT.to_be_bytes());
            buffer.extend_from_slice(&4_u16.to_be_bytes());
            buffer.extend_from_slice(&crc.to_be_bytes());
        } else {
            // No fingerprint: the header length is simply what was written.
            let final_length = (buffer.len() - HEADER_LEN) as u16;
            buffer[2..4].copy_from_slice(&final_length.to_be_bytes());
        }
        buffer
    }
}

fn encode_attribute(buffer: &mut Vec<u8>, txn: &TransactionId, attribute: &Attribute) {
    let (attr_type, value): (u16, Vec<u8>) = match attribute {
        Attribute::Username(name) => (ATTR_USERNAME, name.as_bytes().to_vec()),
        Attribute::Priority(priority) => (ATTR_PRIORITY, priority.to_be_bytes().to_vec()),
        Attribute::UseCandidate => (ATTR_USE_CANDIDATE, Vec::new()),
        Attribute::IceControlling(tie) => (ATTR_ICE_CONTROLLING, tie.to_be_bytes().to_vec()),
        Attribute::IceControlled(tie) => (ATTR_ICE_CONTROLLED, tie.to_be_bytes().to_vec()),
        Attribute::XorMappedAddress(addr) => (ATTR_XOR_MAPPED_ADDRESS, encode_xor_addr(addr, txn)),
        Attribute::ErrorCode { code, reason } => {
            let mut value = vec![0, 0, (code / 100) as u8, (code % 100) as u8];
            value.extend_from_slice(reason.as_bytes());
            (ATTR_ERROR_CODE, value)
        }
        Attribute::Software(software) => (ATTR_SOFTWARE, software.as_bytes().to_vec()),
        Attribute::UnknownAttributes(types) => (
            ATTR_UNKNOWN_ATTRIBUTES,
            types.iter().flat_map(|t| t.to_be_bytes()).collect(),
        ),
    };
    buffer.extend_from_slice(&attr_type.to_be_bytes());
    buffer.extend_from_slice(&(value.len() as u16).to_be_bytes());
    buffer.extend_from_slice(&value);
    // Attributes are padded to 32-bit boundaries; padding bytes are ignored
    // by receivers, zero is the conventional fill.
    let padding = (4 - value.len() % 4) % 4;
    buffer.extend_from_slice(&[0_u8; 3][..padding]);
}

fn encode_xor_addr(addr: &SocketAddr, txn: &TransactionId) -> Vec<u8> {
    let xor_port = addr.port() ^ (MAGIC_COOKIE >> 16) as u16;
    match addr.ip() {
        IpAddr::V4(ip) => {
            let xored = u32::from(ip) ^ MAGIC_COOKIE;
            let mut value = vec![0, 0x01];
            value.extend_from_slice(&xor_port.to_be_bytes());
            value.extend_from_slice(&xored.to_be_bytes());
            value
        }
        IpAddr::V6(ip) => {
            let mut mask = [0_u8; 16];
            mask[..4].copy_from_slice(&MAGIC_COOKIE.to_be_bytes());
            mask[4..].copy_from_slice(&txn.0);
            let raw = ip.octets();
            let mut xored = [0_u8; 16];
            for index in 0..16 {
                xored[index] = raw[index] ^ mask[index];
            }
            let mut value = vec![0, 0x02];
            value.extend_from_slice(&xor_port.to_be_bytes());
            value.extend_from_slice(&xored);
            value
        }
    }
}

/// A parsed STUN message borrowing the datagram it came from.
///
/// FINGERPRINT, when present, was already verified by [`parse`]; integrity
/// is verified separately with [`ParsedMessage::verify_integrity`] once the
/// caller has resolved the password for the USERNAME.
#[derive(Debug)]
pub struct ParsedMessage<'a> {
    raw: &'a [u8],
    /// Message class.
    pub class: MessageClass,
    /// Transaction id.
    pub transaction_id: TransactionId,
    /// USERNAME attribute, when present and valid UTF-8.
    pub username: Option<&'a str>,
    /// PRIORITY attribute.
    pub priority: Option<u32>,
    /// USE-CANDIDATE presence.
    pub use_candidate: bool,
    /// ICE-CONTROLLING tie-breaker, when present.
    pub controlling: Option<u64>,
    /// ICE-CONTROLLED tie-breaker, when present.
    pub controlled: Option<u64>,
    /// XOR-MAPPED-ADDRESS, decoded.
    pub xor_mapped_address: Option<SocketAddr>,
    /// ERROR-CODE, when present.
    pub error: Option<(u16, String)>,
    /// Whether MESSAGE-INTEGRITY was present.
    pub has_integrity: bool,
    integrity_offset: Option<usize>,
}

/// Parse one datagram as STUN.
///
/// # Errors
///
/// [`StunError::NotStun`] when the datagram is not STUN (share-the-socket
/// case), [`StunError::Malformed`] for structural violations, and
/// [`StunError::FingerprintMismatch`] when a present FINGERPRINT fails —
/// which per RFC 8489 §7.3 means the packet must be discarded silently.
pub fn parse(data: &[u8]) -> Result<ParsedMessage<'_>, StunError> {
    if data.len() < HEADER_LEN || data[0] & 0b1100_0000 != 0 {
        return Err(StunError::NotStun);
    }
    if data[4..8] != MAGIC_COOKIE.to_be_bytes() {
        return Err(StunError::NotStun);
    }
    let type_code = u16::from_be_bytes([data[0], data[1]]);
    let class = match type_code {
        0x0001 => MessageClass::Request,
        0x0011 => MessageClass::Indication,
        0x0101 => MessageClass::SuccessResponse,
        0x0111 => MessageClass::ErrorResponse,
        _ => return Err(StunError::NotStun),
    };
    let length = u16::from_be_bytes([data[2], data[3]]) as usize;
    if !length.is_multiple_of(4) || HEADER_LEN + length != data.len() {
        return Err(StunError::Malformed("length does not match datagram"));
    }
    let mut transaction_id = [0_u8; TXN_ID_LEN];
    transaction_id.copy_from_slice(&data[8..20]);

    let mut message = ParsedMessage {
        raw: data,
        class,
        transaction_id: TransactionId(transaction_id),
        username: None,
        priority: None,
        use_candidate: false,
        controlling: None,
        controlled: None,
        xor_mapped_address: None,
        error: None,
        has_integrity: false,
        integrity_offset: None,
    };

    let mut cursor = HEADER_LEN;
    let mut fingerprint_offset: Option<usize> = None;
    while cursor + 4 <= data.len() {
        let attr_type = u16::from_be_bytes([data[cursor], data[cursor + 1]]);
        let attr_len = u16::from_be_bytes([data[cursor + 2], data[cursor + 3]]) as usize;
        let value_start = cursor + 4;
        let value_end = value_start + attr_len;
        if value_end > data.len() {
            return Err(StunError::Malformed("attribute overruns datagram"));
        }
        if fingerprint_offset.is_some() {
            return Err(StunError::Malformed("attribute after FINGERPRINT"));
        }
        let value = &data[value_start..value_end];
        match attr_type {
            ATTR_USERNAME => message.username = std::str::from_utf8(value).ok(),
            ATTR_PRIORITY => {
                if attr_len == 4 {
                    message.priority =
                        Some(u32::from_be_bytes([value[0], value[1], value[2], value[3]]));
                }
            }
            ATTR_USE_CANDIDATE => message.use_candidate = true,
            ATTR_ICE_CONTROLLING => {
                if attr_len == 8 {
                    let mut tie = [0_u8; 8];
                    tie.copy_from_slice(value);
                    message.controlling = Some(u64::from_be_bytes(tie));
                }
            }
            ATTR_ICE_CONTROLLED => {
                if attr_len == 8 {
                    let mut tie = [0_u8; 8];
                    tie.copy_from_slice(value);
                    message.controlled = Some(u64::from_be_bytes(tie));
                }
            }
            ATTR_XOR_MAPPED_ADDRESS | ATTR_MAPPED_ADDRESS => {
                message.xor_mapped_address = decode_addr(
                    value,
                    &message.transaction_id,
                    attr_type == ATTR_XOR_MAPPED_ADDRESS,
                );
            }
            ATTR_ERROR_CODE => {
                if attr_len >= 4 {
                    let code = u16::from(value[2]) * 100 + u16::from(value[3]);
                    let reason = String::from_utf8_lossy(&value[4..]).into_owned();
                    message.error = Some((code, reason));
                }
            }
            ATTR_MESSAGE_INTEGRITY => {
                if attr_len != 20 {
                    return Err(StunError::Malformed("MESSAGE-INTEGRITY length"));
                }
                message.has_integrity = true;
                message.integrity_offset = Some(cursor);
            }
            ATTR_FINGERPRINT => {
                if attr_len != 4 {
                    return Err(StunError::Malformed("FINGERPRINT length"));
                }
                let advertised = u32::from_be_bytes([value[0], value[1], value[2], value[3]]);
                let computed = crc32(&data[..cursor]) ^ FINGERPRINT_XOR;
                if advertised != computed {
                    return Err(StunError::FingerprintMismatch);
                }
                fingerprint_offset = Some(cursor);
            }
            _ => {}
        }
        cursor = value_end + (4 - attr_len % 4) % 4;
    }
    Ok(message)
}

impl ParsedMessage<'_> {
    /// Verify MESSAGE-INTEGRITY with the short-term-credential password.
    ///
    /// Returns `false` when the attribute is absent or the HMAC mismatches.
    /// The HMAC input has the header length rewritten as if
    /// MESSAGE-INTEGRITY were the final attribute (RFC 8489 §14.5), which
    /// is why this recomputes over a copy.
    #[must_use]
    pub fn verify_integrity(&self, key: &[u8]) -> bool {
        let Some(offset) = self.integrity_offset else {
            return false;
        };
        let mut covered = self.raw[..offset].to_vec();
        let patched_length = (offset - HEADER_LEN + 24) as u16;
        covered[2..4].copy_from_slice(&patched_length.to_be_bytes());
        let mut mac = Hmac::<Sha1>::new_from_slice(key).expect("HMAC accepts any key length");
        mac.update(&covered);
        let expected = &self.raw[offset + 4..offset + 24];
        mac.verify_slice(expected).is_ok()
    }
}

fn decode_addr(value: &[u8], txn: &TransactionId, xored: bool) -> Option<SocketAddr> {
    if value.len() < 8 {
        return None;
    }
    let family = value[1];
    let raw_port = u16::from_be_bytes([value[2], value[3]]);
    let port = if xored {
        raw_port ^ (MAGIC_COOKIE >> 16) as u16
    } else {
        raw_port
    };
    match family {
        0x01 => {
            let raw = u32::from_be_bytes([value[4], value[5], value[6], value[7]]);
            let ip = if xored { raw ^ MAGIC_COOKIE } else { raw };
            Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(ip)), port))
        }
        0x02 if value.len() >= 20 => {
            let mut octets = [0_u8; 16];
            octets.copy_from_slice(&value[4..20]);
            if xored {
                let mut mask = [0_u8; 16];
                mask[..4].copy_from_slice(&MAGIC_COOKIE.to_be_bytes());
                mask[4..].copy_from_slice(&txn.0);
                for index in 0..16 {
                    octets[index] ^= mask[index];
                }
            }
            Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(octets)), port))
        }
        _ => None,
    }
}

/// CRC-32 (ISO-HDLC), table-driven. Implemented here rather than pulling a
/// dependency: FINGERPRINT is its only caller.
fn crc32(data: &[u8]) -> u32 {
    const TABLE: [u32; 256] = {
        let mut table = [0_u32; 256];
        let mut index = 0;
        while index < 256 {
            let mut crc = index as u32;
            let mut bit = 0;
            while bit < 8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
                bit += 1;
            }
            table[index] = crc;
            index += 1;
        }
        table
    };
    let mut crc = 0xFFFF_FFFF_u32;
    for byte in data {
        crc = (crc >> 8) ^ TABLE[((crc ^ u32::from(*byte)) & 0xFF) as usize];
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 5769 §2.1 — sample request. The vector is self-validating: the
    /// FINGERPRINT covers every byte before it and the HMAC everything
    /// before *it*, so a transcription error cannot silently pass.
    const RFC5769_REQUEST: &[u8] = &[
        0x00, 0x01, 0x00, 0x58, 0x21, 0x12, 0xa4, 0x42, 0xb7, 0xe7, 0xa7, 0x01, 0xbc, 0x34,
        0xd6, 0x86, 0xfa, 0x87, 0xdf, 0xae, 0x80, 0x22, 0x00, 0x10, 0x53, 0x54, 0x55, 0x4e,
        0x20, 0x74, 0x65, 0x73, 0x74, 0x20, 0x63, 0x6c, 0x69, 0x65, 0x6e, 0x74, 0x00, 0x24,
        0x00, 0x04, 0x6e, 0x00, 0x01, 0xff, 0x80, 0x29, 0x00, 0x08, 0x93, 0x2f, 0xf9, 0xb1,
        0x51, 0x26, 0x3b, 0x36, 0x00, 0x06, 0x00, 0x09, 0x65, 0x76, 0x74, 0x6a, 0x3a, 0x68,
        0x36, 0x76, 0x59, 0x20, 0x20, 0x20, 0x00, 0x08, 0x00, 0x14, 0x9a, 0xea, 0xa7, 0x0c,
        0xbf, 0xd8, 0xcb, 0x56, 0x78, 0x1e, 0xf2, 0xb5, 0xb2, 0xd3, 0xf2, 0x49, 0xc1, 0xb5,
        0x71, 0xa2, 0x80, 0x28, 0x00, 0x04, 0xe5, 0x7a, 0x3b, 0xcf,
    ];

    /// RFC 5769 §2.2 — sample IPv4 response.
    const RFC5769_RESPONSE: &[u8] = &[
        0x01, 0x01, 0x00, 0x3c, 0x21, 0x12, 0xa4, 0x42, 0xb7, 0xe7, 0xa7, 0x01, 0xbc, 0x34,
        0xd6, 0x86, 0xfa, 0x87, 0xdf, 0xae, 0x80, 0x22, 0x00, 0x0b, 0x74, 0x65, 0x73, 0x74,
        0x20, 0x76, 0x65, 0x63, 0x74, 0x6f, 0x72, 0x20, 0x00, 0x20, 0x00, 0x08, 0x00, 0x01,
        0xa1, 0x47, 0xe1, 0x12, 0xa6, 0x43, 0x00, 0x08, 0x00, 0x14, 0x2b, 0x91, 0xf5, 0x99,
        0xfd, 0x9e, 0x90, 0xc3, 0x8c, 0x74, 0x89, 0xf9, 0x2a, 0xf9, 0xba, 0x53, 0xf0, 0x6b,
        0xe7, 0xd7, 0x80, 0x28, 0x00, 0x04, 0xc0, 0x7d, 0x4c, 0x96,
    ];

    const RFC5769_PASSWORD: &[u8] = b"VOkJxbRl1RmTxUk/WvJxBt";

    #[test]
    fn rfc5769_request_parses_and_authenticates() {
        let message = parse(RFC5769_REQUEST).expect("fingerprint must verify during parse");
        assert_eq!(message.class, MessageClass::Request);
        assert_eq!(message.username, Some("evtj:h6vY"));
        assert_eq!(message.priority, Some(0x6e00_01ff));
        assert_eq!(message.controlled, Some(0x932f_f9b1_5126_3b36));
        assert!(message.verify_integrity(RFC5769_PASSWORD));
        assert!(
            !message.verify_integrity(b"wrong password"),
            "a wrong password must not verify"
        );
    }

    #[test]
    fn rfc5769_response_parses_and_authenticates() {
        let message = parse(RFC5769_RESPONSE).expect("parse");
        assert_eq!(message.class, MessageClass::SuccessResponse);
        assert_eq!(
            message.xor_mapped_address,
            Some("192.0.2.1:32853".parse().unwrap())
        );
        assert!(message.verify_integrity(RFC5769_PASSWORD));
    }

    #[test]
    fn a_flipped_byte_fails_the_fingerprint_and_is_discarded() {
        let mut tampered = RFC5769_REQUEST.to_vec();
        tampered[45] ^= 0x01; // inside PRIORITY
        assert!(matches!(
            parse(&tampered),
            Err(StunError::FingerprintMismatch)
        ));
    }

    #[test]
    fn ice_check_roundtrip_encodes_and_authenticates() {
        let txn = TransactionId::random();
        let encoded = StunMessage::binding(MessageClass::Request, txn)
            .with(Attribute::Username("aBcD:eFgH".into()))
            .with(Attribute::Priority(0x1234_5678))
            .with(Attribute::IceControlling(42))
            .with(Attribute::UseCandidate)
            .encode(Some(b"the-short-term-password"), true);
        let parsed = parse(&encoded).expect("roundtrip parse");
        assert_eq!(parsed.class, MessageClass::Request);
        assert_eq!(parsed.username, Some("aBcD:eFgH"));
        assert_eq!(parsed.priority, Some(0x1234_5678));
        assert_eq!(parsed.controlling, Some(42));
        assert!(parsed.use_candidate);
        assert!(parsed.verify_integrity(b"the-short-term-password"));
        assert!(!parsed.verify_integrity(b"not-it"));
    }

    #[test]
    fn xor_mapped_address_roundtrips_for_both_families() {
        for addr in ["203.0.113.7:49152", "[2001:db8::7]:5004"] {
            let addr: SocketAddr = addr.parse().unwrap();
            let txn = TransactionId::random();
            let encoded = StunMessage::binding(MessageClass::SuccessResponse, txn)
                .with(Attribute::XorMappedAddress(addr))
                .encode(Some(b"pw"), true);
            let parsed = parse(&encoded).expect("parse");
            assert_eq!(parsed.xor_mapped_address, Some(addr));
        }
    }

    #[test]
    fn error_response_roundtrips_a_role_conflict() {
        let txn = TransactionId::random();
        let encoded = StunMessage::binding(MessageClass::ErrorResponse, txn)
            .with(Attribute::ErrorCode {
                code: 487,
                reason: "Role Conflict".into(),
            })
            .encode(Some(b"pw"), true);
        let parsed = parse(&encoded).expect("parse");
        assert_eq!(parsed.error, Some((487, "Role Conflict".into())));
    }

    #[test]
    fn non_stun_datagrams_are_identified_not_errored() {
        assert!(matches!(
            parse(&[0x80, 0x00, 0x00, 0x00]),
            Err(StunError::NotStun)
        ));
        let rtp_like = [0x80_u8; 24];
        assert!(matches!(parse(&rtp_like), Err(StunError::NotStun)));
    }
}
