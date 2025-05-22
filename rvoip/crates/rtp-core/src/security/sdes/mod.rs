//! SDES (Security DEScriptions) implementation
//!
//! This module implements the Security Descriptions for SDP as defined in RFC 4568.
//! SDES allows keys and related information to be transported over SDP.
//!
//! Reference: https://tools.ietf.org/html/rfc4568

use crate::Error;
use crate::security::SecurityKeyExchange;
use crate::srtp::{SrtpCryptoSuite, SRTP_AES128_CM_SHA1_80, SRTP_AES128_CM_SHA1_32};
use crate::srtp::crypto::SrtpCryptoKey;
use rand::{RngCore, rngs::OsRng};
use base64;

/// SDES crypto attribute representation
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdesCryptoAttribute {
    /// Tag (unique for each crypto attribute)
    pub tag: u32,
    /// Crypto-suite (e.g., "AES_CM_128_HMAC_SHA1_80")
    pub crypto_suite: String,
    /// Key method (always "inline" for SDES)
    pub key_method: String,
    /// Key information
    pub key_info: String,
    /// Optional session parameters
    pub session_params: Vec<String>,
}

impl SdesCryptoAttribute {
    /// Create a new SDES crypto attribute
    pub fn new(tag: u32, crypto_suite: &str, key_info: &str) -> Self {
        Self {
            tag,
            crypto_suite: crypto_suite.to_string(),
            key_method: "inline".to_string(),
            key_info: key_info.to_string(),
            session_params: Vec::new(),
        }
    }
    
    /// Add a session parameter
    pub fn add_session_param(&mut self, param: &str) {
        self.session_params.push(param.to_string());
    }
    
    /// Format the crypto attribute as a string for SDP
    pub fn to_string(&self) -> String {
        let mut result = format!("{} {} {}:{}", self.tag, self.crypto_suite, self.key_method, self.key_info);
        
        // Add session parameters
        if !self.session_params.is_empty() {
            result.push_str(" ");
            result.push_str(&self.session_params.join(";"));
        }
        
        result
    }
    
    /// Parse a crypto attribute from a string
    pub fn parse(s: &str) -> Result<Self, Error> {
        let parts: Vec<&str> = s.split_whitespace().collect();
        
        if parts.len() < 3 {
            return Err(Error::ParseError("Invalid SDES crypto attribute format".into()));
        }
        
        // Parse tag
        let tag = parts[0].parse::<u32>()
            .map_err(|_| Error::ParseError("Invalid tag in SDES crypto attribute".into()))?;
        
        // Parse crypto suite
        let crypto_suite = parts[1].to_string();
        
        // Parse key method and key info
        let key_parts: Vec<&str> = parts[2].split(':').collect();
        if key_parts.len() != 2 {
            return Err(Error::ParseError("Invalid key format in SDES crypto attribute".into()));
        }
        
        let key_method = key_parts[0].to_string();
        let key_info = key_parts[1].to_string();
        
        if key_method != "inline" {
            return Err(Error::ParseError("Only 'inline' key method is supported".into()));
        }
        
        // Parse session parameters
        let mut session_params = Vec::new();
        if parts.len() > 3 {
            let params_str = parts[3..].join(" ");
            for param in params_str.split(';') {
                session_params.push(param.trim().to_string());
            }
        }
        
        Ok(Self {
            tag,
            crypto_suite,
            key_method,
            key_info,
            session_params,
        })
    }
}

/// SDES role in key exchange
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdesRole {
    /// Offerer (creates initial crypto attributes)
    Offerer,
    /// Answerer (responds to crypto attributes)
    Answerer,
}

/// SDES state
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SdesState {
    /// Initial state
    Initial,
    /// Offer sent
    OfferSent,
    /// Answer received
    AnswerReceived,
    /// Completed
    Completed,
}

/// SDES configuration
#[derive(Debug, Clone)]
pub struct SdesConfig {
    /// List of supported SRTP crypto suites in order of preference
    pub crypto_suites: Vec<SrtpCryptoSuite>,
    /// Number of crypto attributes to include in offer
    pub offer_count: usize,
}

impl Default for SdesConfig {
    fn default() -> Self {
        Self {
            crypto_suites: vec![SRTP_AES128_CM_SHA1_80, SRTP_AES128_CM_SHA1_32],
            offer_count: 2,
        }
    }
}

