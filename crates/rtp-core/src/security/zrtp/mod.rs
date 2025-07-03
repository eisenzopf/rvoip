//! ZRTP (Z Real-time Transport Protocol) implementation
//!
//! This module implements ZRTP key exchange as specified in RFC 6189.
//! ZRTP is a key management protocol that performs a Diffie-Hellman key exchange
//! during call setup to establish SRTP keys, without requiring PKI or pre-provisioned
//! certificates.
//!
//! Reference: https://tools.ietf.org/html/rfc6189

use crate::Error;
use crate::security::SecurityKeyExchange;
use crate::srtp::{SrtpCryptoSuite, SRTP_AES128_CM_SHA1_80};
use crate::srtp::crypto::SrtpCryptoKey;
use rand::{RngCore, rngs::OsRng};
use hmac::{Hmac, Mac};
use sha2::{Sha256, Digest};
use p256::ecdh::EphemeralSecret;
use p256::PublicKey;

pub mod packet;
pub mod hash;

use packet::{ZrtpPacket, ZrtpMessageType, ZrtpVersion};
use hash::ZrtpHash as ZrtpHashFunctions;

/// ZRTP cipher algorithms
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZrtpCipher {
    /// AES-128 Counter Mode
    Aes1,
    /// AES-256 Counter Mode
    Aes3,
    /// Two-Fish 128-bit Counter Mode
    TwoF,
}

/// ZRTP hash algorithms
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZrtpHash {
    /// SHA-256
    S256,
    /// SHA-384
    S384,
}

/// ZRTP authentication tag algorithms
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZrtpAuthTag {
    /// HMAC-SHA1 32-bit
    HS32,
    /// HMAC-SHA1 80-bit
    HS80,
}

/// ZRTP key agreement algorithms
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZrtpKeyAgreement {
    /// Diffie-Hellman 3072-bit
    DH3k,
    /// Diffie-Hellman 4096-bit
    DH4k,
    /// Elliptic Curve P-256
    EC25,
    /// Elliptic Curve P-384
    EC38,
}

/// ZRTP SAS rendering algorithms
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZrtpSasType {
    /// 32-bit SAS using ASCII decimal (base 10)
    B32,
    /// 32-bit SAS using base 32 (5 bits)
    B32E,
}

/// ZRTP configuration
#[derive(Debug, Clone)]
pub struct ZrtpConfig {
    /// Supported cipher algorithms in order of preference
    pub ciphers: Vec<ZrtpCipher>,
    /// Supported hash algorithms in order of preference
    pub hashes: Vec<ZrtpHash>,
    /// Supported authentication tag algorithms in order of preference
    pub auth_tags: Vec<ZrtpAuthTag>,
    /// Supported key agreement algorithms in order of preference
    pub key_agreements: Vec<ZrtpKeyAgreement>,
    /// Supported SAS rendering algorithms in order of preference
    pub sas_types: Vec<ZrtpSasType>,
    /// Client identifier string
    pub client_id: String,
    /// SRTP crypto suite to use
    pub srtp_profile: SrtpCryptoSuite,
}

impl Default for ZrtpConfig {
    fn default() -> Self {
        Self {
            ciphers: vec![ZrtpCipher::Aes1],
            hashes: vec![ZrtpHash::S256],
            auth_tags: vec![ZrtpAuthTag::HS80, ZrtpAuthTag::HS32],
            key_agreements: vec![ZrtpKeyAgreement::EC25, ZrtpKeyAgreement::DH3k],
            sas_types: vec![ZrtpSasType::B32],
            client_id: "RVOIP ZRTP 1.0".to_string(),
            srtp_profile: SRTP_AES128_CM_SHA1_80,
        }
    }
}

/// ZRTP role in key exchange
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZrtpRole {
    /// Initiator (sends Hello first)
    Initiator,
    /// Responder (responds to Hello)
    Responder,
}

