//! IMS AKA provider adapters for `rvoip-sip`.
//!
//! IMS AKA is provider-backed: SIP carries AKA as Digest-family authentication
//! headers, while RAND/AUTN/XRES/SQN material comes from SIM/USIM, HSS/AuC,
//! UDM/AUSF, or a lab vector source. This crate implements the `rvoip-sip`
//! provider traits without claiming carrier IMS certification.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use rvoip_sip::{
    AkaClientProvider, AkaVectorProvider, AuthIdentity, Result as SipResult, SessionError,
    SipAuthChallenge, SipAuthScheme, SipAuthSource,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// AKA algorithm name carried in SIP Digest-family headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImsAkaAlgorithm {
    AKAv1Md5,
    AKAv2Md5,
}

impl ImsAkaAlgorithm {
    pub fn as_header_value(self) -> &'static str {
        match self {
            Self::AKAv1Md5 => "AKAv1-MD5",
            Self::AKAv2Md5 => "AKAv2-MD5",
        }
    }
}

impl std::fmt::Display for ImsAkaAlgorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_header_value())
    }
}

/// Non-secret AKA vector fields needed by the SIP auth layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImsAkaVector {
    pub username: String,
    pub realm: String,
    pub nonce: String,
    pub algorithm: ImsAkaAlgorithm,
    /// Expected Digest/AKA response for deterministic tests or broker-backed
    /// validation. Production providers should avoid logging this value.
    pub expected_response: String,
    pub subject: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
}

impl ImsAkaVector {
    pub fn challenge_value(&self) -> String {
        format!(
            "Digest realm=\"{}\", nonce=\"{}\", algorithm={}, qop=\"auth\"",
            escape_header(&self.realm),
            escape_header(&self.nonce),
            self.algorithm
        )
    }
}

/// IMS AKA adapter error.
#[derive(Debug, Error)]
pub enum ImsAkaError {
    #[error("AKA configuration error: {0}")]
    Config(String),
    #[error("AKA validation failed")]
    Invalid,
    #[error("AKA provider unavailable: {0}")]
    Unavailable(String),
}

impl From<ImsAkaError> for SessionError {
    fn from(err: ImsAkaError) -> Self {
        SessionError::AuthError(err.to_string())
    }
}

/// Deterministic vector provider for tests and lab fixtures.
#[derive(Debug, Clone)]
pub struct StaticAkaProvider {
    vector: Arc<ImsAkaVector>,
}

impl StaticAkaProvider {
    pub fn new(vector: ImsAkaVector) -> Self {
        Self {
            vector: Arc::new(vector),
        }
    }

    pub fn vector(&self) -> &ImsAkaVector {
        &self.vector
    }
}

impl AkaClientProvider for StaticAkaProvider {
    fn authorization(
        &self,
        challenge_header: &str,
        _method: &str,
        request_uri: &str,
        nonce_count: u32,
    ) -> SipResult<String> {
        let params = parse_auth_params(challenge_header);
        let nonce = params.get("nonce").unwrap_or(&self.vector.nonce);
        let realm = params.get("realm").unwrap_or(&self.vector.realm);
        let algorithm = params
            .get("algorithm")
            .map(String::as_str)
            .unwrap_or(self.vector.algorithm.as_header_value());
        if nonce != &self.vector.nonce
            || realm != &self.vector.realm
            || !algorithm.eq_ignore_ascii_case(self.vector.algorithm.as_header_value())
        {
            return Err(SessionError::AuthError(
                "AKA challenge does not match configured vector".to_string(),
            ));
        }
        Ok(format!(
            "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\", response=\"{}\", algorithm={}, qop=auth, nc={:08x}, cnonce=\"static-aka\"",
            escape_header(&self.vector.username),
            escape_header(&self.vector.realm),
            escape_header(&self.vector.nonce),
            escape_header(request_uri),
            escape_header(&self.vector.expected_response),
            self.vector.algorithm,
            nonce_count,
        ))
    }
}