/// SDES implementation
pub struct Sdes {
    /// Configuration
    config: SdesConfig,
    /// Role (offerer or answerer)
    role: SdesRole,
    /// Current state
    state: SdesState,
    /// Local crypto attributes
    local_attrs: Vec<SdesCryptoAttribute>,
    /// Remote crypto attributes
    remote_attrs: Vec<SdesCryptoAttribute>,
    /// Selected crypto attribute
    selected_attr: Option<SdesCryptoAttribute>,
    /// Negotiated SRTP crypto key
    srtp_key: Option<SrtpCryptoKey>,
    /// Negotiated SRTP crypto suite
    srtp_suite: Option<SrtpCryptoSuite>,
}

impl Sdes {
    /// Create a new SDES instance
    pub fn new(config: SdesConfig, role: SdesRole) -> Self {
        Self {
            config,
            role,
            state: SdesState::Initial,
            local_attrs: Vec::new(),
            remote_attrs: Vec::new(),
            selected_attr: None,
            srtp_key: None,
            srtp_suite: None,
        }
    }
    
    /// Create a crypto attribute for a specific crypto suite
    fn create_crypto_attribute(&self, tag: u32, suite: &SrtpCryptoSuite) -> Result<(SdesCryptoAttribute, SrtpCryptoKey), Error> {
        // Map SRTP crypto suite to SDES crypto suite string
        let crypto_suite_str = match (suite.encryption, suite.authentication) {
            (crate::srtp::SrtpEncryptionAlgorithm::AesCm, crate::srtp::SrtpAuthenticationAlgorithm::HmacSha1_80) => {
                "AES_CM_128_HMAC_SHA1_80"
            },
            (crate::srtp::SrtpEncryptionAlgorithm::AesCm, crate::srtp::SrtpAuthenticationAlgorithm::HmacSha1_32) => {
                "AES_CM_128_HMAC_SHA1_32"
            },
            _ => return Err(Error::UnsupportedFeature("Unsupported SRTP crypto suite for SDES".into())),
        };
        
        // Generate random key
        let mut key = vec![0u8; 16]; // 128-bit AES key
        OsRng.fill_bytes(&mut key);
        
        // Generate random salt
        let mut salt = vec![0u8; 14]; // 112-bit salt
        OsRng.fill_bytes(&mut salt);
        
        // Combine key and salt
        let mut keysalt = Vec::with_capacity(key.len() + salt.len());
        keysalt.extend_from_slice(&key);
        keysalt.extend_from_slice(&salt);
        
        // Base64 encode key+salt
        let key_info = base64::encode(&keysalt);
        
        // Create crypto attribute
        let mut attr = SdesCryptoAttribute::new(tag, crypto_suite_str, &key_info);
        
        // Create SRTP key for later use
        let srtp_key = SrtpCryptoKey::new(key, salt);
        
        Ok((attr, srtp_key))
    }
    
    /// Create offer crypto attributes
    fn create_offer(&mut self) -> Result<Vec<String>, Error> {
        if self.role != SdesRole::Offerer {
            return Err(Error::InvalidState("Only offerer can create offer".into()));
        }
        
        let mut offer = Vec::new();
        
        // Create crypto attributes for each supported crypto suite
        for (i, suite) in self.config.crypto_suites.iter().take(self.config.offer_count).enumerate() {
            let tag = (i + 1) as u32;
            let (attr, srtp_key) = self.create_crypto_attribute(tag, suite)?;
            
            // Store local attribute
            self.local_attrs.push(attr.clone());
            
            // Add to offer
            offer.push(format!("a=crypto:{}", attr.to_string()));
            
            // Save key if it's the first one (the default)
            if i == 0 {
                self.srtp_key = Some(srtp_key);
                self.srtp_suite = Some(suite.clone());
            }
        }
        
        // Update state
        self.state = SdesState::OfferSent;
        
        Ok(offer)
    }
    