/// ZRTP state machine states
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZrtpState {
    /// Initial state
    Initial,
    /// Hello sent
    HelloSent,
    /// Hello received
    HelloReceived,
    /// Hello ACK sent
    HelloAckSent,
    /// Hello ACK received
    HelloAckReceived,
    /// Commit sent
    CommitSent,
    /// Commit received
    CommitReceived,
    /// DH Part 1 sent
    DhPart1Sent,
    /// DH Part 1 received
    DhPart1Received,
    /// DH Part 2 sent
    DhPart2Sent,
    /// DH Part 2 received
    DhPart2Received,
    /// Confirm 1 sent
    Confirm1Sent,
    /// Confirm 1 received
    Confirm1Received,
    /// Confirm 2 sent
    Confirm2Sent,
    /// Confirm 2 received
    Confirm2Received,
    /// Confirm ACK sent
    ConfirmAckSent,
    /// Confirm ACK received
    ConfirmAckReceived,
    /// Completed
    Completed,
    /// Error state
    Error,
}

/// ZRTP key exchange implementation
pub struct Zrtp {
    /// Configuration
    config: ZrtpConfig,
    /// Role in the exchange (initiator or responder)
    role: ZrtpRole,
    /// Current state
    state: ZrtpState,
    /// Local ZRTP ID (ZID)
    zid: [u8; 12],
    /// Remote ZRTP ID (ZID)
    peer_zid: Option<[u8; 12]>,
    /// Local Hello hash
    hello_hash: Option<[u8; 32]>,
    /// Remote Hello hash
    peer_hello_hash: Option<[u8; 32]>,
    /// Selected cipher
    selected_cipher: Option<ZrtpCipher>,
    /// Selected hash
    selected_hash: Option<ZrtpHash>,
    /// Selected authentication tag
    selected_auth_tag: Option<ZrtpAuthTag>,
    /// Selected key agreement
    selected_key_agreement: Option<ZrtpKeyAgreement>,
    /// Selected SAS type
    selected_sas_type: Option<ZrtpSasType>,
    /// Local DH key pair
    dh_key: Option<EphemeralSecret>,
    /// Remote DH public key
    peer_public_key: Option<PublicKey>,
    /// Shared secret
    shared_secret: Option<Vec<u8>>,
    /// SRTP keys for initiator
    srtp_initiator_key: Option<SrtpCryptoKey>,
    /// SRTP keys for responder
    srtp_responder_key: Option<SrtpCryptoKey>,
}

impl Zrtp {
    /// Create a new ZRTP key exchange
    pub fn new(config: ZrtpConfig, role: ZrtpRole) -> Self {
        // Generate random ZID
        let mut zid = [0u8; 12];
        OsRng.fill_bytes(&mut zid);
        
        Self {
            config,
            role,
            state: ZrtpState::Initial,
            zid,
            peer_zid: None,
            hello_hash: None,
            peer_hello_hash: None,
            selected_cipher: None,
            selected_hash: None,
            selected_auth_tag: None,
            selected_key_agreement: None,
            selected_sas_type: None,
            dh_key: None,
            peer_public_key: None,
            shared_secret: None,
            srtp_initiator_key: None,
            srtp_responder_key: None,
        }
    }
    
    /// Create Hello message
    fn create_hello(&mut self) -> Result<ZrtpPacket, Error> {
        // Create ZRTP Hello packet
        let mut hello = ZrtpPacket::new(ZrtpMessageType::Hello);
        
        // Set version
        hello.set_version(ZrtpVersion::V12);
        
        // Set client ID
        hello.set_client_id(&self.config.client_id);
        
        // Set ZID
        hello.set_zid(&self.zid);
        
        // Add supported algorithms
        for cipher in &self.config.ciphers {
            hello.add_cipher(*cipher);
        }
        
        for hash in &self.config.hashes {
            hello.add_hash(*hash);
        }
        
        for auth_tag in &self.config.auth_tags {
            hello.add_auth_tag(*auth_tag);
        }
        
        for key_agreement in &self.config.key_agreements {
            hello.add_key_agreement(*key_agreement);
        }
        
        for sas_type in &self.config.sas_types {
            hello.add_sas_type(*sas_type);
        }
        
        // Calculate Hello hash
        let hello_bytes = hello.to_bytes();
        let mut hasher = Sha256::new();
        hasher.update(&hello_bytes);
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&hasher.finalize());
        self.hello_hash = Some(hash);
        
        // Update state
        self.state = ZrtpState::HelloSent;
        