#[async_trait]
impl AkaVectorProvider for StaticAkaProvider {
    async fn validate(
        &self,
        authorization: &str,
        _method: &str,
        _request_uri: &str,
        _body: Option<&[u8]>,
    ) -> SipResult<Option<AuthIdentity>> {
        let params = parse_auth_params(authorization);
        let valid = params
            .get("username")
            .is_some_and(|value| value == &self.vector.username)
            && params
                .get("realm")
                .is_some_and(|value| value == &self.vector.realm)
            && params
                .get("nonce")
                .is_some_and(|value| value == &self.vector.nonce)
            && params
                .get("response")
                .is_some_and(|value| value == &self.vector.expected_response)
            && params.get("algorithm").is_some_and(|value| {
                value.eq_ignore_ascii_case(self.vector.algorithm.as_header_value())
            });
        if !valid {
            return Ok(None);
        }
        Ok(Some(AuthIdentity {
            scheme: SipAuthScheme::Aka,
            username: Some(self.vector.username.clone()),
            subject: self.vector.subject.clone(),
            realm: Some(self.vector.realm.clone()),
            scopes: self.vector.scopes.clone(),
            source: SipAuthSource::Origin,
        }))
    }

    fn challenge(&self, source: SipAuthSource) -> SipAuthChallenge {
        SipAuthChallenge {
            scheme: SipAuthScheme::Aka,
            value: self.vector.challenge_value(),
            source,
        }
    }
}

/// Provider that validates AKA Authorization headers through an external
/// HSS/UDM/AUSF broker while issuing a locally configured challenge.
#[cfg(feature = "http")]
#[derive(Debug, Clone)]
pub struct HttpAkaVectorProvider {
    vector: Arc<ImsAkaVector>,
    validation_endpoint: String,
    client: reqwest::Client,
}

#[cfg(feature = "http")]
impl HttpAkaVectorProvider {
    pub fn new(vector: ImsAkaVector, validation_endpoint: impl Into<String>) -> Self {
        Self {
            vector: Arc::new(vector),
            validation_endpoint: validation_endpoint.into(),
            client: reqwest::Client::new(),
        }
    }
}

#[cfg(feature = "http")]
#[async_trait]
impl AkaVectorProvider for HttpAkaVectorProvider {
    async fn validate(
        &self,
        authorization: &str,
        method: &str,
        request_uri: &str,
        body: Option<&[u8]>,
    ) -> SipResult<Option<AuthIdentity>> {
        let request = HttpAkaValidationRequest {
            authorization: authorization.to_string(),
            method: method.to_string(),
            request_uri: request_uri.to_string(),
            body_sha256: body.map(sha256_hex),
        };
        let response = self
            .client
            .post(&self.validation_endpoint)
            .json(&request)
            .send()
            .await
            .map_err(|err| ImsAkaError::Unavailable(err.to_string()))?;
        if !response.status().is_success() {
            return Ok(None);
        }
        let response = response
            .json::<HttpAkaValidationResponse>()
            .await
            .map_err(|err| ImsAkaError::Unavailable(err.to_string()))?;
        if !response.valid {
            return Ok(None);
        }
        Ok(Some(AuthIdentity {
            scheme: SipAuthScheme::Aka,
            username: response
                .username
                .or_else(|| Some(self.vector.username.clone())),
            subject: response.subject,
            realm: Some(self.vector.realm.clone()),
            scopes: response.scopes,
            source: SipAuthSource::Origin,
        }))
    }

    fn challenge(&self, source: SipAuthSource) -> SipAuthChallenge {
        SipAuthChallenge {
            scheme: SipAuthScheme::Aka,
            value: self.vector.challenge_value(),
            source,
        }
    }
}

#[cfg(feature = "http")]
#[derive(Debug, Serialize)]
struct HttpAkaValidationRequest {
    authorization: String,
    method: String,
    request_uri: String,
    body_sha256: Option<String>,
}

#[cfg(feature = "http")]
#[derive(Debug, Deserialize)]
struct HttpAkaValidationResponse {
    valid: bool,
    username: Option<String>,
    subject: Option<String>,
    #[serde(default)]
    scopes: Vec<String>,
}

/// Lab-only Milenage adapter placeholder for deployments that want to plug
/// software vector generation behind the same provider traits.
#[cfg(feature = "lab-milenage")]
#[derive(Debug, Clone)]
pub struct MilenageLabProvider {
    inner: StaticAkaProvider,
}

