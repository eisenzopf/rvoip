//! # SIP Identity Header (RFC 8224)
//!
//! The `Identity` header carries a signed PASSporT (RFC 8225) JWT used to
//! authenticate the originator of a SIP request. STIR/SHAKEN
//! (ATIS-1000074) builds on this primitive for telephone-number attestation
//! on the PSTN.
//!
//! Wire format (RFC 8224 §4.1):
//!
//! ```text
//! Identity         = "Identity" HCOLON signed-identity-digest
//!                       *(SEMI ident-info-params)
//! signed-identity-digest = 1*(base64-char / ".")
//! ident-info-params      = info-param / alg-param / ppt-param
//!                          / generic-param
//! info-param             = "info" EQUAL ident-info
//! ident-info             = LAQUOT absoluteURI RAQUOT
//! alg-param              = "alg" EQUAL token
//! ppt-param              = "ppt" EQUAL token
//! ```
//!
//! Example:
//!
//! ```text
//! Identity: eyJhbGciOiJFUzI1NiIsInR5cCI6InBhc3Nwb3J0IiwicHB0Ijoic2hha2VuIn0.
//!  eyJhdHRlc3QiOiJBIiwiZGVzdCI6eyJ0biI6WyIxMjAyNTU1MDAwMSJdfSwib3JpZyI6eyJ0biI6
//!  IjEyMDI1NTUwMDAyIn19.signature;
//!  info=<https://cert.example.org/passport.cer>;alg=ES256;ppt=shaken
//! ```
//!
//! This module exposes a typed wrapper with:
//!
//! - **Lossless preservation** of the original byte form (`raw`) so that
//!   STIR/SHAKEN verifiers can recompute the JWT signature against the
//!   upstream signer's exact wire bytes (RFC 8224 §7.4).
//! - Parsed access to the JWT compact-form string and the three
//!   well-known parameters (`info`, `alg`, `ppt`).
//!
//! Crypto operations (JWT/JWS signature verification, certificate chain
//! validation, `info=` URL fetch) are intentionally **out of scope** for
//! `rvoip-sip-core`. Those are provided by `rvoip-stir-shaken` via the
//! `PASSporTVerifier` trait defined in `rvoip-sip-dialog`.

use crate::error::{Error, Result};
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// `Identity` header (RFC 8224).
///
/// Holds the compact-form JWT (PASSporT token) plus the well-known
/// parameters `info`, `alg`, and `ppt`. The `raw` field preserves the
/// original wire value byte-for-byte so verifiers can recompute the JWS
/// signature against the upstream signer's canonical form (RFC 8224 §7.4
/// — signature is computed over the JWT's own header.payload, not the
/// SIP header text, but the raw form is preserved to avoid re-encoding
/// pitfalls).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    /// Compact-form JWT: `header.payload.signature` (base64url-encoded
    /// segments joined by `.`). RFC 8224 §6.2.1.
    pub jwt: String,

    /// `info=<URI>` parameter — URL of the certificate that contains the
    /// public key whose corresponding private key was used to sign the
    /// JWT. Required by RFC 8224 §4.1.
    pub info: Option<String>,

    /// `alg=` parameter — JWS signing algorithm name. SHAKEN mandates
    /// `ES256` (ATIS-1000074); the field stays a plain string so other
    /// PASSporT profiles can carry their own algorithm names.
    pub alg: Option<String>,

    /// `ppt=` parameter — PASSporT extension type. SHAKEN uses
    /// `shaken`; diversion uses `div`; rich-call-data uses `rcd`.
    /// Absence implies the base PASSporT profile (RFC 8225).
    pub ppt: Option<String>,

    /// Original wire value preserved byte-for-byte, including any
    /// generic parameters not parsed into the typed fields. Used by
    /// verifiers when re-serialising for signature checks.
    pub raw: String,
}

impl Identity {
    /// Construct from just a JWT string (no parameters).
    pub fn new(jwt: impl Into<String>) -> Self {
        let jwt = jwt.into();
        let raw = jwt.clone();
        Self {
            jwt,
            info: None,
            alg: None,
            ppt: None,
            raw,
        }
    }

    /// Construct from a JWT plus the three well-known parameters.
    pub fn with_params(
        jwt: impl Into<String>,
        info: Option<String>,
        alg: Option<String>,
        ppt: Option<String>,
    ) -> Self {
        let jwt = jwt.into();
        // Build the canonical wire form from the parts.
        let mut raw = jwt.clone();
        if let Some(info) = &info {
            raw.push_str(";info=<");
            raw.push_str(info);
            raw.push('>');
        }
        if let Some(alg) = &alg {
            raw.push_str(";alg=");
            raw.push_str(alg);
        }
        if let Some(ppt) = &ppt {
            raw.push_str(";ppt=");
            raw.push_str(ppt);
        }
        Self {
            jwt,
            info,
            alg,
            ppt,
            raw,
        }
    }
}

impl fmt::Display for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Always emit the byte-preserved raw form so we don't drift from
        // what the upstream signer produced.
        f.write_str(&self.raw)
    }
}

impl FromStr for Identity {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        if s.trim().is_empty() {
            return Err(Error::ParseError("Empty Identity header value".to_string()));
        }
        let raw_input = s.trim();