        Ok(hello)
    }
    
    /// Process Hello message
    fn process_hello(&mut self, packet: &ZrtpPacket) -> Result<ZrtpPacket, Error> {
        if packet.message_type() != ZrtpMessageType::Hello {
            return Err(Error::InvalidMessage("Expected Hello message".into()));
        }
        
        // Extract peer ZID
        let peer_zid = packet.zid().ok_or_else(|| Error::InvalidMessage("Missing ZID in Hello".into()))?;
        self.peer_zid = Some(peer_zid);
        
        // Calculate Hello hash for peer's Hello
        let hello_bytes = packet.to_bytes();
        let mut hasher = Sha256::new();
        hasher.update(&hello_bytes);
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&hasher.finalize());
        self.peer_hello_hash = Some(hash);
        
        // Find common algorithms
        // For simplicity, we'll just select the first ones in common
        
        // Find common cipher
        for our_cipher in &self.config.ciphers {
            if packet.ciphers().contains(our_cipher) {
                self.selected_cipher = Some(*our_cipher);
                break;
            }
        }
        
        // Find common hash
        for our_hash in &self.config.hashes {
            if packet.hashes().contains(our_hash) {
                self.selected_hash = Some(*our_hash);
                break;
            }
        }
        
        // Find common auth tag
        for our_auth_tag in &self.config.auth_tags {
            if packet.auth_tags().contains(our_auth_tag) {
                self.selected_auth_tag = Some(*our_auth_tag);
                break;
            }
        }
        
        // Find common key agreement
        for our_key_agreement in &self.config.key_agreements {
            if packet.key_agreements().contains(our_key_agreement) {
                self.selected_key_agreement = Some(*our_key_agreement);
                break;
            }
        }
        
        // Find common SAS type
        for our_sas_type in &self.config.sas_types {
            if packet.sas_types().contains(our_sas_type) {
                self.selected_sas_type = Some(*our_sas_type);
                break;
            }
        }
        
        // Verify we have common algorithms
        if self.selected_cipher.is_none() ||
           self.selected_hash.is_none() ||
           self.selected_auth_tag.is_none() ||
           self.selected_key_agreement.is_none() ||
           self.selected_sas_type.is_none() {
            return Err(Error::NegotiationFailed("No common algorithms".into()));
        }
        
        // Create Hello ACK
        let hello_ack = ZrtpPacket::new(ZrtpMessageType::HelloAck);
        
        // Update state
        self.state = ZrtpState::HelloReceived;
        
        Ok(hello_ack)
    }
    
    /// Process Hello ACK
    fn process_hello_ack(&mut self, packet: &ZrtpPacket) -> Result<ZrtpPacket, Error> {
        if packet.message_type() != ZrtpMessageType::HelloAck {
            return Err(Error::InvalidMessage("Expected HelloAck message".into()));
        }
        
        // Create Commit
        let mut commit = ZrtpPacket::new(ZrtpMessageType::Commit);
        
        // Set ZID
        commit.set_zid(&self.zid);
        
        // Set selected algorithms
        commit.set_cipher(self.selected_cipher.unwrap());
        commit.set_hash(self.selected_hash.unwrap());
        commit.set_auth_tag(self.selected_auth_tag.unwrap());
        commit.set_key_agreement(self.selected_key_agreement.unwrap());
        commit.set_sas_type(self.selected_sas_type.unwrap());
        
        // Generate DH key pair based on selected key agreement
        if self.selected_key_agreement == Some(ZrtpKeyAgreement::EC25) {
            self.dh_key = Some(EphemeralSecret::random(&mut OsRng));
        } else {
            // Fallback to default EC25 for now
            // In a full implementation, we'd support all key agreement methods
            self.dh_key = Some(EphemeralSecret::random(&mut OsRng));
        }
        
        // Update state
        self.state = ZrtpState::HelloAckReceived;
        
        Ok(commit)
    }
    
    /// Process Commit
    fn process_commit(&mut self, packet: &ZrtpPacket) -> Result<ZrtpPacket, Error> {
        if packet.message_type() != ZrtpMessageType::Commit {
            return Err(Error::InvalidMessage("Expected Commit message".into()));
        }
        
        // Extract selected algorithms
        self.selected_cipher = Some(packet.cipher().unwrap());
        self.selected_hash = Some(packet.hash().unwrap());
        self.selected_auth_tag = Some(packet.auth_tag().unwrap());
        self.selected_key_agreement = Some(packet.key_agreement().unwrap());
        self.selected_sas_type = Some(packet.sas_type().unwrap());
        
        // Generate DH key pair based on selected key agreement
        if self.selected_key_agreement == Some(ZrtpKeyAgreement::EC25) {
            self.dh_key = Some(EphemeralSecret::random(&mut OsRng));
        } else {
            // Fallback to default EC25 for now
            self.dh_key = Some(EphemeralSecret::random(&mut OsRng));
        }
        
        // Create DH Part 1
        let mut dh_part1 = ZrtpPacket::new(ZrtpMessageType::DHPart1);
        
        // Set public key
        if let Some(key) = &self.dh_key {
            let public_key = PublicKey::from(key);
            let public_key_bytes = public_key.to_sec1_bytes();
            dh_part1.set_public_key(&public_key_bytes);
        }
        
        // Update state
        self.state = ZrtpState::CommitReceived;
        
        Ok(dh_part1)
    }
    
    /// Process DH Part 1
    fn process_dh_part1(&mut self, packet: &ZrtpPacket) -> Result<ZrtpPacket, Error> {
        if packet.message_type() != ZrtpMessageType::DHPart1 {
            return Err(Error::InvalidMessage("Expected DHPart1 message".into()));
        }
        
        // Extract peer's public key
        let peer_public_key_bytes = packet.public_key().ok_or_else(|| 
            Error::InvalidMessage("Missing public key in DHPart1".into())
        )?;
        
        // Parse peer's public key
        let peer_public_key = PublicKey::from_sec1_bytes(&peer_public_key_bytes)
            .map_err(|_| Error::CryptoError("Invalid peer public key".into()))?;
        
        self.peer_public_key = Some(peer_public_key);
        
        // Create DH Part 2
        let mut dh_part2 = ZrtpPacket::new(ZrtpMessageType::DHPart2);
        
        // Set public key
        if let Some(key) = &self.dh_key {
            let public_key = PublicKey::from(key);
            let public_key_bytes = public_key.to_sec1_bytes();
            dh_part2.set_public_key(&public_key_bytes);
        }
        
        // Update state
        self.state = ZrtpState::DhPart1Received;
        
        Ok(dh_part2)
    }
    
    /// Process DH Part 2
    fn process_dh_part2(&mut self, packet: &ZrtpPacket) -> Result<ZrtpPacket, Error> {
        if packet.message_type() != ZrtpMessageType::DHPart2 {
            return Err(Error::InvalidMessage("Expected DHPart2 message".into()));
        }
        
        // Extract peer's public key
        let peer_public_key_bytes = packet.public_key().ok_or_else(|| 
            Error::InvalidMessage("Missing public key in DHPart2".into())
        )?;
        
        // Parse peer's public key
        let peer_public_key = PublicKey::from_sec1_bytes(&peer_public_key_bytes)
            .map_err(|_| Error::CryptoError("Invalid peer public key".into()))?;
        
        self.peer_public_key = Some(peer_public_key);
        
        // Compute shared secret
        if let (Some(dh_key), Some(peer_key)) = (&self.dh_key, &self.peer_public_key) {
            let shared_secret = dh_key.diffie_hellman(peer_key);
            let shared_secret_bytes = shared_secret.raw_secret_bytes().to_vec();
            self.shared_secret = Some(shared_secret_bytes.clone());
            
            // Derive SRTP keys
            self.derive_srtp_keys()?;
        } else {
            return Err(Error::CryptoError("Missing keys for DH exchange".into()));
        }
        
        // Create Confirm1
        let mut confirm1 = ZrtpPacket::new(ZrtpMessageType::Confirm1);
        
        // Set ZID
        confirm1.set_zid(&self.zid);
        
        // Calculate HMAC
        if let Some(shared_secret) = &self.shared_secret {
            let mut mac = Hmac::<Sha256>::new_from_slice(shared_secret)
                .map_err(|_| Error::CryptoError("Failed to create HMAC".into()))?;
            
            mac.update(&self.zid);
            if let Some(peer_zid) = &self.peer_zid {
                mac.update(peer_zid);
            }
            
            let mac_result = mac.finalize().into_bytes();
            confirm1.set_mac(&mac_result);
        }
        
        // Update state
        self.state = ZrtpState::DhPart2Received;
        
        Ok(confirm1)
    }
    
    /// Process Confirm1
    fn process_confirm1(&mut self, packet: &ZrtpPacket) -> Result<ZrtpPacket, Error> {
        if packet.message_type() != ZrtpMessageType::Confirm1 {
            return Err(Error::InvalidMessage("Expected Confirm1 message".into()));
        }
        
        // Verify MAC
        if let (Some(shared_secret), Some(mac)) = (&self.shared_secret, packet.mac()) {
            let mut expected_mac = Hmac::<Sha256>::new_from_slice(shared_secret)
                .map_err(|_| Error::CryptoError("Failed to create HMAC".into()))?;
            
            if let Some(peer_zid) = &self.peer_zid {
                expected_mac.update(peer_zid);
            }
            expected_mac.update(&self.zid);
            
            expected_mac.verify_slice(&mac)
                .map_err(|_| Error::AuthenticationFailed("ZRTP MAC verification failed".into()))?;
        } else {
            return Err(Error::CryptoError("Missing shared secret or MAC".into()));
        }
        
        // Create Confirm2
        let mut confirm2 = ZrtpPacket::new(ZrtpMessageType::Confirm2);
        
        // Set ZID
        confirm2.set_zid(&self.zid);
        
        // Calculate HMAC
        if let Some(shared_secret) = &self.shared_secret {
            let mut mac = Hmac::<Sha256>::new_from_slice(shared_secret)
                .map_err(|_| Error::CryptoError("Failed to create HMAC".into()))?;
            
            mac.update(&self.zid);
            if let Some(peer_zid) = &self.peer_zid {
                mac.update(peer_zid);
            }
            
            let mac_result = mac.finalize().into_bytes();
            confirm2.set_mac(&mac_result);
        }
        
        // Update state
        self.state = ZrtpState::Confirm1Received;
        
        Ok(confirm2)
    }
    
    /// Process Confirm2
    fn process_confirm2(&mut self, packet: &ZrtpPacket) -> Result<ZrtpPacket, Error> {
        if packet.message_type() != ZrtpMessageType::Confirm2 {
            return Err(Error::InvalidMessage("Expected Confirm2 message".into()));
        }
        
        // Verify MAC
        if let (Some(shared_secret), Some(mac)) = (&self.shared_secret, packet.mac()) {
            let mut expected_mac = Hmac::<Sha256>::new_from_slice(shared_secret)
                .map_err(|_| Error::CryptoError("Failed to create HMAC".into()))?;
            
            if let Some(peer_zid) = &self.peer_zid {
                expected_mac.update(peer_zid);
            }
            expected_mac.update(&self.zid);
            
            expected_mac.verify_slice(&mac)
                .map_err(|_| Error::AuthenticationFailed("ZRTP MAC verification failed".into()))?;
        } else {
            return Err(Error::CryptoError("Missing shared secret or MAC".into()));
        }
        
        // Create Conf2Ack
        let conf2_ack = ZrtpPacket::new(ZrtpMessageType::Conf2Ack);
        
        // Update state
        self.state = ZrtpState::Confirm2Received;
        
        Ok(conf2_ack)
    }
    
    /// Process Conf2Ack
    fn process_conf2_ack(&mut self, packet: &ZrtpPacket) -> Result<(), Error> {
        if packet.message_type() != ZrtpMessageType::Conf2Ack {
            return Err(Error::InvalidMessage("Expected Conf2Ack message".into()));
        }
        
        // Update state
        self.state = ZrtpState::Completed;
        
        Ok(())
    }
    
    /// Derive SRTP keys from shared secret
    fn derive_srtp_keys(&mut self) -> Result<(), Error> {
        if let Some(shared_secret) = &self.shared_secret {
            // Derive initiator keys
            let mut hasher = Sha256::new();
            hasher.update(b"Initiator SRTP master key");
            hasher.update(shared_secret);
            let initiator_key_hash = hasher.finalize();
            
            let initiator_key = initiator_key_hash[0..16].to_vec();
            
            let mut hasher = Sha256::new();
            hasher.update(b"Initiator SRTP master salt");
            hasher.update(shared_secret);
            let initiator_salt_hash = hasher.finalize();
            
            let initiator_salt = initiator_salt_hash[0..14].to_vec();
            
            self.srtp_initiator_key = Some(SrtpCryptoKey::new(initiator_key, initiator_salt));
            
            // Derive responder keys
            let mut hasher = Sha256::new();
            hasher.update(b"Responder SRTP master key");
            hasher.update(shared_secret);
            let responder_key_hash = hasher.finalize();
            
            let responder_key = responder_key_hash[0..16].to_vec();
            
            let mut hasher = Sha256::new();
            hasher.update(b"Responder SRTP master salt");
            hasher.update(shared_secret);
            let responder_salt_hash = hasher.finalize();
            
            let responder_salt = responder_salt_hash[0..14].to_vec();
            
            self.srtp_responder_key = Some(SrtpCryptoKey::new(responder_key, responder_salt));
            
            Ok(())
        } else {
            Err(Error::CryptoError("Missing shared secret for key derivation".into()))
        }
    }

    /// Generate SAS (Short Authentication String) for user verification
    pub fn generate_sas(&self) -> Result<String, Error> {
        if !self.is_complete() {
            return Err(Error::InvalidState("ZRTP exchange not complete".into()));
        }

        let shared_secret = self.shared_secret.as_ref()
            .ok_or_else(|| Error::CryptoError("No shared secret available".into()))?;

        let hello_hash_i = self.hello_hash.as_ref()
            .ok_or_else(|| Error::CryptoError("Missing local Hello hash".into()))?;

        let hello_hash_r = self.peer_hello_hash.as_ref()
            .ok_or_else(|| Error::CryptoError("Missing peer Hello hash".into()))?;

        // ZRTP SAS uses consistent ordering: shared_secret || Hello_hash_I || Hello_hash_R
        // Where I is always initiator and R is always responder (not local/peer)
        let mut sas_input = Vec::new();
        sas_input.extend_from_slice(shared_secret);
        
        // Use consistent ordering based on role
        match self.role {
            ZrtpRole::Initiator => {
                // For initiator: local=I, peer=R
                sas_input.extend_from_slice(hello_hash_i);  // Our hello = initiator
                sas_input.extend_from_slice(hello_hash_r);  // Peer hello = responder
            },
            ZrtpRole::Responder => {
                // For responder: peer=I, local=R
                sas_input.extend_from_slice(hello_hash_r);  // Peer hello = initiator  
                sas_input.extend_from_slice(hello_hash_i);  // Our hello = responder
            },
        }

        // Hash the SAS input
        let mut hasher = Sha256::new();
        hasher.update(&sas_input);
        let sas_hash = hasher.finalize();

        // Generate SAS based on selected type
        let sas_type = self.selected_sas_type
            .ok_or_else(|| Error::InvalidState("No SAS type selected".into()))?;

        match sas_type {
            ZrtpSasType::B32 => {
                // Base 32 encoding - 4 characters, 20 bits from hash
                let sas_value = u32::from_be_bytes([
                    sas_hash[0], sas_hash[1], sas_hash[2], sas_hash[3]
                ]) & 0x000FFFFF; // 20 bits

                // Convert to base 32 (A-Z, 2-7)
                let charset = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
                let mut sas = String::new();
                let mut value = sas_value;
                
                for _ in 0..4 {
                    let index = (value & 0x1F) as usize;
                    sas.insert(0, charset.chars().nth(index).unwrap());
                    value >>= 5;
                }
                
                Ok(sas)
            },
            ZrtpSasType::B32E => {
                // Base 32 extended - use first 20 bits for 4-character SAS
                let sas_value = u32::from_be_bytes([
                    sas_hash[0], sas_hash[1], sas_hash[2], sas_hash[3]
                ]) & 0x000FFFFF; // 20 bits

                // Convert to decimal for user display
                Ok(format!("{:04}", sas_value % 10000))
            },
        }
    }

    /// Verify SAS matches what the user sees on both endpoints
    pub fn verify_sas(&self, user_sas: &str) -> Result<bool, Error> {
        let generated_sas = self.generate_sas()?;
        Ok(generated_sas.eq_ignore_ascii_case(user_sas))
    }

    /// Get human-readable SAS display for the user
    pub fn get_sas_display(&self) -> Result<String, Error> {
        let sas = self.generate_sas()?;
        let sas_type = self.selected_sas_type
            .ok_or_else(|| Error::InvalidState("No SAS type selected".into()))?;

        match sas_type {
            ZrtpSasType::B32 => {
                Ok(format!("SAS: {} (Read aloud: \"{}\")", sas, 
                    sas.chars().map(|c| c.to_string()).collect::<Vec<String>>().join(" ")))
            },
            ZrtpSasType::B32E => {
                Ok(format!("SAS: {} (Read as: \"{}\")", sas,
                    sas.chars().map(|c| c.to_string()).collect::<Vec<String>>().join(" ")))
            },
        }
    }
}

