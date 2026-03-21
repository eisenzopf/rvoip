//! TURN-specific message types and attributes per RFC 5766.
//!
//! Extends the STUN message format with TURN methods (Allocate, Refresh,
//! CreatePermission, ChannelBind, Send, Data) and TURN-specific attributes
//! (CHANNEL-NUMBER, LIFETIME, XOR-PEER-ADDRESS, DATA, XOR-RELAYED-ADDRESS,
//! REQUESTED-TRANSPORT, DONT-FRAGMENT).

use std::net::SocketAddr;

use crate::Error;
use crate::stun::message::{
    self, StunMessage, TransactionId, MAGIC_COOKIE, HEADER_SIZE, ATTR_HEADER_SIZE,
    ATTR_USERNAME, ATTR_MESSAGE_INTEGRITY, ATTR_REALM, ATTR_NONCE, ATTR_ERROR_CODE,
    decode_address, encode_xor_address,
};

// ---------------------------------------------------------------------------
// TURN message types (RFC 5766 Section 13)
// ---------------------------------------------------------------------------

/// Allocate Request (method 0x003, class Request).
pub const ALLOCATE_REQUEST: u16 = 0x0003;
/// Allocate Success Response.
pub const ALLOCATE_RESPONSE: u16 = 0x0103;
/// Allocate Error Response.
pub const ALLOCATE_ERROR_RESPONSE: u16 = 0x0113;

/// Refresh Request.
pub const REFRESH_REQUEST: u16 = 0x0004;
/// Refresh Success Response.
pub const REFRESH_RESPONSE: u16 = 0x0104;
/// Refresh Error Response.
pub const REFRESH_ERROR_RESPONSE: u16 = 0x0114;

/// CreatePermission Request.
pub const CREATE_PERMISSION_REQUEST: u16 = 0x0008;
/// CreatePermission Success Response.
pub const CREATE_PERMISSION_RESPONSE: u16 = 0x0108;
/// CreatePermission Error Response.
pub const CREATE_PERMISSION_ERROR_RESPONSE: u16 = 0x0118;

/// ChannelBind Request.
pub const CHANNEL_BIND_REQUEST: u16 = 0x0009;
/// ChannelBind Success Response.
pub const CHANNEL_BIND_RESPONSE: u16 = 0x0109;
/// ChannelBind Error Response.
pub const CHANNEL_BIND_ERROR_RESPONSE: u16 = 0x0119;

/// Send Indication.
pub const SEND_INDICATION: u16 = 0x0016;
/// Data Indication.
pub const DATA_INDICATION: u16 = 0x0017;

// ---------------------------------------------------------------------------
// TURN attribute types (RFC 5766 Section 14)
// ---------------------------------------------------------------------------

/// CHANNEL-NUMBER attribute.
pub const ATTR_CHANNEL_NUMBER: u16 = 0x000C;
/// LIFETIME attribute.
pub const ATTR_LIFETIME: u16 = 0x000D;
/// XOR-PEER-ADDRESS attribute.
pub const ATTR_XOR_PEER_ADDRESS: u16 = 0x0012;
/// DATA attribute.
pub const ATTR_DATA: u16 = 0x0013;
/// XOR-RELAYED-ADDRESS attribute.
pub const ATTR_XOR_RELAYED_ADDRESS: u16 = 0x0016;
/// REQUESTED-TRANSPORT attribute.
pub const ATTR_REQUESTED_TRANSPORT: u16 = 0x0019;
/// DONT-FRAGMENT attribute.
pub const ATTR_DONT_FRAGMENT: u16 = 0x001A;

/// UDP transport protocol number for REQUESTED-TRANSPORT.
pub const TRANSPORT_UDP: u8 = 17;

// ---------------------------------------------------------------------------
// Helper: classify a STUN/TURN message type
// ---------------------------------------------------------------------------

/// Identifies whether a message type is a TURN method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnMessageType {
    /// Allocate method.
    Allocate,
    /// Refresh method.
    Refresh,
    /// CreatePermission method.
    CreatePermission,
    /// ChannelBind method.
    ChannelBind,
    /// Send indication.
    Send,
    /// Data indication.
    Data,
    /// Some other STUN message type (e.g., Binding).
    Other(u16),
}

