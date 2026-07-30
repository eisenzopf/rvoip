//! SDES (Security DEScriptions) implementation
//!
//! This module implements the Security Descriptions for SDP as defined in RFC 4568.
//! SDES allows keys and related information to be transported over SDP.
//!
//! Reference: <https://tools.ietf.org/html/rfc4568>

use crate::security::SecurityKeyExchange;
use crate::srtp::crypto::SrtpCryptoKey;
use crate::srtp::{SrtpCryptoSuite, SRTP_AES128_CM_SHA1_32, SRTP_AES128_CM_SHA1_80};
use crate::Error;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rand::{rngs::OsRng, RngCore};

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
        let mut result = format!(
            "{} {} {}:{}",
            self.tag, self.crypto_suite, self.key_method, self.key_info
        );

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
            return Err(Error::ParseError(
                "Invalid SDES crypto attribute format".into(),
            ));
        }

        // Parse tag
        let tag = parts[0]
            .parse::<u32>()
            .map_err(|_| Error::ParseError("Invalid tag in SDES crypto attribute".into()))?;

        // Parse crypto suite
        let crypto_suite = parts[1].to_string();

        // Parse key method and key info
        let (key_method, key_info) = parts[2].split_once(':').ok_or_else(|| {
            Error::ParseError("Invalid key format in SDES crypto attribute".into())
        })?;
        let key_method = key_method.to_string();
        let key_info = key_info.to_string();

        if key_method != "inline" {
            return Err(Error::ParseError(
                "Only 'inline' key method is supported".into(),
            ));
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

/// Directional key material produced by an SDES offer/answer exchange.
#[derive(Debug, Clone)]
pub struct SdesKeyMaterial {
    /// Key this endpoint uses to protect outbound RTP and RTCP.
    pub local_tx: SrtpCryptoKey,
    /// Peer key this endpoint uses to unprotect inbound RTP and RTCP.
    pub remote_rx: SrtpCryptoKey,
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
    /// Local transmit key advertised by this endpoint.
    local_srtp_key: Option<SrtpCryptoKey>,
    /// Remote transmit key used by this endpoint for inbound unprotection.
    remote_srtp_key: Option<SrtpCryptoKey>,
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
            local_srtp_key: None,
            remote_srtp_key: None,
            srtp_suite: None,
        }
    }

    /// Return the RFC 4568 name for an SDES suite implemented by this module.
    fn suite_name(suite: &SrtpCryptoSuite) -> Result<&'static str, Error> {
        suite.validate()?;
        if suite == &SRTP_AES128_CM_SHA1_80 {
            Ok("AES_CM_128_HMAC_SHA1_80")
        } else if suite == &SRTP_AES128_CM_SHA1_32 {
            Ok("AES_CM_128_HMAC_SHA1_32")
        } else {
            Err(Error::UnsupportedFeature(format!(
                "SRTP suite {suite:?} is not implemented for SDES"
            )))
        }
    }

    /// Map an RFC 4568 suite name to an implemented SRTP suite.
    fn suite_from_name(name: &str) -> Option<SrtpCryptoSuite> {
        match name {
            "AES_CM_128_HMAC_SHA1_80" => Some(SRTP_AES128_CM_SHA1_80),
            "AES_CM_128_HMAC_SHA1_32" => Some(SRTP_AES128_CM_SHA1_32),
            _ => None,
        }
    }

    /// Reject RFC 4568 extensions that this implementation does not enforce.
    /// Accepting or echoing one would make the negotiated security properties
    /// differ from the actual SRTP context.
    fn validate_attribute_features(attr: &SdesCryptoAttribute) -> Result<(), Error> {
        if attr.key_info.contains('|') {
            return Err(Error::UnsupportedFeature(
                "SDES lifetime and MKI key parameters are not implemented".to_string(),
            ));
        }

        if let Some(param) = attr
            .session_params
            .iter()
            .map(|param| param.trim())
            .find(|param| !param.is_empty())
        {
            return Err(Error::UnsupportedFeature(format!(
                "SDES session parameter {param:?} is not implemented"
            )));
        }

        Ok(())
    }

    /// Reject configurations that could produce an empty or dishonest offer.
    fn validate_config(&self) -> Result<(), Error> {
        if self.config.crypto_suites.is_empty() {
            return Err(Error::InvalidParameter(
                "SDES requires at least one implemented SRTP suite".to_string(),
            ));
        }
        if self.role == SdesRole::Offerer && self.config.offer_count == 0 {
            return Err(Error::InvalidParameter(
                "SDES offer_count must be greater than zero".to_string(),
            ));
        }
        for suite in &self.config.crypto_suites {
            Self::suite_name(suite)?;
        }
        Ok(())
    }

    /// Decode the fixed-size master key and salt used by implemented suites.
    fn decode_key(
        attr: &SdesCryptoAttribute,
        suite: &SrtpCryptoSuite,
    ) -> Result<SrtpCryptoKey, Error> {
        let keysalt = BASE64
            .decode(&attr.key_info)
            .map_err(|_| Error::ParseError("Invalid Base64 encoding in key info".into()))?;
        let expected_len = suite.key_length + 14;
        if keysalt.len() != expected_len {
            return Err(Error::ParseError(format!(
                "SDES key info must contain exactly {expected_len} bytes, got {}",
                keysalt.len()
            )));
        }
        Ok(SrtpCryptoKey::new(
            keysalt[..suite.key_length].to_vec(),
            keysalt[suite.key_length..].to_vec(),
        ))
    }

    /// Create a crypto attribute for a specific crypto suite
    fn create_crypto_attribute(
        &self,
        tag: u32,
        suite: &SrtpCryptoSuite,
    ) -> Result<(SdesCryptoAttribute, SrtpCryptoKey), Error> {
        let crypto_suite_str = Self::suite_name(suite)?;

        // Generate random key
        let mut key = vec![0u8; suite.key_length];
        OsRng.fill_bytes(&mut key);

        // Generate random salt
        let mut salt = vec![0u8; 14]; // 112-bit salt
        OsRng.fill_bytes(&mut salt);

        // Combine key and salt
        let mut keysalt = Vec::with_capacity(key.len() + salt.len());
        keysalt.extend_from_slice(&key);
        keysalt.extend_from_slice(&salt);

        // Base64 encode key+salt
        let key_info = BASE64.encode(&keysalt);

        // Create crypto attribute
        let attr = SdesCryptoAttribute::new(tag, crypto_suite_str, &key_info);

        // Create SRTP key for later use
        let srtp_key = SrtpCryptoKey::new(key, salt);

        Ok((attr, srtp_key))
    }

    /// Create offer crypto attributes
    fn create_offer(&mut self) -> Result<Vec<String>, Error> {
        if self.role != SdesRole::Offerer {
            return Err(Error::InvalidState("Only offerer can create offer".into()));
        }
        self.validate_config()?;

        let mut offer = Vec::new();

        // Create crypto attributes for each supported crypto suite
        for (i, suite) in self
            .config
            .crypto_suites
            .iter()
            .take(self.config.offer_count)
            .enumerate()
        {
            let tag = (i + 1) as u32;
            let (attr, srtp_key) = self.create_crypto_attribute(tag, suite)?;

            // Store local attribute
            self.local_attrs.push(attr.clone());

            // Add to offer
            offer.push(format!("a=crypto:{}", attr.to_string()));

            // Save key if it's the first one (the default)
            if i == 0 {
                self.local_srtp_key = Some(srtp_key);
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
            return Err(Error::InvalidState(
                "Only answerer can create answer".into(),
            ));
        }
        self.validate_config()?;

        // Parse offer
        let mut remote_attrs = Vec::new();
        for line in offer {
            if line.starts_with("a=crypto:") {
                let attr_str = line.trim_start_matches("a=crypto:");
                let attr = SdesCryptoAttribute::parse(attr_str)?;
                Self::validate_attribute_features(&attr)?;
                remote_attrs.push(attr);
            }
        }

        if remote_attrs.is_empty() {
            return Err(Error::InvalidMessage(
                "No crypto attributes in offer".into(),
            ));
        }

        // Select the first offered suite that is also locally configured.
        let (selected, srtp_suite) = remote_attrs
            .iter()
            .find_map(|attr| {
                let suite = Self::suite_from_name(&attr.crypto_suite)?;
                self.config
                    .crypto_suites
                    .contains(&suite)
                    .then_some((attr, suite))
            })
            .ok_or_else(|| {
                Error::NegotiationFailed(
                    "No mutually configured, implemented SDES crypto suite".to_string(),
                )
            })?;
        let remote_srtp_key = Self::decode_key(selected, &srtp_suite)?;
        let selected = selected.clone();

        // RFC 4568 uses independent key material in each direction. The
        // answer keeps the offered key only for inbound traffic and advertises
        // a freshly generated local transmit key with the selected tag/suite.
        let (local_attr, local_srtp_key) =
            self.create_crypto_attribute(selected.tag, &srtp_suite)?;

        // Commit negotiation state only after both directional keys and the
        // answer attribute have been validated and created successfully.
        self.remote_attrs = remote_attrs;
        self.local_attrs = vec![local_attr.clone()];
        self.local_srtp_key = Some(local_srtp_key);
        self.remote_srtp_key = Some(remote_srtp_key);
        self.srtp_suite = Some(srtp_suite);
        self.selected_attr = Some(local_attr.clone());

        // Create answer
        let mut answer = Vec::new();
        answer.push(format!("a=crypto:{}", local_attr.to_string()));

        // Update state
        self.state = SdesState::Completed;

        Ok(answer)
    }

    /// Process answer
    fn process_answer(&mut self, answer: &[String]) -> Result<(), Error> {
        if self.role != SdesRole::Offerer {
            return Err(Error::InvalidState(
                "Only offerer can process answer".into(),
            ));
        }
        self.validate_config()?;

        // Parse answer
        let mut selected_attr = None;
        let mut remote_attrs = Vec::new();

        for line in answer {
            if line.starts_with("a=crypto:") {
                let attr_str = line.trim_start_matches("a=crypto:");
                let attr = SdesCryptoAttribute::parse(attr_str)?;
                Self::validate_attribute_features(&attr)?;

                // This is the selected attribute
                selected_attr = Some(attr.clone());
                remote_attrs.push(attr);
                break;
            }
        }

        let selected = selected_attr
            .ok_or_else(|| Error::InvalidMessage("No crypto attributes in answer".into()))?;

        // Find matching local attribute by tag
        let local_attr = self
            .local_attrs
            .iter()
            .find(|attr| attr.tag == selected.tag)
            .ok_or_else(|| {
                Error::InvalidMessage(format!(
                    "No matching local attribute for tag {}",
                    selected.tag
                ))
            })?;
        if selected.crypto_suite != local_attr.crypto_suite {
            return Err(Error::NegotiationFailed(format!(
                "SDES answer changed suite for tag {}",
                selected.tag
            )));
        }
        let suite = Self::suite_from_name(&local_attr.crypto_suite).ok_or_else(|| {
            Error::UnsupportedFeature(format!(
                "Unsupported crypto suite: {}",
                local_attr.crypto_suite
            ))
        })?;
        if !self.config.crypto_suites.contains(&suite) {
            return Err(Error::NegotiationFailed(
                "SDES answer selected a suite outside the local configuration".to_string(),
            ));
        }
        let local_key = Self::decode_key(local_attr, &suite)?;
        let remote_key = Self::decode_key(&selected, &suite)?;

        // Commit only after the answer and both directional keys are valid.
        self.remote_attrs = remote_attrs;
        self.selected_attr = Some(selected);
        self.local_srtp_key = Some(local_key);
        self.remote_srtp_key = Some(remote_key);
        self.srtp_suite = Some(suite);

        // Update state
        self.state = SdesState::Completed;

        Ok(())
    }
}

