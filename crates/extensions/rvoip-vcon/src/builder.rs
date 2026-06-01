//! [`VconBuilder`] — fluent constructor that consumes rvoip-core
//! session metadata and produces a [`crate::Vcon`].
//!
//! Adapters call `VconBuilder::new()` at session start, accumulate
//! parties / dialog segments as the call progresses, then `.build()`
//! at `session.ended` and hand the result to a [`crate::VconStore`].

use chrono::{DateTime, Utc};
use jsonwebtoken::{decode, DecodingKey, Validation};

use crate::types::{Dialog, DialogKind, Party, Vcon, VconError};

/// Fluent builder for [`Vcon`] documents. Cheap to construct; clone-
/// safe at any point.
pub struct VconBuilder {
    vcon: Vcon,
}

impl VconBuilder {
    /// New builder with a fresh uuid + creation timestamp.
    pub fn new() -> Self {
        Self {
            vcon: Vcon::new_now(),
        }
    }

    /// Use an explicit uuid (for cases where the recording id is
    /// allocated separately and must match the vCon's id).
    pub fn with_uuid(mut self, uuid: uuid::Uuid) -> Self {
        self.vcon.uuid = uuid;
        self
    }

    /// Set the conversation subject.
    pub fn subject(mut self, subject: impl Into<String>) -> Self {
        self.vcon.subject = Some(subject.into());
        self
    }

    /// Append one party. Returns the index — useful for the dialog
    /// builder calls that follow.
    pub fn party(&mut self, party: Party) -> u32 {
        let idx = self.vcon.parties.len() as u32;
        self.vcon.parties.push(party);
        idx
    }

    /// Builder variant: party() returning `self`. Caller has to track
    /// indices externally if they want to reference them in dialogs.
    pub fn with_party(mut self, party: Party) -> Self {
        self.vcon.parties.push(party);
        self
    }

    /// Append a recording dialog segment.
    pub fn recording(
        mut self,
        start: DateTime<Utc>,
        duration_ms: u64,
        parties: Vec<u32>,
        mediatype: impl Into<String>,
    ) -> Self {
        self.vcon.dialog.push(Dialog {
            kind: DialogKind::Recording,
            start,
            duration_ms: Some(duration_ms),
            parties,
            mediatype: Some(mediatype.into()),
            body: None,
            url: None,
        });
        self
    }

    /// Append a text dialog segment (chat / IM / transcript turn).
    pub fn text(
        mut self,
        start: DateTime<Utc>,
        party: u32,
        body: impl Into<String>,
    ) -> Self {
        self.vcon.dialog.push(Dialog {
            kind: DialogKind::Text,
            start,
            duration_ms: Some(0),
            parties: vec![party],
            mediatype: Some("text/plain".into()),
            body: Some(body.into()),
            url: None,
        });
        self
    }

    /// Finalize. Returns the populated [`Vcon`] ready for persistence.
    pub fn build(self) -> Vcon {
        self.vcon
    }
}

impl Default for VconBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Verify a JWS-signed vCon and return the parsed [`Vcon`]. Use the
/// same `DecodingKey` shape as `rvoip_auth_core::JwtValidator` — HMAC
/// secret, RSA PEM, or EC PEM. Validates signature only; the caller
/// is responsible for any further semantic checks (e.g. issuer
/// allowlist).
pub fn verify_jws(
    compact: &str,
    decoding_key: &DecodingKey,
    algorithm: jsonwebtoken::Algorithm,
) -> Result<Vcon, VconError> {
    let mut validation = Validation::new(algorithm);
    // vCon JWS doesn't carry standard JWT claims (exp/iss/aud); skip
    // those checks. The container body IS the payload.
    validation.required_spec_claims.clear();
    validation.validate_exp = false;
    validation.validate_aud = false;
    let data = decode::<Vcon>(compact, decoding_key, &validation)
        .map_err(|e| VconError::Verify(e.to_string()))?;
    Ok(data.claims)
}