impl TurnMessageType {
    /// Classify a raw STUN/TURN message type value.
    pub fn from_raw(msg_type: u16) -> Self {
        // Extract method from STUN message type (RFC 5389 Section 6):
        //   M3..M0 = bits 3..0, M6..M4 = bits 7..5, M11..M7 = bits 13..9
        //   C0 = bit 4, C1 = bit 8 (class bits, stripped for method)
        let method = (msg_type & 0x000F)
            | ((msg_type >> 1) & 0x0070)
            | ((msg_type >> 2) & 0x0F80);

        match method {
            0x0003 => Self::Allocate,
            0x0004 => Self::Refresh,
            0x0008 => Self::CreatePermission,
            0x0009 => Self::ChannelBind,
            0x0006 => Self::Send,
            0x0007 => Self::Data,
            _ => Self::Other(msg_type),
        }
    }
}

// ---------------------------------------------------------------------------
// TURN attribute: parsed representation
// ---------------------------------------------------------------------------

/// A parsed TURN-specific attribute.
#[derive(Debug, Clone)]
pub enum TurnAttribute {
    /// CHANNEL-NUMBER: channel number (0x4000..0x7FFF valid range).
    ChannelNumber(u16),
    /// LIFETIME: allocation lifetime in seconds.
    Lifetime(u32),
    /// XOR-PEER-ADDRESS: the peer's transport address (XOR encoded).
    XorPeerAddress(SocketAddr),
    /// DATA: opaque application data relayed through the TURN server.
    Data(Vec<u8>),
    /// XOR-RELAYED-ADDRESS: the relay address allocated by the server.
    XorRelayedAddress(SocketAddr),
    /// REQUESTED-TRANSPORT: transport protocol (always UDP = 17).
    RequestedTransport(u8),
    /// DONT-FRAGMENT: request that the server set DF bit.
    DontFragment,
    /// REALM: authentication realm string.
    Realm(String),
    /// NONCE: authentication nonce string.
    Nonce(String),
    /// USERNAME: authentication username.
    Username(String),
    /// MESSAGE-INTEGRITY: HMAC-SHA1 value (20 bytes).
    MessageIntegrity([u8; 20]),
    /// ERROR-CODE: error class/number + reason phrase.
    ErrorCode { code: u16, reason: String },
}

// ---------------------------------------------------------------------------
// Encode helpers
// ---------------------------------------------------------------------------

/// Build a complete STUN/TURN message from parts.
///
/// This produces a raw byte buffer with the 20-byte STUN header followed by
/// the encoded attributes. The MESSAGE-INTEGRITY attribute, if desired, must
/// be computed separately (see [`crate::turn::credentials`]).
pub fn build_turn_message(
    msg_type: u16,
    txn_id: &TransactionId,
    attributes: &[TurnAttribute],
) -> Vec<u8> {
    let attr_bytes = encode_turn_attributes(attributes, &txn_id.0);

    let mut buf = Vec::with_capacity(HEADER_SIZE + attr_bytes.len());
    buf.extend_from_slice(&msg_type.to_be_bytes());
    buf.extend_from_slice(&(attr_bytes.len() as u16).to_be_bytes());
    buf.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
    buf.extend_from_slice(&txn_id.0);
    buf.extend_from_slice(&attr_bytes);
    buf
}