impl Sdes {
    /// Return both directional keys after negotiation completes.
    ///
    /// An incomplete exchange returns an error instead of falling back to the
    /// local key for both directions.
    pub fn get_directional_keys(&self) -> Result<SdesKeyMaterial, Error> {
        let local_tx = self.local_srtp_key.clone().ok_or_else(|| {
            Error::InvalidState("SDES local transmit key is not available".to_string())
        })?;
        let remote_rx = self.remote_srtp_key.clone().ok_or_else(|| {
            Error::InvalidState("SDES remote receive key is not available".to_string())
        })?;
        Ok(SdesKeyMaterial {
            local_tx,
            remote_rx,
        })
    }

    /// Return the peer's transmit key used for inbound unprotection.
    pub fn get_remote_srtp_key(&self) -> Option<SrtpCryptoKey> {
        self.remote_srtp_key.clone()
    }
}

impl SecurityKeyExchange for Sdes {
    fn init(&mut self) -> Result<(), Error> {
        self.validate_config()
    }

    fn process_message(&mut self, message: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        self.validate_config()?;
        // SDES messages are SDP lines
        let message_str = std::str::from_utf8(message)
            .map_err(|_| Error::ParseError("Invalid UTF-8 in SDES message".into()))?;

        let lines: Vec<String> = message_str.lines().map(|s| s.trim().to_string()).collect();

        match (self.role, &self.state) {
            (SdesRole::Offerer, SdesState::Initial) => {
                // Create offer
                let offer = self.create_offer()?;
                let offer_str = offer.join("\r\n");
                Ok(Some(offer_str.into_bytes()))
            }
            (SdesRole::Offerer, SdesState::OfferSent) => {
                // Process answer
                self.process_answer(&lines)?;
                Ok(None)
            }
            (SdesRole::Answerer, SdesState::Initial) => {
                // Create answer
                let answer = self.create_answer(&lines)?;
                let answer_str = answer.join("\r\n");
                Ok(Some(answer_str.into_bytes()))
            }
            _ => Err(Error::InvalidState(
                "Invalid state for message processing".into(),
            )),
        }
    }

    fn get_srtp_key(&self) -> Option<SrtpCryptoKey> {
        // Compatibility accessor: SDES now has directional keys, so this
        // continues to expose the key used for local transmission.
        self.local_srtp_key.clone()
    }

    fn get_remote_srtp_key(&self) -> Option<SrtpCryptoKey> {
        self.remote_srtp_key.clone()
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
