//! JWS General JSON Serialization for signed vCons.

use std::collections::{BTreeMap, BTreeSet};

use base64::Engine as _;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;
use uuid::Uuid;

use crate::{encode_base64url, Vcon, VconError};

/// The JWS General JSON representation required for a signed vCon.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SignedVcon {
    pub payload: String,
    pub signatures: Vec<JwsSignature>,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// One signature over the common payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JwsSignature {
    /// Unprotected certificate-discovery metadata. This metadata is not
    /// a trust anchor and MUST NOT be used as one by a verifier.
    pub header: JwsHeader,
    /// Base64url-encoded [`JwsProtectedHeader`].
    pub protected: String,
    /// Base64url-encoded cryptographic signature.
    pub signature: String,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Unprotected JWS header. Exactly one of `x5c` and `x5u` is required.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct JwsHeader {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x5c: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x5u: Option<String>,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Integrity-protected vCon signing metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JwsProtectedHeader {
    pub alg: Algorithm,
    pub uuid: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typ: Option<String>,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Certificate-chain discovery metadata placed in the unprotected header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CertificateReference {
    X5c(Vec<String>),
    X5u(String),
}

impl CertificateReference {
    fn into_header(self) -> Result<JwsHeader, VconError> {
        match self {
            Self::X5c(chain) if !valid_x5c_chain(&chain) => Err(VconError::Sign(
                "x5c must contain base64-encoded DER certificate metadata".into(),
            )),
            Self::X5c(chain) => Ok(JwsHeader {
                x5c: Some(chain),
                ..JwsHeader::default()
            }),
            Self::X5u(url) if !is_https_url(&url) => {
                Err(VconError::Sign("x5u certificate URL must use HTTPS".into()))
            }
            Self::X5u(url) => Ok(JwsHeader {
                x5u: Some(url),
                ..JwsHeader::default()
            }),
        }
    }
}

/// A caller-selected trusted verification key.
///
/// The resolver used by [`verify_jws_with`] returns this value; embedded
/// `x5c`/`x5u` metadata is never treated as trusted key material.
#[derive(Clone)]
pub struct TrustedKey {
    pub key: DecodingKey,
    pub algorithm: Algorithm,
}

impl TrustedKey {
    pub fn new(key: DecodingKey, algorithm: Algorithm) -> Self {
        Self { key, algorithm }
    }
}

/// Sign an unsigned vCon using JWS General JSON Serialization.
pub fn sign_jws(
    vcon: &Vcon,
    encoding_key: &EncodingKey,
    algorithm: Algorithm,
    certificate: CertificateReference,
) -> Result<SignedVcon, VconError> {
    vcon.validate()?;
    reject_hmac(algorithm, true)?;

    let payload = encode_base64url(serde_json::to_vec(vcon)?);
    let signature = create_signature(&payload, vcon.uuid, encoding_key, algorithm, certificate)?;
    Ok(SignedVcon {
        payload,
        signatures: vec![signature],
        extra: BTreeMap::new(),
    })
}

/// Add another signature over an existing signed vCon payload.
///
/// The payload is decoded and validated before the new signature is added,
/// but is never reserialized.
pub fn append_signature(
    signed: &mut SignedVcon,
    encoding_key: &EncodingKey,
    algorithm: Algorithm,
    certificate: CertificateReference,
) -> Result<(), VconError> {
    reject_hmac(algorithm, true)?;
    validate_general_members(signed).map_err(VconError::Sign)?;
    let vcon = decode_payload(&signed.payload, VconError::Sign)?;
    vcon.validate()?;
    let signature = create_signature(
        &signed.payload,
        vcon.uuid,
        encoding_key,
        algorithm,
        certificate,
    )?;
    signed.signatures.push(signature);
    Ok(())
}

/// Verify every signature using one caller-trusted key.
///
/// For documents signed by different keys, use [`verify_jws_with`].
pub fn verify_jws(
    signed: &SignedVcon,
    decoding_key: &DecodingKey,
    algorithm: Algorithm,
) -> Result<Vcon, VconError> {
    let key = decoding_key.clone();
    verify_jws_with(signed, move |_, _| {
        Ok(TrustedKey::new(key.clone(), algorithm))
    })
}

/// Verify every signature using keys selected by a caller-provided resolver.
///
/// The resolver receives parsed protected and unprotected headers, but must
/// resolve them against an independently trusted key source.
pub fn verify_jws_with<F>(signed: &SignedVcon, mut resolver: F) -> Result<Vcon, VconError>
where
    F: FnMut(&JwsProtectedHeader, &JwsHeader) -> Result<TrustedKey, VconError>,
{
    validate_general_members(signed).map_err(VconError::Verify)?;
    if signed.signatures.is_empty() {
        return Err(VconError::Verify(
            "JWS General JSON must contain at least one signature".into(),
        ));
    }

    let mut protected_headers = Vec::with_capacity(signed.signatures.len());
    for (index, signature) in signed.signatures.iter().enumerate() {
        validate_unprotected_header(&signature.header)
            .map_err(|message| VconError::Verify(format!("signature[{index}]: {message}")))?;
        let protected = decode_protected(&signature.protected)
            .map_err(|message| VconError::Verify(format!("signature[{index}]: {message}")))?;
        validate_header_names(&protected, &signature.header)
            .map_err(|message| VconError::Verify(format!("signature[{index}]: {message}")))?;
        reject_hmac(protected.alg, false)?;
        let trusted = resolver(&protected, &signature.header)?;
        if trusted.algorithm != protected.alg {
            return Err(VconError::Verify(format!(
                "signature[{index}] protected algorithm {:?} does not match trusted algorithm {:?}",
                protected.alg, trusted.algorithm
            )));
        }
        let signing_input = format!("{}.{}", signature.protected, signed.payload);
        let verified = jsonwebtoken::crypto::verify(
            &signature.signature,
            signing_input.as_bytes(),
            &trusted.key,
            protected.alg,
        )
        .map_err(|error| VconError::Verify(error.to_string()))?;
        if !verified {
            return Err(VconError::Verify(format!(
                "signature[{index}] verification failed"
            )));
        }
        protected_headers.push(protected);
    }

    let vcon = decode_payload(&signed.payload, VconError::Verify)?;
    vcon.validate()
        .map_err(|error| VconError::Verify(error.to_string()))?;
    for (index, protected) in protected_headers.iter().enumerate() {
        if protected.uuid != vcon.uuid {
            return Err(VconError::Verify(format!(
                "signature[{index}] protected uuid {} does not match payload uuid {}",
                protected.uuid, vcon.uuid
            )));
        }
    }
    Ok(vcon)
}

fn validate_general_members(signed: &SignedVcon) -> Result<(), String> {
    validate_flattened_names("signed vCon", &signed.extra, &["payload", "signatures"])?;
    for (index, signature) in signed.signatures.iter().enumerate() {
        validate_flattened_names(
            &format!("signature[{index}]"),
            &signature.extra,
            &["header", "protected", "signature"],
        )?;
    }
    Ok(())
}

fn validate_flattened_names(
    path: &str,
    extra: &BTreeMap<String, Value>,
    defined: &[&str],
) -> Result<(), String> {
    for name in extra.keys() {
        if name.trim().is_empty() {
            return Err(format!("{path} contains an empty member name"));
        }
        if defined.contains(&name.as_str()) {
            return Err(format!("{path}.{name} collides with a defined JWS member"));
        }
    }
    Ok(())
}

fn create_signature(
    payload: &str,
    uuid: Uuid,
    encoding_key: &EncodingKey,
    algorithm: Algorithm,
    certificate: CertificateReference,
) -> Result<JwsSignature, VconError> {
    let header = certificate.into_header()?;
    let protected = JwsProtectedHeader {
        alg: algorithm,
        uuid,
        typ: None,
        extra: BTreeMap::new(),
    };
    let protected = encode_base64url(serde_json::to_vec(&protected)?);
    let signing_input = format!("{protected}.{payload}");
    let signature = jsonwebtoken::crypto::sign(signing_input.as_bytes(), encoding_key, algorithm)
        .map_err(|error| VconError::Sign(error.to_string()))?;
    Ok(JwsSignature {
        header,
        protected,
        signature,
        extra: BTreeMap::new(),
    })
}

fn decode_payload(payload: &str, error: fn(String) -> VconError) -> Result<Vcon, VconError> {
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|source| error(format!("payload is not valid base64url: {source}")))?;
    serde_json::from_slice(&decoded).map_err(|source| {
        error(format!(
            "payload is not an uncompressed unsigned vCon JSON document: {source}"
        ))
    })
}

fn decode_protected(encoded: &str) -> Result<JwsProtectedHeader, String> {
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|source| format!("protected header is not valid base64url: {source}"))?;
    serde_json::from_slice(&decoded)
        .map_err(|source| format!("protected header is not valid JSON: {source}"))
}