/// Encode a list of TURN attributes into a raw byte buffer.
fn encode_turn_attributes(attributes: &[TurnAttribute], txn_id: &[u8; 12]) -> Vec<u8> {
    let mut buf = Vec::new();
    for attr in attributes {
        match attr {
            TurnAttribute::ChannelNumber(ch) => {
                // CHANNEL-NUMBER is 4 bytes: 2 bytes number + 2 bytes RFFU (reserved)
                let mut val = Vec::with_capacity(4);
                val.extend_from_slice(&ch.to_be_bytes());
                val.extend_from_slice(&[0x00, 0x00]); // RFFU
                StunMessage::encode_attr(&mut buf, ATTR_CHANNEL_NUMBER, &val);
            }
            TurnAttribute::Lifetime(secs) => {
                StunMessage::encode_attr(&mut buf, ATTR_LIFETIME, &secs.to_be_bytes());
            }
            TurnAttribute::XorPeerAddress(addr) => {
                let val = encode_xor_address(addr, txn_id);
                StunMessage::encode_attr(&mut buf, ATTR_XOR_PEER_ADDRESS, &val);
            }
            TurnAttribute::Data(data) => {
                StunMessage::encode_attr(&mut buf, ATTR_DATA, data);
            }
            TurnAttribute::XorRelayedAddress(addr) => {
                let val = encode_xor_address(addr, txn_id);
                StunMessage::encode_attr(&mut buf, ATTR_XOR_RELAYED_ADDRESS, &val);
            }
            TurnAttribute::RequestedTransport(proto) => {
                // 4 bytes: 1 byte protocol + 3 bytes RFFU
                let val = [*proto, 0x00, 0x00, 0x00];
                StunMessage::encode_attr(&mut buf, ATTR_REQUESTED_TRANSPORT, &val);
            }
            TurnAttribute::DontFragment => {
                StunMessage::encode_attr(&mut buf, ATTR_DONT_FRAGMENT, &[]);
            }
            TurnAttribute::Realm(s) => {
                StunMessage::encode_attr(&mut buf, ATTR_REALM, s.as_bytes());
            }
            TurnAttribute::Nonce(s) => {
                StunMessage::encode_attr(&mut buf, ATTR_NONCE, s.as_bytes());
            }
            TurnAttribute::Username(s) => {
                StunMessage::encode_attr(&mut buf, ATTR_USERNAME, s.as_bytes());
            }
            TurnAttribute::MessageIntegrity(hmac_val) => {
                StunMessage::encode_attr(&mut buf, ATTR_MESSAGE_INTEGRITY, hmac_val);
            }
            TurnAttribute::ErrorCode { code, reason } => {
                let class = (code / 100) as u8;
                let number = (code % 100) as u8;
                let mut val = vec![0x00, 0x00, class & 0x07, number];
                val.extend_from_slice(reason.as_bytes());
                StunMessage::encode_attr(&mut buf, ATTR_ERROR_CODE, &val);
            }
        }
    }
    buf
}

// ---------------------------------------------------------------------------
// Decode helpers
// ---------------------------------------------------------------------------

/// Parse TURN-specific attributes from a decoded STUN message.
///
/// Iterates over the raw attribute area of a STUN message and extracts
/// both standard STUN attributes (REALM, NONCE, USERNAME, MESSAGE-INTEGRITY,
/// ERROR-CODE) and TURN-specific attributes.
pub fn decode_turn_attributes(data: &[u8], txn_id: &[u8; 12]) -> Result<Vec<TurnAttribute>, Error> {
    let mut attrs = Vec::new();
    let mut offset = 0;

    while offset + ATTR_HEADER_SIZE <= data.len() {
        let attr_type = u16::from_be_bytes([data[offset], data[offset + 1]]);
        let attr_len = u16::from_be_bytes([data[offset + 2], data[offset + 3]]) as usize;
        offset += ATTR_HEADER_SIZE;

        if offset + attr_len > data.len() {
            return Err(Error::TurnError(format!(
                "TURN attribute 0x{:04X} truncated: need {} bytes at offset {} but only {} remain",
                attr_type, attr_len, offset, data.len() - offset
            )));
        }

        let value = &data[offset..offset + attr_len];

        let attr = match attr_type {
            ATTR_CHANNEL_NUMBER => {
                if value.len() < 4 {
                    return Err(Error::TurnError("CHANNEL-NUMBER too short".into()));
                }
                let ch = u16::from_be_bytes([value[0], value[1]]);
                Some(TurnAttribute::ChannelNumber(ch))
            }
            ATTR_LIFETIME => {
                if value.len() < 4 {
                    return Err(Error::TurnError("LIFETIME too short".into()));
                }
                let secs = u32::from_be_bytes([value[0], value[1], value[2], value[3]]);
                Some(TurnAttribute::Lifetime(secs))
            }
            ATTR_XOR_PEER_ADDRESS => {
                let addr = decode_address(value, true, txn_id)
                    .map_err(|e| Error::TurnError(format!("XOR-PEER-ADDRESS: {e}")))?;
                Some(TurnAttribute::XorPeerAddress(addr))
            }
            ATTR_DATA => {
                Some(TurnAttribute::Data(value.to_vec()))
            }
            ATTR_XOR_RELAYED_ADDRESS => {
                let addr = decode_address(value, true, txn_id)
                    .map_err(|e| Error::TurnError(format!("XOR-RELAYED-ADDRESS: {e}")))?;
                Some(TurnAttribute::XorRelayedAddress(addr))
            }
            ATTR_REQUESTED_TRANSPORT => {
                if value.is_empty() {
                    return Err(Error::TurnError("REQUESTED-TRANSPORT too short".into()));
                }
                Some(TurnAttribute::RequestedTransport(value[0]))
            }
            ATTR_DONT_FRAGMENT => {
                Some(TurnAttribute::DontFragment)
            }
            ATTR_REALM => {
                Some(TurnAttribute::Realm(String::from_utf8_lossy(value).into_owned()))
            }
            ATTR_NONCE => {
                Some(TurnAttribute::Nonce(String::from_utf8_lossy(value).into_owned()))
            }
            ATTR_USERNAME => {
                Some(TurnAttribute::Username(String::from_utf8_lossy(value).into_owned()))
            }
            ATTR_MESSAGE_INTEGRITY => {
                if value.len() >= 20 {
                    let mut hmac_val = [0u8; 20];
                    hmac_val.copy_from_slice(&value[..20]);
                    Some(TurnAttribute::MessageIntegrity(hmac_val))
                } else {
                    None // skip malformed
                }
            }
            ATTR_ERROR_CODE => {
                if value.len() < 4 {
                    return Err(Error::TurnError("ERROR-CODE too short".into()));
                }
                let class = (value[2] & 0x07) as u16;
                let number = value[3] as u16;
                let code = class * 100 + number;
                let reason = String::from_utf8_lossy(&value[4..]).into_owned();
                Some(TurnAttribute::ErrorCode { code, reason })
            }
            // XOR-MAPPED-ADDRESS (0x0020) — also used in Allocate responses
            0x0020 => {
                let addr = decode_address(value, true, txn_id)
                    .map_err(|e| Error::TurnError(format!("XOR-MAPPED-ADDRESS: {e}")))?;
                Some(TurnAttribute::XorPeerAddress(addr))
            }
            _ => None, // skip unknown attributes
        };

        if let Some(a) = attr {
            attrs.push(a);
        }

        // Advance past value + padding
        let padded_len = attr_len + ((4 - (attr_len % 4)) % 4);
        offset += padded_len;
    }

    Ok(attrs)
}