impl SecurityKeyExchange for Zrtp {
    fn init(&mut self) -> Result<(), Error> {
        match self.role {
            ZrtpRole::Initiator => {
                // Initiator creates and sends Hello
                let _ = self.create_hello()?;
                Ok(())
            },
            ZrtpRole::Responder => {
                // Responder waits for Hello
                Ok(())
            },
        }
    }
    
    fn process_message(&mut self, message: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        // Parse ZRTP packet
        let packet = ZrtpPacket::parse(message)
            .map_err(|_| Error::ParseError("Failed to parse ZRTP packet".into()))?;
        
        let response = match (self.role, &self.state) {
            (ZrtpRole::Initiator, ZrtpState::HelloSent) => {
                // Process Hello response
                if packet.message_type() == ZrtpMessageType::Hello {
                    // Process peer's Hello
                    let hello_ack = self.process_hello(&packet)?;
                    self.state = ZrtpState::HelloAckSent;
                    Ok(Some(hello_ack.to_bytes()))
                } else {
                    Err(Error::InvalidMessage("Expected Hello message".into()))
                }
            },
            (ZrtpRole::Initiator, ZrtpState::HelloAckSent) => {
                // Process Commit from responder
                let dh_part1 = self.process_commit(&packet)?;
                self.state = ZrtpState::DhPart1Sent;
                Ok(Some(dh_part1.to_bytes()))
            },
            (ZrtpRole::Initiator, ZrtpState::DhPart1Sent) => {
                // Process DH Part 2 from responder
                let confirm1 = self.process_dh_part2(&packet)?;
                self.state = ZrtpState::Confirm1Sent;
                Ok(Some(confirm1.to_bytes()))
            },
            (ZrtpRole::Initiator, ZrtpState::Confirm1Sent) => {
                // Process Confirm 2 from responder
                let conf2_ack = self.process_confirm2(&packet)?;
                self.state = ZrtpState::ConfirmAckSent;
                Ok(Some(conf2_ack.to_bytes()))
            },
            (ZrtpRole::Responder, ZrtpState::Initial) => {
                // Process Hello from initiator
                let hello = self.create_hello()?;
                let hello_bytes = hello.to_bytes();
                
                // Also process Hello message from initiator
                let hello_ack = self.process_hello(&packet)?;
                let hello_ack_bytes = hello_ack.to_bytes();
                
                // Combine responses
                let mut combined = Vec::new();
                combined.extend_from_slice(&hello_bytes);
                combined.extend_from_slice(&hello_ack_bytes);
                
                Ok(Some(combined))
            },
            (ZrtpRole::Responder, ZrtpState::HelloReceived) => {
                // Process Hello ACK from initiator
                let commit = self.process_hello_ack(&packet)?;
                self.state = ZrtpState::CommitSent;
                Ok(Some(commit.to_bytes()))
            },
            (ZrtpRole::Responder, ZrtpState::CommitSent) => {
                // Process DH Part 1 from initiator
                let dh_part2 = self.process_dh_part1(&packet)?;
                self.state = ZrtpState::DhPart2Sent;
                Ok(Some(dh_part2.to_bytes()))
            },
            (ZrtpRole::Responder, ZrtpState::DhPart2Sent) => {
                // Process Confirm 1 from initiator
                let confirm2 = self.process_confirm1(&packet)?;
                self.state = ZrtpState::Confirm2Sent;
                Ok(Some(confirm2.to_bytes()))
            },
            (ZrtpRole::Responder, ZrtpState::Confirm2Sent) => {
                // Process Confirm ACK from initiator
                self.process_conf2_ack(&packet)?;
                self.state = ZrtpState::Completed;
                Ok(None)
            },
            _ => Err(Error::InvalidState(format!("Invalid state {:?} for message processing", self.state))),
        };
        
        response
    }
    
    fn get_srtp_key(&self) -> Option<SrtpCryptoKey> {
        match self.role {
            ZrtpRole::Initiator => self.srtp_initiator_key.clone(),
            ZrtpRole::Responder => self.srtp_responder_key.clone(),
        }
    }
    
    fn get_srtp_suite(&self) -> Option<SrtpCryptoSuite> {
        Some(self.config.srtp_profile.clone())
    }
    
    fn is_complete(&self) -> bool {
        self.state == ZrtpState::Completed
    }
}

#[cfg(test)]
mod tests; 