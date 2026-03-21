//! Long-term credential mechanism for TURN authentication (RFC 5389 Section 10.2).
//!
//! The long-term credential key is derived as:
//! ```text
//!   key = MD5(username ":" realm ":" password)
//! ```
//!
//! MESSAGE-INTEGRITY is then computed as HMAC-SHA1 over the STUN message
//! (up to and including the dummy MESSAGE-INTEGRITY length) using this key.

use hmac::{Hmac, Mac};
use md5::{Md5, Digest};
use sha1::Sha1;

use crate::Error;
use crate::stun::message::{MAGIC_COOKIE, HEADER_SIZE, ATTR_MESSAGE_INTEGRITY};

type HmacSha1 = Hmac<Sha1>;

/// Long-term credential state for TURN authentication.
#[derive(Debug, Clone)]
pub struct LongTermCredentials {
    /// The username.
    username: String,
    /// The password.
    password: String,
    /// The realm (provided by the server in a 401 response).
    realm: Option<String>,
    /// The nonce (provided by the server in a 401 response).
    nonce: Option<String>,
}

impl LongTermCredentials {
    /// Create new credentials with the given username and password.
    pub fn new(username: String, password: String) -> Self {
        Self {
            username,
            password,
            realm: None,
            nonce: None,
        }
    }

    /// The username.
    pub fn username(&self) -> &str {
        &self.username
    }

    /// The current realm (set after receiving a 401 from the server).
    pub fn realm(&self) -> Option<&str> {
        self.realm.as_deref()
    }

    /// The current nonce (set after receiving a 401 from the server).
    pub fn nonce(&self) -> Option<&str> {
        self.nonce.as_deref()
    }

    /// Update the realm and nonce from a 401 Unauthorized response.
    pub fn set_challenge(&mut self, realm: String, nonce: String) {
        self.realm = Some(realm);
        self.nonce = Some(nonce);
    }

    /// Whether we have received a challenge (realm + nonce) from the server.
    pub fn has_challenge(&self) -> bool {
        self.realm.is_some() && self.nonce.is_some()
    }

    /// Derive the long-term credential key: MD5(username:realm:password).
    ///
    /// Returns an error if realm has not been set yet.
    pub fn derive_key(&self) -> Result<[u8; 16], Error> {
        let realm = self.realm.as_deref().ok_or_else(|| {
            Error::TurnError("cannot derive key: realm not set (no 401 challenge received)".into())
        })?;

        let input = format!("{}:{}:{}", self.username, realm, self.password);
        let mut hasher = Md5::new();
        hasher.update(input.as_bytes());
        let result = hasher.finalize();

        let mut key = [0u8; 16];
        key.copy_from_slice(&result);
        Ok(key)
    }

    /// Compute the MESSAGE-INTEGRITY HMAC-SHA1 for a STUN/TURN message.
    ///
    /// The `message_bytes` should be the complete STUN message up to (but not
    /// including) the MESSAGE-INTEGRITY attribute. This function:
    /// 1. Adjusts the message length in the header to include MESSAGE-INTEGRITY.
    /// 2. Computes HMAC-SHA1 over the adjusted message.
    ///
    /// Returns the 20-byte HMAC-SHA1 value.
    pub fn compute_message_integrity(&self, message_bytes: &[u8]) -> Result<[u8; 20], Error> {
        let key = self.derive_key()?;

        // MESSAGE-INTEGRITY attribute is 24 bytes: 4-byte header + 20-byte HMAC
        let integrity_attr_size: u16 = 24;

        // Make a mutable copy to adjust the message length field
        let mut adjusted = message_bytes.to_vec();

        // The message length field (bytes 2-3) should account for attributes
        // already present PLUS the MESSAGE-INTEGRITY attribute we are about to add.
        if adjusted.len() >= 4 {
            let current_len = u16::from_be_bytes([adjusted[2], adjusted[3]]);
            let new_len = current_len + integrity_attr_size;
            let new_len_bytes = new_len.to_be_bytes();
            adjusted[2] = new_len_bytes[0];
            adjusted[3] = new_len_bytes[1];
        }

        let mut mac = HmacSha1::new_from_slice(&key)
            .map_err(|e| Error::TurnError(format!("HMAC init failed: {e}")))?;
        mac.update(&adjusted);
        let result = mac.finalize();

        let mut hmac_bytes = [0u8; 20];
        hmac_bytes.copy_from_slice(&result.into_bytes());
        Ok(hmac_bytes)
    }