/// Parse a complete STUN/TURN message and extract TURN attributes.
pub fn parse_turn_response(data: &[u8]) -> Result<(StunMessage, Vec<TurnAttribute>), Error> {
    let msg = StunMessage::decode(data)?;
    let msg_len = u16::from_be_bytes([data[2], data[3]]) as usize;
    let turn_attrs = decode_turn_attributes(&data[HEADER_SIZE..HEADER_SIZE + msg_len], &msg.transaction_id.0)?;
    Ok((msg, turn_attrs))
}

// ---------------------------------------------------------------------------
// Attribute lookup helpers
// ---------------------------------------------------------------------------

/// Find the first XOR-RELAYED-ADDRESS in a list of TURN attributes.
pub fn find_relayed_address(attrs: &[TurnAttribute]) -> Option<SocketAddr> {
    for attr in attrs {
        if let TurnAttribute::XorRelayedAddress(addr) = attr {
            return Some(*addr);
        }
    }
    None
}

/// Find the first XOR-PEER-ADDRESS (also used for XOR-MAPPED-ADDRESS).
pub fn find_mapped_address(attrs: &[TurnAttribute]) -> Option<SocketAddr> {
    for attr in attrs {
        if let TurnAttribute::XorPeerAddress(addr) = attr {
            return Some(*addr);
        }
    }
    None
}

/// Find the LIFETIME attribute value.
pub fn find_lifetime(attrs: &[TurnAttribute]) -> Option<u32> {
    for attr in attrs {
        if let TurnAttribute::Lifetime(secs) = attr {
            return Some(*secs);
        }
    }
    None
}

/// Find the REALM attribute value.
pub fn find_realm(attrs: &[TurnAttribute]) -> Option<&str> {
    for attr in attrs {
        if let TurnAttribute::Realm(s) = attr {
            return Some(s.as_str());
        }
    }
    None
}

/// Find the NONCE attribute value.
pub fn find_nonce(attrs: &[TurnAttribute]) -> Option<&str> {
    for attr in attrs {
        if let TurnAttribute::Nonce(s) = attr {
            return Some(s.as_str());
        }
    }
    None
}

/// Find the ERROR-CODE attribute.
pub fn find_error_code(attrs: &[TurnAttribute]) -> Option<(u16, &str)> {
    for attr in attrs {
        if let TurnAttribute::ErrorCode { code, reason } = attr {
            return Some((*code, reason.as_str()));
        }
    }
    None
}