#[cfg(feature = "lab-milenage")]
impl MilenageLabProvider {
    /// Build a lab provider from an already computed test vector.
    ///
    /// The crate intentionally does not certify operator K/OPc provisioning or
    /// carrier HSS behavior. Use this for deterministic lab vectors and wire
    /// real production vectors through [`HttpAkaVectorProvider`] or a custom
    /// [`AkaVectorProvider`] implementation.
    pub fn from_test_vector(vector: ImsAkaVector) -> Self {
        Self {
            inner: StaticAkaProvider::new(vector),
        }
    }
}

#[cfg(feature = "lab-milenage")]
impl AkaClientProvider for MilenageLabProvider {
    fn authorization(
        &self,
        challenge_header: &str,
        method: &str,
        request_uri: &str,
        nonce_count: u32,
    ) -> SipResult<String> {
        self.inner
            .authorization(challenge_header, method, request_uri, nonce_count)
    }
}

#[cfg(feature = "lab-milenage")]
#[async_trait]
impl AkaVectorProvider for MilenageLabProvider {
    async fn validate(
        &self,
        authorization: &str,
        method: &str,
        request_uri: &str,
        body: Option<&[u8]>,
    ) -> SipResult<Option<AuthIdentity>> {
        self.inner
            .validate(authorization, method, request_uri, body)
            .await
    }

    fn challenge(&self, source: SipAuthSource) -> SipAuthChallenge {
        self.inner.challenge(source)
    }
}

fn parse_auth_params(value: &str) -> BTreeMap<String, String> {
    let mut header = value.trim();
    if let Some(rest) = header.strip_prefix("Digest ") {
        header = rest;
    }
    let mut params = BTreeMap::new();
    for part in header.split(',') {
        let Some((key, value)) = part.trim().split_once('=') else {
            continue;
        };
        params.insert(
            key.trim().to_ascii_lowercase(),
            value.trim().trim_matches('"').to_string(),
        );
    }
    params
}

fn escape_header(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(feature = "http")]
fn sha256_hex(body: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(body);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vector() -> ImsAkaVector {
        ImsAkaVector {
            username: "sip-user".to_string(),
            realm: "ims.example.test".to_string(),
            nonce: "RAND-AUTN".to_string(),
            algorithm: ImsAkaAlgorithm::AKAv1Md5,
            expected_response: "expected-res".to_string(),
            subject: Some("imsi-001010123456789".to_string()),
            scopes: vec!["sip.register".to_string()],
        }
    }

    #[test]
    fn static_provider_builds_digest_family_aka_challenge() {
        let provider = StaticAkaProvider::new(vector());
        let challenge = provider.challenge(SipAuthSource::Origin);
        assert_eq!(challenge.scheme, SipAuthScheme::Aka);
        assert!(challenge.value.starts_with("Digest "));
        assert!(challenge.value.contains("algorithm=AKAv1-MD5"));
        assert!(challenge.value.contains("qop=\"auth\""));
    }

    #[tokio::test]
    async fn static_provider_authorization_round_trip_validates() {
        let provider = StaticAkaProvider::new(vector());
        let challenge = provider.challenge(SipAuthSource::Origin);
        let authorization = provider
            .authorization(&challenge.value, "REGISTER", "sip:ims.example.test", 1)
            .unwrap();
        let identity = provider
            .validate(&authorization, "REGISTER", "sip:ims.example.test", None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(identity.scheme, SipAuthScheme::Aka);
        assert_eq!(identity.username.as_deref(), Some("sip-user"));
        assert_eq!(identity.subject.as_deref(), Some("imsi-001010123456789"));
        assert_eq!(identity.scopes, vec!["sip.register"]);
    }

    #[tokio::test]
    async fn static_provider_rejects_wrong_response() {
        let provider = StaticAkaProvider::new(vector());
        let rejected = provider
            .validate(
                "Digest username=\"sip-user\", realm=\"ims.example.test\", nonce=\"RAND-AUTN\", uri=\"sip:ims.example.test\", response=\"wrong\", algorithm=AKAv1-MD5",
                "REGISTER",
                "sip:ims.example.test",
                None,
            )
            .await
            .unwrap();
        assert!(rejected.is_none());
    }
}
