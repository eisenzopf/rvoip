//! Parser for the `Identity` header (RFC 8224 §4.1).
//!
//! The wire form is:
//!
//! ```text
//! Identity         = "Identity" HCOLON signed-identity-digest
//!                       *(SEMI ident-info-params)
//! ```
//!
//! Parsing splits the JWT (compact-form, up to the first `;`) from the
//! parameter list (`info=<URI>`, `alg=token`, `ppt=token`, plus generic
//! params). Validation of the JWT signature, certificate chain, and
//! PASSporT claims is out of scope here — that is `rvoip-stir-shaken`'s
//! responsibility.
//!
//! The full parse logic lives on `Identity::from_str` in
//! `crate::types::identity` — the function here is the nom-shaped entry
//! point used by the header-dispatch tables.

use crate::error::Error;
use crate::parser::ParseResult;
use crate::types::identity::Identity;
use std::str::FromStr;

/// Parse the value of an `Identity` header into the typed wrapper.
///
/// Per RFC 8224 the value is essentially `token / "." / "<" / ">" / "="`
/// with `;`-separated parameters. We delegate the actual parameter
/// splitting to `Identity::from_str` (which keeps the byte-preserved
/// `raw` form intact), and only consume the whole input here so the
/// nom-driven dispatcher in `parser/headers/mod.rs` can use it
/// uniformly with the other typed-header parsers.
pub fn parse_identity(input: &[u8]) -> ParseResult<'_, Identity> {
    let s = match std::str::from_utf8(input) {
        Ok(s) => s,
        Err(_) => {
            return Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Verify,
            )));
        }
    };
    match Identity::from_str(s) {
        Ok(id) => Ok((&input[input.len()..], id)),
        Err(Error::ParseError(_)) | Err(_) => Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Verify,
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_JWT: &str = "eyJhbGciOiJFUzI1NiIsInR5cCI6InBhc3Nwb3J0IiwicHB0Ijoic2hha2VuIn0.\
         eyJhdHRlc3QiOiJBIn0.\
         dGVzdHNpZw";

    #[test]
    fn parses_jwt_with_params() {
        let input = format!(
            "{};info=<https://cert.example.org/p.cer>;alg=ES256;ppt=shaken",
            SAMPLE_JWT
        );
        let (rem, id) = parse_identity(input.as_bytes()).unwrap();
        assert!(rem.is_empty());
        assert_eq!(id.jwt, SAMPLE_JWT);
        assert_eq!(id.alg.as_deref(), Some("ES256"));
        assert_eq!(id.ppt.as_deref(), Some("shaken"));
        assert_eq!(id.info.as_deref(), Some("https://cert.example.org/p.cer"));
    }

    #[test]
    fn parses_bare_jwt() {
        let (_, id) = parse_identity(SAMPLE_JWT.as_bytes()).unwrap();
        assert_eq!(id.jwt, SAMPLE_JWT);
        assert!(id.info.is_none());
    }

    #[test]
    fn rejects_empty() {
        assert!(parse_identity(b"").is_err());
    }
}