/// Find the DATA attribute value.
pub fn find_data(attrs: &[TurnAttribute]) -> Option<&[u8]> {
    for attr in attrs {
        if let TurnAttribute::Data(d) = attr {
            return Some(d.as_slice());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_turn_message_type_classification() {
        assert_eq!(TurnMessageType::from_raw(ALLOCATE_REQUEST), TurnMessageType::Allocate);
        assert_eq!(TurnMessageType::from_raw(ALLOCATE_RESPONSE), TurnMessageType::Allocate);
        assert_eq!(TurnMessageType::from_raw(ALLOCATE_ERROR_RESPONSE), TurnMessageType::Allocate);
        assert_eq!(TurnMessageType::from_raw(REFRESH_REQUEST), TurnMessageType::Refresh);
        assert_eq!(TurnMessageType::from_raw(CREATE_PERMISSION_REQUEST), TurnMessageType::CreatePermission);
        assert_eq!(TurnMessageType::from_raw(CHANNEL_BIND_REQUEST), TurnMessageType::ChannelBind);
        assert_eq!(TurnMessageType::from_raw(SEND_INDICATION), TurnMessageType::Send);
        assert_eq!(TurnMessageType::from_raw(DATA_INDICATION), TurnMessageType::Data);
    }

    #[test]
    fn test_allocate_request_encode_decode_roundtrip() {
        let txn_id = TransactionId::random();
        let attrs = vec![
            TurnAttribute::RequestedTransport(TRANSPORT_UDP),
            TurnAttribute::Lifetime(600),
        ];

        let encoded = build_turn_message(ALLOCATE_REQUEST, &txn_id, &attrs);

        // Verify header
        assert!(encoded.len() >= HEADER_SIZE);
        let msg_type = u16::from_be_bytes([encoded[0], encoded[1]]);
        assert_eq!(msg_type, ALLOCATE_REQUEST);

        // Decode and verify attributes
        let msg_len = u16::from_be_bytes([encoded[2], encoded[3]]) as usize;
        let decoded_attrs = decode_turn_attributes(
            &encoded[HEADER_SIZE..HEADER_SIZE + msg_len],
            &txn_id.0,
        ).unwrap_or_else(|e| panic!("decode failed: {e}"));

        // Should have REQUESTED-TRANSPORT and LIFETIME
        let mut found_transport = false;
        let mut found_lifetime = false;
        for attr in &decoded_attrs {
            match attr {
                TurnAttribute::RequestedTransport(proto) => {
                    assert_eq!(*proto, TRANSPORT_UDP);
                    found_transport = true;
                }
                TurnAttribute::Lifetime(secs) => {
                    assert_eq!(*secs, 600);
                    found_lifetime = true;
                }
                _ => {}
            }
        }
        assert!(found_transport, "REQUESTED-TRANSPORT not found");
        assert!(found_lifetime, "LIFETIME not found");
    }

    #[test]
    fn test_xor_peer_address_encode_decode() {
        let txn_id = TransactionId([0x01, 0x02, 0x03, 0x04, 0x05, 0x06,
                                     0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C]);
        let peer_addr: SocketAddr = "198.51.100.42:9000".parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));

        let attrs = vec![TurnAttribute::XorPeerAddress(peer_addr)];
        let encoded = build_turn_message(CREATE_PERMISSION_REQUEST, &txn_id, &attrs);

        let msg_len = u16::from_be_bytes([encoded[2], encoded[3]]) as usize;
        let decoded_attrs = decode_turn_attributes(
            &encoded[HEADER_SIZE..HEADER_SIZE + msg_len],
            &txn_id.0,
        ).unwrap_or_else(|e| panic!("decode failed: {e}"));

        let found = find_mapped_address(&decoded_attrs);
        assert_eq!(found, Some(peer_addr));
    }

    #[test]
    fn test_xor_relayed_address_encode_decode() {
        let txn_id = TransactionId([0xAA; 12]);
        let relay_addr: SocketAddr = "203.0.113.10:50000".parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));

        let attrs = vec![TurnAttribute::XorRelayedAddress(relay_addr)];
        let encoded = build_turn_message(ALLOCATE_RESPONSE, &txn_id, &attrs);

        let msg_len = u16::from_be_bytes([encoded[2], encoded[3]]) as usize;
        let decoded_attrs = decode_turn_attributes(
            &encoded[HEADER_SIZE..HEADER_SIZE + msg_len],
            &txn_id.0,
        ).unwrap_or_else(|e| panic!("decode failed: {e}"));

        let found = find_relayed_address(&decoded_attrs);
        assert_eq!(found, Some(relay_addr));
    }

    #[test]
    fn test_channel_number_encode_decode() {
        let txn_id = TransactionId::random();
        let channel: u16 = 0x4000;
        let peer_addr: SocketAddr = "10.0.0.1:5060".parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));

        let attrs = vec![
            TurnAttribute::ChannelNumber(channel),
            TurnAttribute::XorPeerAddress(peer_addr),
        ];
        let encoded = build_turn_message(CHANNEL_BIND_REQUEST, &txn_id, &attrs);

        let msg_len = u16::from_be_bytes([encoded[2], encoded[3]]) as usize;
        let decoded_attrs = decode_turn_attributes(
            &encoded[HEADER_SIZE..HEADER_SIZE + msg_len],
            &txn_id.0,
        ).unwrap_or_else(|e| panic!("decode failed: {e}"));

        let mut found_channel = false;
        for attr in &decoded_attrs {
            if let TurnAttribute::ChannelNumber(ch) = attr {
                assert_eq!(*ch, channel);
                found_channel = true;
            }
        }
        assert!(found_channel, "CHANNEL-NUMBER not found");
    }

    #[test]
    fn test_data_attribute_roundtrip() {
        let txn_id = TransactionId::random();
        let payload = b"Hello from TURN relay!";

        let attrs = vec![
            TurnAttribute::XorPeerAddress("192.168.1.1:8000".parse()
                .unwrap_or_else(|e| panic!("parse: {e}"))),
            TurnAttribute::Data(payload.to_vec()),
        ];
        let encoded = build_turn_message(SEND_INDICATION, &txn_id, &attrs);

        let msg_len = u16::from_be_bytes([encoded[2], encoded[3]]) as usize;
        let decoded_attrs = decode_turn_attributes(
            &encoded[HEADER_SIZE..HEADER_SIZE + msg_len],
            &txn_id.0,
        ).unwrap_or_else(|e| panic!("decode failed: {e}"));

        let data = find_data(&decoded_attrs);
        assert_eq!(data, Some(payload.as_slice()));
    }

    #[test]
    fn test_error_code_401_roundtrip() {
        let txn_id = TransactionId::random();
        let attrs = vec![
            TurnAttribute::ErrorCode { code: 401, reason: "Unauthorized".into() },
            TurnAttribute::Realm("example.com".into()),
            TurnAttribute::Nonce("abc123".into()),
        ];
        let encoded = build_turn_message(ALLOCATE_ERROR_RESPONSE, &txn_id, &attrs);

        let msg_len = u16::from_be_bytes([encoded[2], encoded[3]]) as usize;
        let decoded_attrs = decode_turn_attributes(
            &encoded[HEADER_SIZE..HEADER_SIZE + msg_len],
            &txn_id.0,
        ).unwrap_or_else(|e| panic!("decode failed: {e}"));

        let error = find_error_code(&decoded_attrs);
        assert!(error.is_some());
        let (code, reason) = error.unwrap_or_else(|| panic!("no error"));
        assert_eq!(code, 401);
        assert_eq!(reason, "Unauthorized");

        let realm = find_realm(&decoded_attrs);
        assert_eq!(realm, Some("example.com"));

        let nonce = find_nonce(&decoded_attrs);
        assert_eq!(nonce, Some("abc123"));
    }

    #[test]
    fn test_send_indication_structure() {
        let txn_id = TransactionId::random();
        let peer: SocketAddr = "10.0.0.5:3478".parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));
        let payload = vec![0xDE, 0xAD, 0xBE, 0xEF];

        let attrs = vec![
            TurnAttribute::XorPeerAddress(peer),
            TurnAttribute::Data(payload.clone()),
            TurnAttribute::DontFragment,
        ];
        let encoded = build_turn_message(SEND_INDICATION, &txn_id, &attrs);

        let msg_type = u16::from_be_bytes([encoded[0], encoded[1]]);
        assert_eq!(msg_type, SEND_INDICATION);

        let msg_len = u16::from_be_bytes([encoded[2], encoded[3]]) as usize;
        let decoded_attrs = decode_turn_attributes(
            &encoded[HEADER_SIZE..HEADER_SIZE + msg_len],
            &txn_id.0,
        ).unwrap_or_else(|e| panic!("decode failed: {e}"));

        let peer_found = find_mapped_address(&decoded_attrs);
        assert_eq!(peer_found, Some(peer));

        let data = find_data(&decoded_attrs);
        assert_eq!(data, Some(payload.as_slice()));

        let has_dont_fragment = decoded_attrs.iter().any(|a| matches!(a, TurnAttribute::DontFragment));
        assert!(has_dont_fragment);
    }
}