        // Split off the JWT (up to the first `;`) from the parameters.
        // Per RFC 8224 §4.1 the JWT is `1*(base64-char / ".")` followed
        // by zero or more `;`-prefixed parameters.
        let (jwt_part, params_part) = match raw_input.find(';') {
            Some(idx) => (&raw_input[..idx], &raw_input[idx + 1..]),
            None => (raw_input, ""),
        };

        let jwt = jwt_part.trim().to_string();
        if jwt.is_empty() {
            return Err(Error::ParseError("Identity header missing JWT".to_string()));
        }

        let mut info = None;
        let mut alg = None;
        let mut ppt = None;

        for param in params_part.split(';') {
            let param = param.trim();
            if param.is_empty() {
                continue;
            }
            let (name, value) = match param.find('=') {
                Some(eq) => (param[..eq].trim(), param[eq + 1..].trim()),
                None => (param, ""),
            };
            match name.to_lowercase().as_str() {
                "info" => {
                    // RFC 8224: info value is `LAQUOT absoluteURI RAQUOT`
                    // — strip surrounding `<>` if present.
                    let unwrapped = value
                        .strip_prefix('<')
                        .and_then(|s| s.strip_suffix('>'))
                        .unwrap_or(value);
                    info = Some(unwrapped.to_string());
                }
                "alg" => alg = Some(value.to_string()),
                "ppt" => ppt = Some(value.to_string()),
                // Generic params are tolerated but not parsed — they
                // survive in `raw`.
                _ => {}
            }
        }

        Ok(Self {
            jwt,
            info,
            alg,
            ppt,
            raw: raw_input.to_string(),
        })
    }
}

impl TypedHeaderTrait for Identity {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::Identity
    }

    fn to_header(&self) -> Header {
        Header::new(
            Self::header_name(),
            HeaderValue::Raw(self.to_string().into_bytes()),
        )
    }

    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() {
            return Err(Error::InvalidHeader(format!(
                "Expected {} header, got {}",
                Self::header_name(),
                header.name
            )));
        }
        match &header.value {
            HeaderValue::Raw(bytes) => match std::str::from_utf8(bytes) {
                Ok(s) => Identity::from_str(s.trim()),
                Err(_) => Err(Error::InvalidHeader(format!(
                    "Invalid UTF-8 in {} header",
                    Self::header_name()
                ))),
            },
            _ => Err(Error::InvalidHeader(format!(
                "Unexpected header value type for {}",
                Self::header_name()
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_JWT: &str = "eyJhbGciOiJFUzI1NiIsInR5cCI6InBhc3Nwb3J0IiwicHB0Ijoic2hha2VuIn0.\
         eyJhdHRlc3QiOiJBIn0.\
         dGVzdHNpZw";

    #[test]
    fn from_str_jwt_only() {
        let id = Identity::from_str(SAMPLE_JWT).expect("parse");
        assert_eq!(id.jwt, SAMPLE_JWT);
        assert!(id.info.is_none());
        assert!(id.alg.is_none());
        assert!(id.ppt.is_none());
    }

    #[test]
    fn from_str_with_all_params() {
        let input = format!(
            "{};info=<https://cert.example.org/p.cer>;alg=ES256;ppt=shaken",
            SAMPLE_JWT
        );
        let id = Identity::from_str(&input).expect("parse");
        assert_eq!(id.jwt, SAMPLE_JWT);
        assert_eq!(id.info.as_deref(), Some("https://cert.example.org/p.cer"));
        assert_eq!(id.alg.as_deref(), Some("ES256"));
        assert_eq!(id.ppt.as_deref(), Some("shaken"));
    }

    #[test]
    fn raw_is_byte_preserved() {
        // Generic params + odd spacing should survive byte-for-byte in `raw`.
        let input = format!("{};alg=ES256 ; ppt=shaken;x-extra=yes", SAMPLE_JWT);
        let id = Identity::from_str(&input).expect("parse");
        assert_eq!(id.raw, input);
        // Display falls back to raw.
        assert_eq!(id.to_string(), input);
    }

    #[test]
    fn empty_rejected() {
        assert!(Identity::from_str("").is_err());
        assert!(Identity::from_str("   ").is_err());
    }

    #[test]
    fn missing_jwt_rejected() {
        assert!(Identity::from_str(";alg=ES256").is_err());
    }

    #[test]
    fn typed_header_roundtrip() {
        let input = format!("{};alg=ES256;ppt=shaken", SAMPLE_JWT);
        let id = Identity::from_str(&input).expect("parse");
        let header = id.to_header();
        assert_eq!(header.name, HeaderName::Identity);
        let back = Identity::from_header(&header).expect("back");
        assert_eq!(back.jwt, id.jwt);
        assert_eq!(back.alg, id.alg);
        assert_eq!(back.ppt, id.ppt);
    }

    #[test]
    fn with_params_constructor_builds_raw() {
        let id = Identity::with_params(
            SAMPLE_JWT,
            Some("https://cert.example.org/p.cer".to_string()),
            Some("ES256".to_string()),
            Some("shaken".to_string()),
        );
        assert!(id.raw.contains(SAMPLE_JWT));
        assert!(id.raw.contains("info=<https://cert.example.org/p.cer>"));
        assert!(id.raw.contains("alg=ES256"));
        assert!(id.raw.contains("ppt=shaken"));
    }
}