    /// Parse offer and create answer
    fn create_answer(&mut self, offer: &[String]) -> Result<Vec<String>, Error> {
        if self.role != SdesRole::Answerer {
            return Err(Error::InvalidState("Only answerer can create answer".into()));
        }
        
        // Parse offer
        for line in offer {
            if line.starts_with("a=crypto:") {
                let attr_str = line.trim_start_matches("a=crypto:");
                let attr = SdesCryptoAttribute::parse(attr_str)?;
                
                // Store remote attribute
                self.remote_attrs.push(attr);
            }
        }
        
        if self.remote_attrs.is_empty() {
            return Err(Error::InvalidMessage("No crypto attributes in offer".into()));
        }
        
        // Select the first supported crypto attribute
        let selected = &self.remote_attrs[0];
        
        // Map SDES crypto suite to SRTP crypto suite
        let srtp_suite = match selected.crypto_suite.as_str() {
            "AES_CM_128_HMAC_SHA1_80" => SRTP_AES128_CM_SHA1_80,
            "AES_CM_128_HMAC_SHA1_32" => SRTP_AES128_CM_SHA1_32,
            _ => return Err(Error::UnsupportedFeature(format!("Unsupported crypto suite: {}", selected.crypto_suite))),
        };
        
        // Parse key info
        let key_info = selected.key_info.as_str();
        
        // Base64 decode key+salt
        let keysalt = base64::decode(key_info)
            .map_err(|_| Error::ParseError("Invalid Base64 encoding in key info".into()))?;
        
        if keysalt.len() < 30 {
            return Err(Error::ParseError("Key info too short".into()));
        }
        
        // Split key and salt
        let key = keysalt[0..16].to_vec();
        let salt = keysalt[16..30].to_vec();
        
        // Store key for later use
        self.srtp_key = Some(SrtpCryptoKey::new(key, salt));
        self.srtp_suite = Some(srtp_suite);
        self.selected_attr = Some(selected.clone());
        
        // Create answer
        let mut answer = Vec::new();
        answer.push(format!("a=crypto:{}", selected.to_string()));
        
        // Update state
        self.state = SdesState::Completed;
        
        Ok(answer)
    }
    
    /// Process answer
    fn process_answer(&mut self, answer: &[String]) -> Result<(), Error> {
        if self.role != SdesRole::Offerer {
            return Err(Error::InvalidState("Only offerer can process answer".into()));
        }
        
        // Parse answer
        let mut selected_attr = None;
        
        for line in answer {
            if line.starts_with("a=crypto:") {
                let attr_str = line.trim_start_matches("a=crypto:");
                let attr = SdesCryptoAttribute::parse(attr_str)?;
                
                // Store remote attribute
                self.remote_attrs.push(attr.clone());
                
                // This is the selected attribute
                selected_attr = Some(attr);
                break;
            }
        }
        
        if selected_attr.is_none() {
            return Err(Error::InvalidMessage("No crypto attributes in answer".into()));
        }
        
        let selected = selected_attr.unwrap();
        
        // Find matching local attribute by tag
        let local_attr = self.local_attrs.iter().find(|a| a.tag == selected.tag);
        
        if local_attr.is_none() {
            return Err(Error::InvalidMessage(format!("No matching local attribute for tag {}", selected.tag)));
        }
        
        // Store selected attribute
        self.selected_attr = Some(selected);
        
        // Update state
        self.state = SdesState::Completed;
        
        Ok(())
    }
}

impl SecurityKeyExchange for Sdes {
    fn init(&mut self) -> Result<(), Error> {
        // No initialization needed for SDES
        Ok(())
    }
    
    fn process_message(&mut self, message: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        // SDES messages are SDP lines
        let message_str = std::str::from_utf8(message)
            .map_err(|_| Error::ParseError("Invalid UTF-8 in SDES message".into()))?;
        
        let lines: Vec<String> = message_str.lines()
            .map(|s| s.trim().to_string())
            .collect();
        
        match (self.role, &self.state) {
            (SdesRole::Offerer, SdesState::Initial) => {
                // Create offer
                let offer = self.create_offer()?;
                let offer_str = offer.join("\r\n");
                Ok(Some(offer_str.into_bytes()))
            },
            (SdesRole::Offerer, SdesState::OfferSent) => {
                // Process answer
                self.process_answer(&lines)?;
                Ok(None)
            },
            (SdesRole::Answerer, SdesState::Initial) => {
                // Create answer
                let answer = self.create_answer(&lines)?;
                let answer_str = answer.join("\r\n");
                Ok(Some(answer_str.into_bytes()))
            },
            _ => Err(Error::InvalidState("Invalid state for message processing".into())),
        }
    }
    
    fn get_srtp_key(&self) -> Option<SrtpCryptoKey> {
        self.srtp_key.clone()
    }
    
    fn get_srtp_suite(&self) -> Option<SrtpCryptoSuite> {
        self.srtp_suite.clone()
    }
    
    fn is_complete(&self) -> bool {
        self.state == SdesState::Completed
    }
}

#[cfg(test)]
mod tests; 