fn validate_unprotected_header(header: &JwsHeader) -> Result<(), String> {
    match (&header.x5c, &header.x5u) {
        (Some(chain), None) if valid_x5c_chain(chain) => Ok(()),
        (None, Some(url)) if is_https_url(url) => Ok(()),
        (Some(_), Some(_)) => Err("x5c and x5u are mutually exclusive".into()),
        (Some(_), None) => Err("x5c must contain base64-encoded DER metadata".into()),
        (None, Some(_)) => Err("x5u must use HTTPS".into()),
        (None, None) => Err("either x5c or x5u is required".into()),
    }
}

fn validate_header_names(
    protected: &JwsProtectedHeader,
    unprotected: &JwsHeader,
) -> Result<(), String> {
    if protected.extra.contains_key("crit") {
        return Err("critical JOSE header parameters are not supported".into());
    }
    if protected.extra.contains_key("b64") {
        return Err(
            "the JWS b64 header is not supported; signed vCons require a base64url payload".into(),
        );
    }
    if unprotected.extra.contains_key("crit") || unprotected.extra.contains_key("b64") {
        return Err("crit and b64 must not appear in the unprotected header".into());
    }
    if protected
        .extra
        .keys()
        .any(|name| matches!(name.as_str(), "alg" | "uuid" | "typ" | "x5c" | "x5u"))
    {
        return Err("protected header repeats or misplaces a reserved parameter".into());
    }
    if unprotected
        .extra
        .keys()
        .any(|name| matches!(name.as_str(), "x5c" | "x5u"))
    {
        return Err("unprotected header repeats a reserved parameter".into());
    }

    let mut protected_names = BTreeSet::from(["alg", "uuid"]);
    if protected.typ.is_some() {
        protected_names.insert("typ");
    }
    protected_names.extend(protected.extra.keys().map(String::as_str));

    let mut unprotected_names = BTreeSet::new();
    if unprotected.x5c.is_some() {
        unprotected_names.insert("x5c");
    }
    if unprotected.x5u.is_some() {
        unprotected_names.insert("x5u");
    }
    unprotected_names.extend(unprotected.extra.keys().map(String::as_str));

    if protected_names.is_disjoint(&unprotected_names) {
        Ok(())
    } else {
        Err("protected and unprotected JOSE header names must be disjoint".into())
    }
}

fn valid_x5c_chain(chain: &[String]) -> bool {
    !chain.is_empty()
        && chain.iter().all(|certificate| {
            let Ok(der) = base64::engine::general_purpose::STANDARD.decode(certificate) else {
                return false;
            };
            let Ok((remaining, _certificate)) = x509_parser::parse_x509_certificate(&der) else {
                return false;
            };
            !der.is_empty() && remaining.is_empty()
        })
}

fn is_https_url(value: &str) -> bool {
    Url::parse(value)
        .ok()
        .is_some_and(|url| url.scheme() == "https" && url.host_str().is_some())
}

fn reject_hmac(algorithm: Algorithm, signing: bool) -> Result<(), VconError> {
    if matches!(
        algorithm,
        Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512
    ) {
        let message = "HMAC algorithms are not permitted for signed vCons".into();
        if signing {
            Err(VconError::Sign(message))
        } else {
            Err(VconError::Verify(message))
        }
    } else {
        Ok(())
    }
}
