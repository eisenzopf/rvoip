//! Shared types: PASSporT claims, attestation levels, originating /
//! destination identifiers.
//!
//! These mirror the RFC 8225 (PASSporT base) and RFC 8588 (SHAKEN
//! extension) claim sets.

use serde::{Deserialize, Serialize};

/// SHAKEN attestation level (ATIS-1000074 §5.2.3).
///
/// - **A — Full Attestation:** the originating service provider
///   authenticated the caller AND verified they are authorised to use
///   the calling number.
/// - **B — Partial Attestation:** the SP authenticated the caller but
///   cannot verify number-use authorisation.
/// - **C — Gateway Attestation:** the SP authenticated the call
///   origination but cannot vouch for the caller (e.g. inbound
///   international gateway).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Attestation {
    #[serde(rename = "A")]
    Full,
    #[serde(rename = "B")]
    Partial,
    #[serde(rename = "C")]
    Gateway,
}

impl Attestation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Attestation::Full => "A",
            Attestation::Partial => "B",
            Attestation::Gateway => "C",
        }
    }
}

/// Originating or destination identifier inside a PASSporT (RFC 8225
/// §5.2). Carries either a telephone number (`tn`) or a URI (`uri`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OrigDestField {
    /// `{"tn": "+15551234567"}` — E.164 telephone number.
    Tn { tn: String },
    /// `{"uri": "sip:alice@example.com"}` — SIP/SIPS URI.
    Uri { uri: String },
}

/// Destination claim shape — PASSporT `dest` is always an object whose
/// values are arrays (RFC 8225 §5.2.2). Most SHAKEN deployments carry
/// a single TN per call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrigDest {
    /// Telephone numbers (`{"tn": ["+15551234567"]}`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tn: Option<Vec<String>>,
    /// URIs (`{"uri": ["sip:alice@example.com"]}`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<Vec<String>>,
}

impl OrigDest {
    pub fn from_tn(tn: impl Into<String>) -> Self {
        Self {
            tn: Some(vec![tn.into()]),
            uri: None,
        }
    }

    pub fn from_uri(uri: impl Into<String>) -> Self {
        Self {
            tn: None,
            uri: Some(vec![uri.into()]),
        }
    }
}

/// PASSporT extension type (the `ppt` parameter on the SIP `Identity`
/// header). SHAKEN deployments use `shaken`; call-diversion uses
/// `div`; rich-call-data uses `rcd`. Absence implies the base profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PptType {
    Shaken,
    Div,
    Rcd,
}

impl PptType {
    pub fn as_str(&self) -> &'static str {
        match self {
            PptType::Shaken => "shaken",
            PptType::Div => "div",
            PptType::Rcd => "rcd",
        }
    }
}

/// PASSporT claim set (RFC 8225 §5 base claims + RFC 8588 SHAKEN
/// additions). Signers populate this from the outbound SIP request;
/// verifiers reconstruct it from the JWT payload to cross-check
/// against the SIP message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PassportClaims {
    /// `orig` — originating identity. Required.
    pub orig: OrigDestField,

    /// `dest` — destination identity. Required.
    pub dest: OrigDest,

    /// `iat` — JWT issued-at, seconds since UNIX epoch. RFC 8225 §5.2.4.
    pub iat: u64,

    /// `origid` — RFC 8588 §4 unique call identifier (UUID). SHAKEN
    /// uses this for traceback.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origid: Option<uuid::Uuid>,

    /// `attest` — SHAKEN attestation level (`A`/`B`/`C`).
    /// Required when `ppt=shaken`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attest: Option<Attestation>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attestation_serialises_as_letter() {
        let json = serde_json::to_string(&Attestation::Full).unwrap();
        assert_eq!(json, "\"A\"");
    }

    #[test]
    fn orig_dest_tn_round_trip() {
        let od = OrigDest::from_tn("+15551234567");
        let json = serde_json::to_string(&od).unwrap();
        assert_eq!(json, "{\"tn\":[\"+15551234567\"]}");
        let back: OrigDest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tn.as_deref(), Some(&["+15551234567".to_string()][..]));
    }

    #[test]
    fn passport_claims_omit_optional_fields() {
        let claims = PassportClaims {
            orig: OrigDestField::Tn {
                tn: "+15551234567".to_string(),
            },
            dest: OrigDest::from_tn("+15550009999"),
            iat: 1_700_000_000,
            origid: None,
            attest: None,
        };
        let json = serde_json::to_string(&claims).unwrap();
        assert!(!json.contains("origid"));
        assert!(!json.contains("attest"));
        assert!(json.contains("\"iat\":1700000000"));
    }
}