    /// Append MESSAGE-INTEGRITY attribute to an existing STUN/TURN message buffer.
    ///
    /// The buffer must contain a valid STUN message (header + attributes).
    /// This function computes the HMAC-SHA1 and appends the MESSAGE-INTEGRITY
    /// attribute, updating the message length in the header.
    pub fn sign_message(&self, message: &mut Vec<u8>) -> Result<(), Error> {
        let hmac_val = self.compute_message_integrity(message)?;

        // Append the MESSAGE-INTEGRITY attribute
        let attr_type_bytes = ATTR_MESSAGE_INTEGRITY.to_be_bytes();
        let attr_len_bytes = 20u16.to_be_bytes();
        message.extend_from_slice(&attr_type_bytes);
        message.extend_from_slice(&attr_len_bytes);
        message.extend_from_slice(&hmac_val);

        // Update message length in header (bytes 2-3)
        if message.len() >= 4 {
            let body_len = (message.len() - HEADER_SIZE) as u16;
            let len_bytes = body_len.to_be_bytes();
            message[2] = len_bytes[0];
            message[3] = len_bytes[1];
        }

        Ok(())
    }

    /// Verify the MESSAGE-INTEGRITY of a received STUN/TURN message.
    ///
    /// `full_message` is the complete received message including the
    /// MESSAGE-INTEGRITY attribute. Returns `true` if the HMAC matches.
    pub fn verify_message_integrity(&self, full_message: &[u8]) -> Result<bool, Error> {
        let key = self.derive_key()?;

        // Find MESSAGE-INTEGRITY attribute by scanning from the end
        // MESSAGE-INTEGRITY is always 24 bytes: 4 header + 20 HMAC
        if full_message.len() < HEADER_SIZE + 24 {
            return Ok(false);
        }

        // Walk attributes to find MESSAGE-INTEGRITY offset
        let msg_len = u16::from_be_bytes([full_message[2], full_message[3]]) as usize;
        if full_message.len() < HEADER_SIZE + msg_len {
            return Ok(false);
        }

        let mut offset = HEADER_SIZE;
        let mut integrity_offset = None;
        let mut integrity_value = None;

        while offset + 4 <= HEADER_SIZE + msg_len {
            let attr_type = u16::from_be_bytes([full_message[offset], full_message[offset + 1]]);
            let attr_len = u16::from_be_bytes([full_message[offset + 2], full_message[offset + 3]]) as usize;

            if attr_type == ATTR_MESSAGE_INTEGRITY && attr_len == 20 && offset + 4 + 20 <= full_message.len() {
                integrity_offset = Some(offset);
                let mut val = [0u8; 20];
                val.copy_from_slice(&full_message[offset + 4..offset + 24]);
                integrity_value = Some(val);
                break;
            }

            let padded = attr_len + ((4 - (attr_len % 4)) % 4);
            offset += 4 + padded;
        }

        let (int_offset, received_hmac) = match (integrity_offset, integrity_value) {
            (Some(o), Some(v)) => (o, v),
            _ => return Ok(false),
        };

        // Build the message to HMAC: everything up to MESSAGE-INTEGRITY,
        // with the length adjusted to include MESSAGE-INTEGRITY (24 bytes).
        let mut to_hash = full_message[..int_offset].to_vec();

        // Adjust length: bytes up to MESSAGE-INTEGRITY offset minus header,
        // plus 24 for the MESSAGE-INTEGRITY attribute itself.
        let adjusted_len = (int_offset - HEADER_SIZE + 24) as u16;
        let len_bytes = adjusted_len.to_be_bytes();
        if to_hash.len() >= 4 {
            to_hash[2] = len_bytes[0];
            to_hash[3] = len_bytes[1];
        }

        let mut mac = HmacSha1::new_from_slice(&key)
            .map_err(|e| Error::TurnError(format!("HMAC init failed: {e}")))?;
        mac.update(&to_hash);

        let expected = mac.finalize().into_bytes();
        let mut expected_bytes = [0u8; 20];
        expected_bytes.copy_from_slice(&expected);

        // Constant-time comparison
        Ok(expected_bytes == received_hmac)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stun::message::{TransactionId, MAGIC_COOKIE, HEADER_SIZE};
    use crate::turn::message::{
        build_turn_message, ALLOCATE_REQUEST, TurnAttribute, TRANSPORT_UDP,
    };

    #[test]
    fn test_derive_key() {
        let mut creds = LongTermCredentials::new("user".into(), "pass".into());
        // Without realm, should fail
        assert!(creds.derive_key().is_err());

        creds.set_challenge("example.com".into(), "nonce123".into());
        let key = creds.derive_key().unwrap_or_else(|e| panic!("derive_key: {e}"));
        assert_eq!(key.len(), 16);

        // Verify it's MD5("user:example.com:pass")
        let mut hasher = Md5::new();
        hasher.update(b"user:example.com:pass");
        let expected = hasher.finalize();
        assert_eq!(&key[..], &expected[..]);
    }

    #[test]
    fn test_has_challenge() {
        let mut creds = LongTermCredentials::new("u".into(), "p".into());
        assert!(!creds.has_challenge());

        creds.set_challenge("realm".into(), "nonce".into());
        assert!(creds.has_challenge());
    }

    #[test]
    fn test_sign_and_verify_message() {
        let mut creds = LongTermCredentials::new("testuser".into(), "testpass".into());
        creds.set_challenge("testrealm".into(), "testnonce".into());

        let txn_id = TransactionId::random();
        let attrs = vec![
            TurnAttribute::RequestedTransport(TRANSPORT_UDP),
            TurnAttribute::Username("testuser".into()),
            TurnAttribute::Realm("testrealm".into()),
            TurnAttribute::Nonce("testnonce".into()),
        ];

        let mut message = build_turn_message(ALLOCATE_REQUEST, &txn_id, &attrs);
        let original_len = message.len();

        creds.sign_message(&mut message).unwrap_or_else(|e| panic!("sign: {e}"));

        // Message should be 24 bytes longer (4 header + 20 HMAC)
        assert_eq!(message.len(), original_len + 24);

        // Verify the signature
        let valid = creds.verify_message_integrity(&message)
            .unwrap_or_else(|e| panic!("verify: {e}"));
        assert!(valid, "MESSAGE-INTEGRITY verification failed");
    }

    #[test]
    fn test_verify_tampered_message() {
        let mut creds = LongTermCredentials::new("user".into(), "pass".into());
        creds.set_challenge("realm".into(), "nonce".into());

        let txn_id = TransactionId::random();
        let attrs = vec![TurnAttribute::RequestedTransport(TRANSPORT_UDP)];

        let mut message = build_turn_message(ALLOCATE_REQUEST, &txn_id, &attrs);
        creds.sign_message(&mut message).unwrap_or_else(|e| panic!("sign: {e}"));

        // Tamper with the message body
        if message.len() > HEADER_SIZE + 2 {
            message[HEADER_SIZE + 2] ^= 0xFF;
        }

        let valid = creds.verify_message_integrity(&message)
            .unwrap_or_else(|e| panic!("verify: {e}"));
        assert!(!valid, "tampered message should not verify");
    }

    #[test]
    fn test_wrong_password_fails_verification() {
        let mut creds1 = LongTermCredentials::new("user".into(), "correct_pass".into());
        creds1.set_challenge("realm".into(), "nonce".into());

        let mut creds2 = LongTermCredentials::new("user".into(), "wrong_pass".into());
        creds2.set_challenge("realm".into(), "nonce".into());

        let txn_id = TransactionId::random();
        let attrs = vec![TurnAttribute::RequestedTransport(TRANSPORT_UDP)];

        let mut message = build_turn_message(ALLOCATE_REQUEST, &txn_id, &attrs);
        creds1.sign_message(&mut message).unwrap_or_else(|e| panic!("sign: {e}"));

        let valid = creds2.verify_message_integrity(&message)
            .unwrap_or_else(|e| panic!("verify: {e}"));
        assert!(!valid, "wrong password should not verify");
    }
}
